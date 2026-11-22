#!/usr/bin/env python3
"""CFP (CNET Filing Protocol, doc/spec_NET_FP.md) server for the host.

The server is one node on a CNET UDP segment (doc/spec_NET.md §8.3) that
answers L3P transactions (doc/spec_NET_L3P.md §5) on socket 9 and exports
host directories or FS2 images:

    python tools/cfp-server/cfp_server.py --bind 127.0.0.1:32041 \\
        --peer 127.0.0.1:32032 --net 1 --export share=C:/cpt32/share \\
        --export disk=build/foundation.fs2,ro

Foundation mounts an export with ``netfs mount .FF02 share s``. The module
also has ``CfpClient``, a small Python client used by the tests.
"""
import argparse
import os
from pathlib import Path
import shutil
import stat as statmod
import struct
import sys
import threading
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'l3p'))
sys.path.insert(0, str(HERE.parent / 'fs2'))
import l3p  # noqa: E402
import fs2  # noqa: E402

CFP_SOCKET = 9
CFP_NODE = 0xFF02
VERSION = 1
IOUNIT = 960
MAX_BODY = 980
MAX_NAME = 900
MAX_WALK = 16
MAX_RENAME_NAMES = 968
HDR = struct.Struct('>BBH')

T_HELLO, T_EXPORTS = 0x01, 0x02
T_ATTACH, T_WALK, T_CLUNK, T_OPEN, T_CREATE, T_READ, T_WRITE, T_READDIR = range(0x10, 0x18)
T_STAT, T_SETATTR, T_REMOVE, T_RENAME, T_SYNC, T_STATFS = range(0x18, 0x1E)
R_ERROR = 0xFF
NAMES = {T_HELLO: 'HELLO', T_EXPORTS: 'EXPORTS', T_ATTACH: 'ATTACH', T_WALK: 'WALK', T_CLUNK: 'CLUNK',
         T_OPEN: 'OPEN', T_CREATE: 'CREATE', T_READ: 'READ', T_WRITE: 'WRITE', T_READDIR: 'READDIR',
         T_STAT: 'STAT', T_SETATTR: 'SETATTR', T_REMOVE: 'REMOVE', T_RENAME: 'RENAME', T_SYNC: 'SYNC',
         T_STATFS: 'STATFS'}

O_READ, O_WRITE, O_CREATE, O_EXCL, O_TRUNC, O_APPEND, O_EXEC = 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40
MODE_DEFAULT = 0xFFFFFFFF
FILE, DIR = 1, 2
NOFID = 0xFFFFFFFF
VOL_FS2, VOL_VIRTUAL = 1, 4

# Foundation error numbers (spec_Foundation §4, spec_Foundation_API §3)
E = {'BAD_ARG': -2, 'BAD_HANDLE': -3, 'ACCESS': -4, 'NO_MEMORY': -5, 'LIMIT': -6, 'TIMEOUT': -8,
     'NOT_FOUND': -10, 'EXISTS': -11, 'FORMAT': -12, 'UNSUPPORTED': -13, 'IO': -16, 'NO_MEDIA': -17,
     'READ_ONLY': -18, 'BUSY': -19, 'NOT_DIR': -64, 'IS_DIR': -65, 'NOT_EMPTY': -66, 'BAD_PATH': -67,
     'NAME_TOO_LONG': -68, 'NO_SPACE': -69, 'CROSS_DEVICE': -70, 'STALE': -73}
ERROR_NAMES = {v: k for k, v in E.items()}


class CfpError(Exception):
    def __init__(self, code, message=''):
        super().__init__(f'{code}: {message}' if message else code)
        self.code = code
        self.message = message


def _oserror(error):
    import errno
    table = {errno.ENOENT: 'NOT_FOUND', errno.EEXIST: 'EXISTS', errno.ENOTDIR: 'NOT_DIR',
             errno.EISDIR: 'IS_DIR', errno.ENOTEMPTY: 'NOT_EMPTY', errno.EACCES: 'ACCESS',
             errno.EPERM: 'ACCESS', errno.ENOSPC: 'NO_SPACE', errno.EROFS: 'READ_ONLY',
             errno.ENAMETOOLONG: 'NAME_TOO_LONG', errno.EXDEV: 'CROSS_DEVICE', errno.EBUSY: 'BUSY'}
    return CfpError(table.get(error.errno, 'IO'), error.strerror or str(error))


def check_name(name):
    """A CFP name (spec §3.4, FS2 §6); '.', '..' and '' are rejected here."""
    if name in ('', '.', '..'):
        raise CfpError('BAD_PATH', f'invalid name {name!r}')
    if len(name.encode('utf-8')) > MAX_NAME:
        raise CfpError('NAME_TOO_LONG', 'name longer than 900 bytes')
    try:
        fs2.check_name(name)
    except fs2.Fs2Error as error:
        raise CfpError(error.code, error.message if hasattr(error, 'message') else str(error))


def visible_name(name):
    try:
        check_name(name)
        return True
    except CfpError:
        return False


def pack_stat(kind, mode, uid, flags, size, created, modified, changed):
    def t(stamp):
        sec, ms = stamp
        return (sec >> 32) & 0xFFFFFFFF, sec & 0xFFFFFFFF, ms
    return struct.pack('>IIIIII3I3I3II', kind, mode, uid, flags, size >> 32, size & 0xFFFFFFFF,
                       *t(created), *t(modified), *t(changed), 0)


def _stamp_ns(ns):
    return ns // 1_000_000_000, ns // 1_000_000 % 1000


# ---- backends ----

class HostDir:
    """A host directory (spec §8.2–§8.4). Nodes are tuples of stored names."""
    fs_type = VOL_VIRTUAL

    def __init__(self, path, read_only=False):
        self.root_path = Path(path).resolve()
        if not self.root_path.is_dir():
            raise ValueError(f'{path}: not a directory')
        self.read_only = read_only

    def root(self):
        return ()

    def real(self, node):
        return self.root_path.joinpath(*node)

    def _inside(self, path):
        try:
            real = Path(os.path.realpath(path))
        except OSError:
            return False
        return real == self.root_path or self.root_path in real.parents

    def _lstat_ok(self, path):
        """os.stat of a visible entry or None (special files, links out)."""
        try:
            st = os.stat(path)
        except OSError:
            return None
        if not (statmod.S_ISREG(st.st_mode) or statmod.S_ISDIR(st.st_mode)):
            return None
        if os.path.islink(path) and not self._inside(path):
            return None
        return st

    def _stat(self, node):
        try:
            return os.stat(self.real(node))
        except OSError as error:
            raise _oserror(error)

    def kind(self, node):
        return DIR if statmod.S_ISDIR(self._stat(node).st_mode) else FILE

    def key(self, node):
        st = self._stat(node)
        return ('host', st.st_dev, st.st_ino) if st.st_ino else ('hostpath', str(self.real(node)).lower())

    def qid(self, node):
        st = self._stat(node)
        birth = getattr(st, 'st_birthtime_ns', 0) or 0
        path = ((st.st_dev & 0xFFFF) << 48 | (st.st_ino & 0xFFFFFFFFFFFF)) if st.st_ino else (
            hash(str(self.real(node)).lower()) & 0xFFFFFFFFFFFFFFFF)
        return (DIR if statmod.S_ISDIR(st.st_mode) else FILE), (birth // 1000) & 0xFFFFFFFF, path

    def stat(self, node):
        st = self._stat(node)
        kind = DIR if statmod.S_ISDIR(st.st_mode) else FILE
        m = st.st_mode
        mode = ((0x20 if m & 0o400 else 0) | (0x10 if m & 0o200 else 0) | (0x08 if m & 0o100 else 0) |
                (0x04 if m & 0o004 else 0) | (0x02 if m & 0o002 else 0) | (0x01 if m & 0o001 else 0))
        if kind == DIR and os.name == 'nt':
            mode |= 0x08 | 0x01
        flags = (1 if self.read_only else 0) | (2 if kind == FILE else 0)
        birth = getattr(st, 'st_birthtime_ns', None) or st.st_mtime_ns
        return pack_stat(kind, mode, 0, flags, st.st_size if kind == FILE else 0,
                         _stamp_ns(birth), _stamp_ns(st.st_mtime_ns), _stamp_ns(st.st_ctime_ns))

    def _entries(self, node):
        """Visible (name, stored name) pairs with fold collisions hidden."""
        try:
            names = sorted(os.listdir(self.real(node)))
        except OSError as error:
            raise _oserror(error)
        return names

    def listdir(self, node):
        seen = set()
        result = []
        names = self._entries(node)
        # names that are exactly what they claim win over fold duplicates
        for name in names:
            if not visible_name(name) or self._lstat_ok(self.real(node + (name,))) is None:
                continue
            key = fs2.fold(name)
            if key in seen:
                continue
            seen.add(key)
            result.append((name, node + (name,)))
        return result

    def lookup(self, node, name):
        if self.kind(node) != DIR:
            raise CfpError('NOT_DIR')
        names = self._entries(node)
        if name in names and self._lstat_ok(self.real(node + (name,))) is not None:
            return node + (name,)
        key = fs2.fold(name)
        matches = [n for n in names if fs2.fold(n) == key and visible_name(n)
                   and self._lstat_ok(self.real(node + (n,))) is not None]
        if len(matches) == 1:
            return node + (matches[0],)
        raise CfpError('NOT_FOUND', name)

    def parent(self, node):
        return node[:-1]

    def exists(self, node, name):
        key = fs2.fold(name)
        return any(fs2.fold(n) == key for n in self._entries(node))

    def read(self, node, offset, count):
        try:
            with open(self.real(node), 'rb') as f:
                f.seek(offset)
                return f.read(count)
        except OSError as error:
            raise _oserror(error)

    def write(self, node, offset, data, append):
        try:
            with open(self.real(node), 'r+b') as f:
                if append:
                    f.seek(0, os.SEEK_END)
                else:
                    end = f.seek(0, os.SEEK_END)
                    if offset > end:
                        f.write(bytes(offset - end))
                    f.seek(offset)
                f.write(data)
                return f.tell()
        except OSError as error:
            raise _oserror(error)

    def create(self, node, name, kind, mode):
        if self.exists(node, name):
            raise CfpError('EXISTS', name)
        path = self.real(node + (name,))
        try:
            if kind == DIR:
                os.mkdir(path)
            else:
                with open(path, 'xb'):
                    pass
        except OSError as error:
            raise _oserror(error)
        if mode != MODE_DEFAULT:
            self._chmod(path, mode)
        return node + (name,)

    def _chmod(self, path, mode):
        m = ((0o400 if mode & 0x20 else 0) | (0o200 if mode & 0x10 else 0) | (0o100 if mode & 0x08 else 0) |
             (0o004 if mode & 0x04 else 0) | (0o002 if mode & 0x02 else 0) | (0o001 if mode & 0x01 else 0))
        try:
            os.chmod(path, m | (0o040 if m & 0o004 else 0))
        except OSError as error:
            raise _oserror(error)

    def remove(self, node, is_dir):
        path = self.real(node)
        kind = self.kind(node)
        if is_dir and kind != DIR:
            raise CfpError('NOT_DIR')
        if not is_dir and kind == DIR:
            raise CfpError('IS_DIR')
        try:
            if kind == DIR:
                if os.listdir(path):
                    raise CfpError('NOT_EMPTY')
                os.rmdir(path)
            else:
                os.remove(path)
        except OSError as error:
            raise _oserror(error)

    def rename(self, old_dir, old_name, new_dir, new_name):
        source = self.lookup(old_dir, old_name)
        key = fs2.fold(new_name)
        clash = [n for n in self._entries(new_dir) if fs2.fold(n) == key]
        if clash and not (new_dir == old_dir and clash == [source[-1]]):
            raise CfpError('EXISTS', new_name)
        if self.kind(source) == DIR and new_dir[:len(source)] == source:
            raise CfpError('BAD_PATH', 'cannot move a directory into itself')
        try:
            os.rename(self.real(source), self.real(new_dir + (new_name,)))
        except OSError as error:
            raise _oserror(error)

    def setattr(self, node, mode=None, uid=None, size=None, mtime=None):
        path = self.real(node)
        if uid is not None and uid != 0:
            raise CfpError('UNSUPPORTED', 'host directories have UID 0 only')
        if mode is not None:
            self._chmod(path, mode)
        try:
            if size is not None:
                os.truncate(path, size)
            if mtime is not None:
                ns = mtime[0] * 1_000_000_000 + mtime[1] * 1_000_000
                os.utime(path, ns=(ns, ns))
        except OSError as error:
            raise _oserror(error)

    def sync(self, node):
        pass

    def statfs(self):
        try:
            usage = shutil.disk_usage(self.root_path)
            return usage.total, usage.free
        except OSError:
            return 0, 0

    def flush(self):
        pass


class Fs2Image:
    """An FS2 image file (spec §8.1). Nodes are inode numbers. Changes are
    made in memory, checked with fs2.check_image and written back: at once
    for structural changes, at CLUNK/SYNC/after a second for writes."""
    fs_type = VOL_FS2

    def __init__(self, path, read_only=False):
        self.path = Path(path)
        self.read_only = read_only
        self.good = bytes(self.path.read_bytes())
        self.vol = fs2.Volume(bytearray(self.good))
        self.dirty_since = None
        self.lock = threading.RLock()

    def _guard(self, fn, *args, flush=True):
        with self.lock:
            try:
                result = fn(*args)
            except fs2.Fs2Error as error:
                self.vol = fs2.Volume(bytearray(self.good), check=False)
                self.dirty_since = None
                raise CfpError(error.code, str(error))
            except CfpError:
                raise
            if flush:
                self.flush()
            elif self.dirty_since is None:
                self.dirty_since = time.monotonic()
            return result

    def flush(self):
        with self.lock:
            img = bytes(self.vol.img)
            if img == self.good:
                self.dirty_since = None
                return
            problems = fs2.check_image(bytearray(img))
            if problems:
                self.vol = fs2.Volume(bytearray(self.good), check=False)
                self.dirty_since = None
                raise CfpError('IO', f'refused an invalid image: {problems[0][1]}')
            temporary = self.path.with_name(self.path.name + '.tmp')
            temporary.write_bytes(img)
            os.replace(temporary, self.path)
            self.good = img
            self.dirty_since = None

    def tick(self):
        if self.dirty_since is not None and time.monotonic() - self.dirty_since > 1.0:
            try:
                self.flush()
            except CfpError as error:
                l3p.log('fs2 flush failed:', error)

    def inode(self, node):
        inode = self.vol.read_inode(node)
        if inode['type'] == fs2.FREE:
            raise CfpError('STALE', 'file no longer exists')
        return inode

    def root(self):
        return fs2.ROOT_INODE

    def kind(self, node):
        return DIR if self.inode(node)['type'] == fs2.DIR else FILE

    def key(self, node):
        return ('fs2', node)

    def qid(self, node):
        i = self.inode(node)
        return self.kind(node), (i['created_sec'] * 1000 + i['created_ms']) & 0xFFFFFFFF, node

    def stat(self, node):
        i = self.inode(node)
        kind = DIR if i['type'] == fs2.DIR else FILE
        flags = (1 if self.read_only else 0) | (2 if kind == FILE else 0)
        return pack_stat(kind, i['mode'], i['uid'], flags, i['size'] if kind == FILE else 0,
                         (i['created_sec'], i['created_ms']), (i['modified_sec'], i['modified_ms']),
                         (i['changed_sec'], i['changed_ms']))

    def listdir(self, node):
        return [(name, number) for _, number, name in self.vol.entries(self.inode(node))]

    def lookup(self, node, name):
        inode = self.inode(node)
        if inode['type'] != fs2.DIR:
            raise CfpError('NOT_DIR')
        found = self.vol.find(inode, name)
        if not found:
            raise CfpError('NOT_FOUND', name)
        return found[1]

    def parent(self, node):
        return self.inode(node)['parent']

    def path_of(self, node):
        parts = []
        while node != fs2.ROOT_INODE:
            parent = self.inode(node)['parent']
            for _, number, name in self.vol.entries(self.inode(parent)):
                if number == node:
                    parts.append(name)
                    break
            else:
                raise CfpError('STALE')
            node = parent
        return '/' + '/'.join(reversed(parts))

    def _join(self, node, name):
        base = self.path_of(node)
        return (base.rstrip('/') + '/' + name)

    def read(self, node, offset, count):
        with self.lock:
            return self.vol.read(self.inode(node))[offset:offset + count]

    def write(self, node, offset, data, append):
        def op():
            inode = self.inode(node)
            start = inode['size'] if append else offset
            self.vol.write(inode, start, data)
            self.vol.touch(inode, fs2.now_time())
            self.vol.write_inode(node, inode)
            return start + len(data)
        return self._guard(op, flush=False)

    def create(self, node, name, kind, mode):
        return self._guard(lambda: self.vol.create(self._join(node, name), fs2.DIR if kind == DIR else fs2.FILE,
                                                   None if mode == MODE_DEFAULT else mode)['number'])

    def remove(self, node, is_dir):
        def op():
            kind = self.kind(node)
            if is_dir and kind != DIR:
                raise CfpError('NOT_DIR')
            if not is_dir and kind == DIR:
                raise CfpError('IS_DIR')
            self.vol.remove(self.path_of(node))
        return self._guard(op)

    def rename(self, old_dir, old_name, new_dir, new_name):
        self._guard(lambda: self.vol.rename(self._join(old_dir, old_name), self._join(new_dir, new_name)))

    def setattr(self, node, mode=None, uid=None, size=None, mtime=None):
        def op():
            if mode is not None or uid is not None:
                self.vol.set_attributes(self.path_of(node), mode, uid)
            if size is not None:
                inode = self.inode(node)
                released = self.vol.truncate(inode, size) or []
                self.vol.touch(inode, fs2.now_time())
                self.vol.write_inode(node, inode)
                self.vol.release(released)
            if mtime is not None:
                inode = self.inode(node)
                inode['modified_sec'], inode['modified_ms'] = mtime
                self.vol.write_inode(node, inode)
        self._guard(op)

    def sync(self, node):
        self.flush()

    def statfs(self):
        sb = self.vol.sb
        return (sb['total_blocks'] - sb['data_start']) * fs2.BLOCK, len(self.vol.free_blocks()) * fs2.BLOCK


# ---- the protocol ----

class Fid:
    __slots__ = ('export', 'node', 'state', 'flags', 'snapshot')

    def __init__(self, export, node, state='walked', flags=0):
        self.export = export
        self.node = node
        self.state = state            # walked, file, dir
        self.flags = flags
        self.snapshot = None


class Export:
    def __init__(self, name, backend):
        self.name = name
        self.backend = backend

    @property
    def read_only(self):
        return self.backend.read_only


class Reader:
    def __init__(self, body):
        self.body = body
        self.pos = 0

    def take(self, fmt):
        size = struct.calcsize(fmt)
        if self.pos + size > len(self.body):
            raise CfpError('BAD_ARG', 'message too short')
        values = struct.unpack_from(fmt, self.body, self.pos)
        self.pos += size
        return values

    def bytes(self, n):
        if self.pos + n > len(self.body):
            raise CfpError('BAD_ARG', 'message too short')
        data = self.body[self.pos:self.pos + n]
        self.pos += n
        return data

    def name(self, n=None):
        if n is None:
            n, = self.take('>H')
        raw = self.bytes(n)
        try:
            return raw.decode('utf-8')
        except UnicodeDecodeError:
            raise CfpError('BAD_PATH', 'name is not UTF-8')

    def end(self):
        if self.pos != len(self.body):
            raise CfpError('BAD_ARG', 'trailing bytes')


def pack_qid(q):
    kind, gen, path = q
    return struct.pack('>B3xIQ', kind, gen, path)


def error_body(code, message=''):
    msg = message.encode('utf-8')[:128]
    return HDR.pack(R_ERROR, 0, 0) + struct.pack('>iHH', E.get(code, E['IO']), len(msg), 0) + msg


class Session:
    def __init__(self):
        self.hello = None
        self.fids = {}


class CfpServer:
    def __init__(self, exports, max_fids=64, verbose=False):
        self.exports = {e.name.lower(): e for e in exports}
        self.max_fids = max_fids
        self.verbose = verbose
        self.epoch = int(time.time()) & 0xFFFFFFFF
        self.sessions = {}
        self.lock = threading.RLock()

    # -- L3P server callbacks --

    def on_open(self, assoc):
        assoc.user = Session()
        with self.lock:
            self.sessions[id(assoc)] = assoc.user
        if self.verbose:
            l3p.log(assoc.name, 'session open')

    def on_close(self, assoc, error):
        session = assoc.user
        if not isinstance(session, Session):
            return
        with self.lock:
            for fid in list(session.fids):
                self._clunk(session, fid)
            self.sessions.pop(id(assoc), None)
        if self.verbose:
            l3p.log(assoc.name, 'session closed', error or '')

    def on_ureq(self, src, payload):
        if len(payload) < 4:
            return None
        ptype, flags, rsv = HDR.unpack_from(payload)
        if ptype != T_EXPORTS:
            return error_body('BAD_ARG', 'only EXPORTS is answered without an association')
        if flags or rsv or len(payload) != 4:
            return error_body('BAD_ARG')
        return self._exports()

    def on_request(self, assoc, payload):
        session = assoc.user if isinstance(assoc.user, Session) else Session()
        assoc.user = session
        try:
            if len(payload) < 4:
                raise CfpError('BAD_ARG', 'short message')
            ptype, flags, rsv = HDR.unpack_from(payload)
            if rsv or flags:
                raise CfpError('BAD_ARG', 'reserved field')
            body = payload[4:]
            if len(body) > MAX_BODY:
                raise CfpError('BAD_ARG', 'body too large')
            if ptype == T_EXPORTS:
                if body:
                    raise CfpError('BAD_ARG')
                return self._exports()
            if ptype == T_HELLO:
                answer = self._hello(session, Reader(body))
            else:
                handler = self.HANDLERS.get(ptype)
                if handler is None:
                    raise CfpError('UNSUPPORTED', f'type 0x{ptype:02X}')
                if session.hello is None:
                    raise CfpError('BAD_ARG', 'HELLO first')
                with self.lock:
                    answer = handler(self, session, Reader(body))
            out = HDR.pack(ptype | 0x80, 0, 0) + answer
            if self.verbose:
                l3p.log(assoc.name, NAMES.get(ptype, hex(ptype)), 'ok')
            return out
        except CfpError as error:
            if self.verbose:
                l3p.log(assoc.name, NAMES.get(payload[0] if payload else 0, '?'), error)
            return error_body(error.code, error.message)

    def tick(self):
        for export in self.exports.values():
            if isinstance(export.backend, Fs2Image):
                with self.lock:
                    export.backend.tick()

    # -- helpers --

    def _exports(self):
        out = bytearray(struct.pack('>HH', 0, 0))
        count = 0
        more = False
        for export in self.exports.values():
            name = export.name.encode('ascii')
            entry = struct.pack('>HH', 1 if export.read_only else 0, len(name)) + name
            entry += bytes(-len(entry) % 4)
            if 4 + len(out) + len(entry) > l3p.MAX_PLAIN:
                more = True
                break
            out += entry
            count += 1
        struct.pack_into('>HH', out, 0, count, 0)
        return HDR.pack(T_EXPORTS | 0x80, 1 if more else 0, 0) + bytes(out)

    def _hello(self, session, r):
        version, name_len, rsv = r.take('>IHH')
        if rsv or name_len > 64:
            raise CfpError('BAD_ARG')
        r.bytes(name_len)
        r.end()
        if version != VERSION:
            raise CfpError('UNSUPPORTED', f'CFP version {version}')
        session.hello = True
        return struct.pack('>IIHHI', VERSION, 0, self.max_fids, IOUNIT, self.epoch)

    def _fid(self, session, number, *states):
        fid = session.fids.get(number)
        if fid is None:
            raise CfpError('BAD_HANDLE', f'fid {number}')
        if states and fid.state not in states:
            raise CfpError('BAD_ARG', f'fid {number} is {fid.state}')
        return fid

    def _new_fid(self, session, number, replacing=None):
        if number == NOFID:
            raise CfpError('BAD_ARG', 'NOFID')
        if number in session.fids and number != replacing:
            raise CfpError('EXISTS', f'fid {number} in use')
        if number not in session.fids and len(session.fids) >= self.max_fids:
            raise CfpError('LIMIT', 'too many fids')

    def _is_open(self, export, node):
        key = export.backend.key(node)
        for session_assoc in self._all_sessions():
            for fid in session_assoc.fids.values():
                if fid.state != 'walked' and fid.export is export:
                    try:
                        if fid.export.backend.key(fid.node) == key:
                            return True
                    except CfpError:
                        continue
        return False

    def _all_sessions(self):
        return list(self.sessions.values())

    def _writable(self, export):
        if export.read_only:
            raise CfpError('READ_ONLY')

    def _clunk(self, session, number):
        fid = session.fids.pop(number, None)
        if fid and fid.state == 'file' and fid.flags & (O_WRITE | O_APPEND):
            try:
                fid.export.backend.flush()
            except CfpError:
                pass

    # -- operations (spec §6) --

    def op_attach(self, session, r):
        number, uid, length, rsv = r.take('>IIHH')
        if rsv:
            raise CfpError('BAD_ARG')
        name = r.name(length)
        r.end()
        self._new_fid(session, number)
        export = self.exports.get(name.lower())
        if export is None:
            raise CfpError('NOT_FOUND', f'no export {name}')
        if uid != 0:
            raise CfpError('ACCESS', 'this server accepts UID 0 only')
        node = export.backend.root()
        session.fids[number] = Fid(export, node)
        return pack_qid(export.backend.qid(node)) + struct.pack('>I', 1 if export.read_only else 0)

    def op_walk(self, session, r):
        number, newfid, count, rsv = r.take('>IIHH')
        if rsv or count > MAX_WALK:
            raise CfpError('BAD_ARG')
        names = [r.name() for _ in range(count)]
        r.end()
        fid = self._fid(session, number, 'walked')
        self._new_fid(session, newfid, replacing=number)
        backend = fid.export.backend
        node = fid.node
        qids = []
        status = 0
        for i, name in enumerate(names):
            try:
                if name == '..':
                    if node != backend.root():
                        node = backend.parent(node)
                else:
                    check_name(name)
                    if backend.kind(node) != DIR:
                        raise CfpError('NOT_DIR')
                    node = backend.lookup(node, name)
                qids.append(pack_qid(backend.qid(node)))
            except CfpError as error:
                if i == 0:
                    raise
                status = E.get(error.code, E['IO'])
                break
        if status == 0:
            session.fids[newfid] = Fid(fid.export, node)
        return struct.pack('>HHi', len(qids), 0, status) + b''.join(qids)

    def op_clunk(self, session, r):
        number, = r.take('>I')
        r.end()
        self._fid(session, number)
        self._clunk(session, number)
        return b''

    def _check_open_flags(self, flags, export):
        if flags & ~(O_READ | O_WRITE | O_TRUNC | O_APPEND | O_EXEC) or flags & (O_CREATE | O_EXCL):
            raise CfpError('BAD_ARG', 'bad flags')
        if not flags & (O_READ | O_WRITE | O_EXEC):
            raise CfpError('BAD_ARG', 'READ or WRITE needed')
        if flags & (O_TRUNC | O_APPEND) and not flags & O_WRITE:
            raise CfpError('BAD_ARG', 'TRUNC and APPEND need WRITE')
        if flags & (O_WRITE | O_TRUNC | O_APPEND):
            self._writable(export)

    def op_open(self, session, r):
        number, flags = r.take('>II')
        r.end()
        fid = self._fid(session, number, 'walked')
        backend = fid.export.backend
        if backend.kind(fid.node) == DIR:
            if flags != O_READ:
                raise CfpError('IS_DIR')
            fid.state = 'dir'
            fid.flags = flags
        else:
            self._check_open_flags(flags, fid.export)
            if flags & O_TRUNC:
                backend.setattr(fid.node, size=0)
            fid.state = 'file'
            fid.flags = flags
        return pack_qid(backend.qid(fid.node)) + struct.pack('>HH', IOUNIT, 0)

    def op_create(self, session, r):
        number, kind, rsv, length, flags, mode = r.take('>IBBHII')
        if rsv or kind not in (FILE, DIR):
            raise CfpError('BAD_ARG')
        name = r.name(length)
        r.end()
        fid = self._fid(session, number, 'walked')
        check_name(name)
        if kind == FILE:
            self._check_open_flags(flags, fid.export)
        elif flags:
            raise CfpError('BAD_ARG', 'DIR takes no flags')
        if mode != MODE_DEFAULT and mode & ~fs2.MODE_MASK:
            raise CfpError('BAD_ARG', 'mode uses bits 0-5')
        self._writable(fid.export)
        backend = fid.export.backend
        if backend.kind(fid.node) != DIR:
            raise CfpError('NOT_DIR')
        node = backend.create(fid.node, name, kind, mode)
        fid.node = node
        fid.state = 'file' if kind == FILE else 'walked'
        fid.flags = flags if kind == FILE else 0
        return pack_qid(backend.qid(node)) + struct.pack('>HH', IOUNIT, 0)

    def op_read(self, session, r):
        number, offset, count, rsv = r.take('>IQHH')
        r.end()
        fid = self._fid(session, number, 'file')
        if rsv or not 1 <= count <= IOUNIT:
            raise CfpError('BAD_ARG')
        if not fid.flags & (O_READ | O_EXEC):
            raise CfpError('ACCESS', 'not open for reading')
        data = fid.export.backend.read(fid.node, offset, count)
        return struct.pack('>HH', len(data), 0) + data

    def op_write(self, session, r):
        number, offset, count, rsv = r.take('>IQHH')
        fid = self._fid(session, number, 'file')
        if rsv or not 1 <= count <= IOUNIT:
            raise CfpError('BAD_ARG')
        data = r.bytes(count)
        r.end()
        if not fid.flags & O_WRITE:
            raise CfpError('ACCESS', 'not open for writing')
        self._writable(fid.export)
        end = fid.export.backend.write(fid.node, offset, data, bool(fid.flags & O_APPEND))
        return struct.pack('>HHQ', count, 0, end)

    def op_readdir(self, session, r):
        number, cookie, limit, rsv = r.take('>IIHH')
        r.end()
        fid = self._fid(session, number, 'dir')
        if rsv or not 76 <= limit <= MAX_BODY:
            raise CfpError('BAD_ARG')
        backend = fid.export.backend
        if cookie == 0 or fid.snapshot is None:
            if cookie != 0:
                raise CfpError('BAD_ARG', 'unknown cookie')
            fid.snapshot = [(n, node) for n, node in backend.listdir(fid.node)
                            if len(n.encode('utf-8')) <= MAX_NAME and visible_name(n)]
        if cookie > len(fid.snapshot):
            raise CfpError('BAD_ARG', 'unknown cookie')
        out = bytearray()
        index = cookie
        count = 0
        while index < len(fid.snapshot):
            name, node = fid.snapshot[index]
            try:
                st = backend.stat(node)
            except CfpError:
                index += 1  # removed meanwhile
                continue
            raw = name.encode('utf-8')
            entry = st + struct.pack('>HH', len(raw), len(name)) + raw
            entry += bytes(-len(entry) % 4)
            if 8 + len(out) + len(entry) > limit:
                if count == 0:
                    raise CfpError('LIMIT', 'MAX too small for the next entry')
                break
            out += entry
            count += 1
            index += 1
        return struct.pack('>IHH', index, count, 0) + bytes(out)

    def op_stat(self, session, r):
        number, = r.take('>I')
        r.end()
        fid = self._fid(session, number)
        backend = fid.export.backend
        return pack_qid(backend.qid(fid.node)) + backend.stat(fid.node)

    def op_setattr(self, session, r):
        number, mask, mode, uid, size, sec, ms, rsv = r.take('>IIIIQqHH')
        r.end()
        fid = self._fid(session, number, 'walked', 'file')
        if rsv or mask == 0 or mask & ~0xF or ms > 999:
            raise CfpError('BAD_ARG')
        self._writable(fid.export)
        kw = {}
        if mask & 1:
            if mode & ~fs2.MODE_MASK:
                raise CfpError('BAD_ARG', 'mode uses bits 0-5')
            kw['mode'] = mode
        if mask & 2:
            if uid >= 0xFFFFFFFF:
                raise CfpError('BAD_ARG')
            kw['uid'] = uid
        if mask & 4:
            if fid.state != 'file' or not fid.flags & O_WRITE:
                raise CfpError('ACCESS', 'SIZE needs a fid open for writing')
            kw['size'] = size
        if mask & 8:
            kw['mtime'] = (sec, ms)
        fid.export.backend.setattr(fid.node, **kw)
        return b''

    def op_remove(self, session, r):
        number, flags = r.take('>II')
        r.end()
        fid = self._fid(session, number)
        try:
            if flags & ~1:
                raise CfpError('BAD_ARG')
            if fid.state != 'walked':
                raise CfpError('BAD_ARG', 'fid is open')
            backend = fid.export.backend
            if fid.node == backend.root():
                raise CfpError('ACCESS', 'the export root cannot be removed')
            self._writable(fid.export)
            if self._is_open(fid.export, fid.node):
                raise CfpError('BUSY')
            backend.remove(fid.node, bool(flags & 1))
        finally:
            session.fids.pop(number, None)
        return b''

    def op_rename(self, session, r):
        old_number, new_number, old_len, new_len = r.take('>IIHH')
        old_name = r.name(old_len)
        new_name = r.name(new_len)
        r.end()
        old = self._fid(session, old_number, 'walked')
        new = self._fid(session, new_number, 'walked')
        check_name(old_name)
        check_name(new_name)
        if old.export is not new.export:
            raise CfpError('CROSS_DEVICE')
        self._writable(old.export)
        backend = old.export.backend
        if backend.kind(old.node) != DIR or backend.kind(new.node) != DIR:
            raise CfpError('NOT_DIR')
        source = backend.lookup(old.node, old_name)
        if self._is_open(old.export, source):
            raise CfpError('BUSY')
        backend.rename(old.node, old_name, new.node, new_name)
        return b''

    def op_sync(self, session, r):
        number, = r.take('>I')
        r.end()
        fid = self._fid(session, number)
        fid.export.backend.sync(fid.node)
        return b''

    def op_statfs(self, session, r):
        number, = r.take('>I')
        r.end()
        fid = self._fid(session, number)
        backend = fid.export.backend
        total, free = backend.statfs()
        return struct.pack('>IIIIIIII', backend.fs_type, 1 if fid.export.read_only else 0,
                           total >> 32, total & 0xFFFFFFFF, free >> 32, free & 0xFFFFFFFF, 0xFFFFFFFF, 0)

    HANDLERS = {T_ATTACH: op_attach, T_WALK: op_walk, T_CLUNK: op_clunk, T_OPEN: op_open,
                T_CREATE: op_create, T_READ: op_read, T_WRITE: op_write, T_READDIR: op_readdir,
                T_STAT: op_stat, T_SETATTR: op_setattr, T_REMOVE: op_remove, T_RENAME: op_rename,
                T_SYNC: op_sync, T_STATFS: op_statfs}


# ---- a small client (tests and host tools) ----

class CfpRemoteError(Exception):
    def __init__(self, status, message=''):
        self.status = status
        self.code = ERROR_NAMES.get(status, str(status))
        super().__init__(f'{self.code}: {message}' if message else self.code)


class CfpClient:
    def __init__(self, endpoint, net, node, sock=CFP_SOCKET, *, encrypt=False, verify=None, name='python'):
        self.assoc = endpoint.open_txn(net, node, sock, encrypt=encrypt, verify=verify)
        rsp = self.call(T_HELLO, struct.pack('>IHH', VERSION, len(name), 0) + name.encode())
        self.version, self.caps, self.max_fids, self.iounit, self.epoch = struct.unpack('>IIHHI', rsp)

    @staticmethod
    def parse(ptype, rsp):
        rtype, flags, _ = HDR.unpack_from(rsp)
        if rtype == R_ERROR:
            status, length, _ = struct.unpack_from('>iHH', rsp, 4)
            raise CfpRemoteError(status, rsp[12:12 + length].decode('utf-8', 'replace'))
        if rtype != ptype | 0x80:
            raise ValueError(f'unexpected reply type 0x{rtype:02X}')
        return rsp[4:]

    def call(self, ptype, body):
        return self.parse(ptype, self.assoc.call(HDR.pack(ptype, 0, 0) + body))

    def raw(self, payload):
        return self.assoc.call(payload)

    def close(self):
        self.assoc.close()

    @staticmethod
    def name(text):
        raw = text.encode('utf-8') if isinstance(text, str) else text
        return struct.pack('>H', len(raw)) + raw

    def attach(self, fid, export, uid=0):
        raw = export.encode()
        rsp = self.call(T_ATTACH, struct.pack('>IIHH', fid, uid, len(raw), 0) + raw)
        return rsp[:16], struct.unpack('>I', rsp[16:20])[0]

    def walk(self, fid, newfid, names):
        rsp = self.call(T_WALK, struct.pack('>IIHH', fid, newfid, len(names), 0) +
                        b''.join(self.name(n) for n in names))
        count, _, status = struct.unpack_from('>HHi', rsp)
        return count, status, [rsp[8 + 16 * i:24 + 16 * i] for i in range(count)]

    def clunk(self, fid):
        self.call(T_CLUNK, struct.pack('>I', fid))

    def open(self, fid, flags):
        return self.call(T_OPEN, struct.pack('>II', fid, flags))

    def create(self, fid, name, kind=FILE, flags=O_READ | O_WRITE, mode=MODE_DEFAULT):
        raw = name.encode('utf-8')
        return self.call(T_CREATE, struct.pack('>IBBHII', fid, kind, 0, len(raw), flags, mode) + raw)

    def read(self, fid, offset, count):
        rsp = self.call(T_READ, struct.pack('>IQHH', fid, offset, count, 0))
        n, = struct.unpack_from('>H', rsp)
        return rsp[4:4 + n]

    def write(self, fid, offset, data):
        rsp = self.call(T_WRITE, struct.pack('>IQHH', fid, offset, len(data), 0) + data)
        count, _, end = struct.unpack('>HHQ', rsp)
        return count, end

    def readdir(self, fid, cookie=0, limit=MAX_BODY):
        rsp = self.call(T_READDIR, struct.pack('>IIHH', fid, cookie, limit, 0))
        nxt, count, _ = struct.unpack_from('>IHH', rsp)
        pos = 8
        entries = []
        for _ in range(count):
            st = rsp[pos:pos + 64]
            nbytes, nchars = struct.unpack_from('>HH', rsp, pos + 64)
            name = rsp[pos + 68:pos + 68 + nbytes].decode('utf-8')
            entries.append((name, st))
            pos += (68 + nbytes + 3) & ~3
        return nxt, entries

    def listdir(self, fid):
        names = []
        cookie = 0
        while True:
            cookie, entries = self.readdir(fid, cookie)
            if not entries:
                return names
            names += [n for n, _ in entries]

    def stat(self, fid):
        rsp = self.call(T_STAT, struct.pack('>I', fid))
        return rsp[:16], rsp[16:80]

    def setattr(self, fid, mode=None, uid=None, size=None, mtime=None):
        mask = (1 if mode is not None else 0) | (2 if uid is not None else 0) | \
               (4 if size is not None else 0) | (8 if mtime is not None else 0)
        sec, ms = mtime or (0, 0)
        self.call(T_SETATTR, struct.pack('>IIIIQqHH', fid, mask, mode or 0, uid or 0, size or 0, sec, ms, 0))

    def remove(self, fid, is_dir=False):
        self.call(T_REMOVE, struct.pack('>II', fid, 1 if is_dir else 0))

    def rename(self, olddir, oldname, newdir, newname):
        a, b = oldname.encode('utf-8'), newname.encode('utf-8')
        self.call(T_RENAME, struct.pack('>IIHH', olddir, newdir, len(a), len(b)) + a + b)

    def sync(self, fid):
        self.call(T_SYNC, struct.pack('>I', fid))

    def statfs(self, fid):
        return struct.unpack('>IIIIIIII', self.call(T_STATFS, struct.pack('>I', fid)))


def list_exports(endpoint, net, node, sock=CFP_SOCKET, wait=0.5):
    """[(server (net, node, socket), [(name, flags)], more)] via UREQ."""
    result = []
    for src, rsp in endpoint.ucall(net, node, sock, HDR.pack(T_EXPORTS, 0, 0), wait=wait):
        try:
            rtype, flags, _ = HDR.unpack_from(rsp)
            if rtype != T_EXPORTS | 0x80:
                continue
            count, _ = struct.unpack_from('>HH', rsp, 4)
            pos = 8
            entries = []
            for _ in range(count):
                eflags, length = struct.unpack_from('>HH', rsp, pos)
                entries.append((rsp[pos + 4:pos + 4 + length].decode('ascii'), eflags))
                pos += (4 + length + 3) & ~3
            result.append((src, entries, bool(flags & 1)))
        except struct.error:
            continue
    return result


# ---- command line ----

def parse_export(text):
    name, sep, rest = text.partition('=')
    if not sep:
        raise ValueError(f'{text}: expected NAME=PATH[,ro]')
    path, _, opt = rest.rpartition(',') if rest.lower().endswith(',ro') else (rest, '', '')
    name = name.lower()
    if not 1 <= len(name) <= 31 or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789_-' for c in name):
        raise ValueError(f'{name}: export names are 1-31 of a-z 0-9 _ -')
    read_only = opt.lower() == 'ro'
    backend = HostDir(path, read_only) if Path(path).is_dir() else Fs2Image(path, read_only)
    return Export(name, backend)


def make_allow(allow, allow_any, net):
    if allow_any:
        return None
    if allow:
        addrs = {l3p.parse_addr(a) for a in allow}
        return lambda peer: any(peer[1] == node and (peer[0] == n or 0 in (peer[0], n)) for n, node in addrs)
    return lambda peer: peer[0] in (0, net)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bind', default='0.0.0.0:32032', help='UDP address of the segment (HOST:PORT)')
    parser.add_argument('--multicast', help='multicast group of the segment (GROUP:PORT)')
    parser.add_argument('--peer', action='append', default=[], help='unicast peer (HOST:PORT), repeatable')
    parser.add_argument('--segment-id', type=lambda t: int(t, 0), default=0)
    parser.add_argument('--net', type=lambda t: int(t, 0), default=0, help='network number of the segment')
    parser.add_argument('--node', type=lambda t: int(t, 16), default=CFP_NODE, help='node number in hex (default FF02)')
    parser.add_argument('--export', action='append', default=[], help='NAME=PATH[,ro]; repeatable')
    parser.add_argument('--allow', action='append', default=[], help='permitted CNET address; repeatable')
    parser.add_argument('--allow-any', action='store_true')
    parser.add_argument('--max-sessions', type=int, default=8)
    parser.add_argument('--require-encryption', action='store_true')
    parser.add_argument('--key', type=Path, help='static X25519 key (default ~/.cpt32/l3p/key)')
    parser.add_argument('--allow-key', action='append', default=[], help='client key fingerprint; repeatable')
    parser.add_argument('--max-fids', type=int, default=64)
    parser.add_argument('--verbose', action='store_true')
    args = parser.parse_args(argv)
    if not args.export:
        parser.error('give at least one --export NAME=PATH')
    if args.max_fids < 16:
        parser.error('--max-fids must be at least 16')
    try:
        exports = [parse_export(e) for e in args.export]
    except (ValueError, OSError, fs2.Fs2Error) as error:
        parser.error(str(error))
    if not args.allow and not args.allow_any and any(isinstance(e.backend, HostDir) for e in exports):
        l3p.log(f'warning: every node of network {args.net:04X} may use the exported host directories')
    key = None
    if l3p.HAVE_CRYPTO:
        key = l3p.load_or_create_key(args.key)
        l3p.log('server key fingerprint', l3p.fingerprint(l3p.public_key(key)))
    elif args.require_encryption or args.allow_key:
        parser.error('encryption needs the cryptography package')
    segment = l3p.Segment(l3p.parse_hostport(args.bind), args.segment_id,
                          l3p.parse_hostport(args.multicast) if args.multicast else None,
                          [l3p.parse_hostport(p) for p in args.peer])
    node = l3p.Node(segment, args.net, args.node)
    server = CfpServer(exports, args.max_fids, args.verbose)
    endpoint = l3p.Endpoint(node, CFP_SOCKET, key=key, require_encryption=args.require_encryption,
                            allow=make_allow(args.allow, args.allow_any, args.net),
                            allow_keys=args.allow_key or None, max_assocs=args.max_sessions,
                            verbose=args.verbose)
    endpoint.listen(l3p.KIND_TXN, server)
    segment.start()
    l3p.log(f'CFP server {l3p.format_addr(args.net, args.node)} socket {CFP_SOCKET} on UDP port {segment.port}: ' +
            ', '.join(f'{e.name}{" (ro)" if e.read_only else ""}' for e in exports))
    try:
        while True:
            time.sleep(0.5)
            server.tick()
    except KeyboardInterrupt:
        pass
    finally:
        for export in exports:
            try:
                export.backend.flush()
            except CfpError as error:
                l3p.log('flush failed:', error)
        endpoint.close()
        segment.close()


if __name__ == '__main__':
    main()
