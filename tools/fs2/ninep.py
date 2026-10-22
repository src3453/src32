"""Small, synchronous-per-connection 9P2000 adapter for the FS2 host utility."""
import contextlib
import os
import socketserver
import struct
import threading
from pathlib import Path

import fs2

DMDIR = 0x80000000
NOFID = 0xFFFFFFFF
MAX_MSIZE = 1024 * 1024


def string(value):
    raw = value.encode('utf-8')
    return struct.pack('<H', len(raw)) + raw


class Reader:
    def __init__(self, data):
        self.data = data
        self.offset = 0

    def take(self, count):
        if count > len(self.data) - self.offset:
            raise ValueError('short message')
        result = self.data[self.offset:self.offset + count]
        self.offset += count
        return result

    def unpack(self, fmt):
        return struct.unpack('<' + fmt, self.take(struct.calcsize('<' + fmt)))

    def text(self):
        return self.take(self.unpack('H')[0]).decode('utf-8')

    def end(self):
        if self.offset != len(self.data):
            raise ValueError('trailing message bytes')


def posix_mode(inode):
    other = inode['mode'] & 7
    return ((DMDIR if inode['type'] == fs2.DIR else 0) |
            ((inode['mode'] >> 3) << 6) | (other << 3) | other)


def fs2_mode(mode):
    if mode & ~(DMDIR | 0o777):
        raise ValueError('unsupported mode bits')
    # FS2 has no group: the POSIX group bits are projected onto other.
    return ((mode >> 6) & 7) << 3 | (mode & 7)


def qid(inode, generation=0):
    version = (inode['modified_sec'] * 1000 + inode['modified_ms']) & 0xFFFFFFFF
    return struct.pack('<BIQ', 0x80 if inode['type'] == fs2.DIR else 0,
                       version, (generation << 32) | inode['number'])


def stat(inode, name, generation=0):
    stamp = max(0, min(0xFFFFFFFF, inode['modified_sec']))
    body = (struct.pack('<HI', 0, 0) + qid(inode, generation) +
            struct.pack('<IIIQ', posix_mode(inode), stamp, stamp,
                        0 if inode['type'] == fs2.DIR else inode['size']) +
            string(name) + string(str(inode['uid'])) + string('0') + string(''))
    return struct.pack('<H', len(body)) + body


class Export:
    def __init__(self, image, read_only=False):
        self.path = Path(image)
        self.volume = fs2.Volume(fs2._load(image))
        self.read_only = read_only
        self.lock = threading.RLock()
        self.generations = {}

    def generation(self, number):
        return self.generations.get(number, 0)

    def change(self, operation):
        if self.read_only:
            raise ValueError('read-only filesystem')
        # Operations can fail after partially changing a Volume. Always use a copy.
        volume = fs2.Volume(bytearray(self.volume.img), check=False)
        result = operation(volume)
        problems = fs2.check_image(volume.img)
        if problems:
            raise fs2.Fs2Error(*problems[0])
        fs2._save(self.path, volume.img)
        self.volume = volume
        return result


class Session:
    def __init__(self, export):
        self.export = export
        self.fids = {}  # fid -> [inode number, incarnation, open mode or None]
        self.msize = MAX_MSIZE
        self.negotiated = False

    def handle(self, kind, payload):
        with self.export.lock:
            try:
                return kind + 1, self.dispatch(kind, Reader(payload))
            except (fs2.Fs2Error, ValueError, OSError, UnicodeError, struct.error) as error:
                message = str(error).encode('utf-8')[:self.msize - 9].decode('utf-8', 'ignore')
                return 107, string(message)

    def inode(self, fid):
        state = self.fids.get(fid)
        if state is None:
            raise ValueError('unknown fid')
        number, generation, _ = state
        inode = self.export.volume.read_inode(number)
        if generation != self.export.generation(number) or inode['type'] == fs2.FREE:
            raise ValueError('stale fid')
        return inode

    def bind(self, fid, inode, mode=None):
        self.fids[fid] = [inode['number'], self.export.generation(inode['number']), mode]

    def path(self, inode):
        parts = []
        volume = self.export.volume
        while inode['number'] != fs2.ROOT_INODE:
            parent = volume.read_inode(inode['parent'])
            parts.append(next(name for _, number, name in volume.entries(parent)
                              if number == inode['number']))
            inode = parent
        return '/' + '/'.join(reversed(parts))

    def qid(self, inode):
        return qid(inode, self.export.generation(inode['number']))

    def resize(self, volume, inode, size):
        if size > volume.sb['total_blocks'] * fs2.PAYLOAD:
            raise ValueError('file too large')
        released = volume.truncate(inode, size) or []
        volume.touch(inode, fs2.now_time())
        volume.write_inode(inode['number'], inode)
        volume.release(released)

    @staticmethod
    def open_mode(inode, mode):
        if mode & ~0x13 or mode & 3 == 3:
            raise ValueError('unsupported open mode')
        if inode['type'] == fs2.DIR and mode != 0:
            raise ValueError('directory must be opened for reading')
        if mode & 0x10 and mode & 3 == 0:
            raise ValueError('truncate requires write access')

    def dispatch(self, kind, r):
        if kind == 100:  # version
            msize, = r.unpack('I')
            version = r.text()
            r.end()
            if msize < 256:
                raise ValueError('msize must be at least 256')
            self.msize = min(msize, MAX_MSIZE)
            self.fids.clear()
            self.negotiated = version == '9P2000'
            return struct.pack('<I', self.msize) + string('9P2000' if self.negotiated else 'unknown')
        if not self.negotiated:
            raise ValueError('negotiate 9P2000 first')
        if kind == 102:
            raise ValueError('authentication is not supported')
        if kind == 108:  # synchronous requests finish before flush is processed
            r.unpack('H')
            r.end()
            return b''
        if kind == 104:
            fid, afid = r.unpack('II')
            r.text()  # uname: this administrative export always acts as UID 0
            aname = r.text()
            r.end()
            if fid in self.fids or fid == NOFID or afid != NOFID or aname not in ('', '/'):
                raise ValueError('invalid attach')
            inode = self.export.volume.read_inode(fs2.ROOT_INODE)
            self.bind(fid, inode)
            return self.qid(inode)
        fid, = r.unpack('I')
        if kind == 120:
            r.end()
            if fid not in self.fids:
                raise ValueError('unknown fid')
            del self.fids[fid]
            return b''
        try:
            inode = self.inode(fid)
        except ValueError:
            if kind == 122:
                self.fids.pop(fid, None)
            raise
        volume = self.export.volume
        opened = self.fids[fid][2]
        if kind == 110:
            newfid, count = r.unpack('IH')
            names = [r.text() for _ in range(count)]
            r.end()
            if count > 16 or newfid == NOFID or (newfid != fid and newfid in self.fids):
                raise ValueError('invalid walk fid/count')
            if opened is not None:
                raise ValueError('cannot walk an open fid')
            qids = []
            for name in names:
                try:
                    if inode['type'] != fs2.DIR:
                        raise ValueError('not a directory')
                    if name not in ('.', '..'):
                        fs2.check_name(name)
                    inode = volume.resolve(self.path(inode).rstrip('/') + '/' + name)
                except (ValueError, fs2.Fs2Error):
                    if not qids:
                        raise
                    return struct.pack('<H', len(qids)) + b''.join(qids)
                qids.append(self.qid(inode))
            self.bind(newfid, inode)
            return struct.pack('<H', count) + b''.join(qids)
        if kind == 112:
            mode, = r.unpack('B')
            r.end()
            if opened is not None:
                raise ValueError('fid already open')
            self.open_mode(inode, mode)
            if self.export.read_only and mode & 3 != 0:
                raise ValueError('read-only filesystem')
            if mode & 0x10:
                self.export.change(lambda v: self.resize(v, v.read_inode(inode['number']), 0))
                inode = self.export.volume.read_inode(inode['number'])
            self.fids[fid][2] = mode & 3
            return self.qid(inode) + struct.pack('<I', self.msize - 24)
        if kind == 114:
            name = r.text()
            perm, mode = r.unpack('IB')
            r.end()
            fs2.check_name(name)
            if opened is not None or inode['type'] != fs2.DIR:
                raise ValueError('create requires an unopened directory fid')
            new_kind = fs2.DIR if perm & DMDIR else fs2.FILE
            self.open_mode({'type': new_kind}, mode)
            permissions = fs2_mode(perm)
            path = self.path(inode).rstrip('/') + '/' + name
            child = self.export.change(lambda v: v.create(path, new_kind, permissions))
            self.bind(fid, child, mode & 3)
            return self.qid(child) + struct.pack('<I', self.msize - 24)
        if kind == 116:
            offset, count = r.unpack('QI')
            r.end()
            if opened not in (0, 2):
                raise ValueError('fid is not open for reading')
            count = min(count, self.msize - 11)
            if inode['type'] == fs2.DIR:
                records = [stat(volume.read_inode(n), name, self.export.generation(n))
                           for _, n, name in volume.entries(inode)]
                data, position = bytearray(), 0
                for record in records:
                    if position < offset:
                        position += len(record)
                        if position > offset:
                            raise ValueError('invalid directory offset')
                        continue
                    if len(data) + len(record) > count:
                        if not data:
                            raise ValueError('directory read buffer too small')
                        break
                    data += record
                if offset > sum(map(len, records)):
                    raise ValueError('invalid directory offset')
                data = bytes(data)
            else:
                data = volume.read(inode)[offset:offset + count]
            return struct.pack('<I', len(data)) + data
        if kind == 118:
            offset, count = r.unpack('QI')
            data = r.take(count)
            r.end()
            if opened not in (1, 2) or inode['type'] != fs2.FILE:
                raise ValueError('fid is not open for writing')
            if offset + count > volume.sb['total_blocks'] * fs2.PAYLOAD:
                raise ValueError('file too large')
            if count:
                def write(v):
                    node = v.read_inode(inode['number'])
                    v.write(node, offset, data)
                    v.touch(node, fs2.now_time())
                    v.write_inode(node['number'], node)
                self.export.change(write)
            return struct.pack('<I', count)
        if kind == 122:
            r.end()
            try:
                self.export.change(lambda v: v.remove(self.path(inode)))
                self.export.generations[inode['number']] = self.export.generation(inode['number']) + 1
            finally:
                del self.fids[fid]  # 9P remove clunks even on failure
            return b''
        if kind == 124:
            r.end()
            data = stat(inode, self.path(inode).rsplit('/', 1)[-1] or '/',
                        self.export.generation(inode['number']))
            return struct.pack('<H', len(data)) + data
        if kind == 126:
            size, = r.unpack('H')
            s = Reader(r.take(size))
            r.end()
            inner, = s.unpack('H')
            if inner != size - 2:
                raise ValueError('invalid stat length')
            typ, dev = s.unpack('HI')
            qtype, qversion, qpath = s.unpack('BIQ')
            mode, atime, mtime, length = s.unpack('IIIQ')
            name, uid, gid, muid = (s.text() for _ in range(4))
            s.end()
            if (typ, dev, qtype, qversion, qpath) != (0xFFFF, NOFID, 255, NOFID, 0xFFFFFFFFFFFFFFFF):
                raise ValueError('cannot change stat identity')
            if atime != NOFID or gid not in ('', '0') or muid:
                raise ValueError('atime, groups and muid are unsupported')
            if mode != NOFID:
                permissions = fs2_mode(mode)
                if bool(mode & DMDIR) != (inode['type'] == fs2.DIR):
                    raise ValueError('cannot change file type')
            if length != 0xFFFFFFFFFFFFFFFF and inode['type'] != fs2.FILE:
                raise ValueError('cannot truncate directory')
            path = self.path(inode)
            if name:
                fs2.check_name(name)
            def update(v):
                target = path
                if name:
                    target = path.rsplit('/', 1)[0] + '/' + name
                    v.rename(path, target)
                if mode != NOFID or uid:
                    v.set_attributes(target, mode=permissions if mode != NOFID else None,
                                     uid=int(uid) if uid else None)
                node = v.read_inode(inode['number'])
                if length != 0xFFFFFFFFFFFFFFFF:
                    self.resize(v, node, length)
                if mtime != NOFID:
                    node['modified_sec'], node['modified_ms'] = mtime, 0
                    v.touch(node, fs2.now_time(), modified=False)
                    v.write_inode(node['number'], node)
            self.export.change(update)
            return b''
        raise ValueError('unsupported 9P request')


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        session = Session(self.server.export)
        while True:
            header = self.rfile.read(4)
            if not header:
                return
            if len(header) != 4:
                return
            size, = struct.unpack('<I', header)
            if not 7 <= size <= session.msize:
                return
            message = self.rfile.read(size - 4)
            if len(message) != size - 4:
                return
            kind, tag = struct.unpack_from('<BH', message)
            reply, data = session.handle(kind, message[3:])
            if len(data) + 7 > session.msize:
                reply, data = 107, string('response exceeds msize')
            self.wfile.write(struct.pack('<IBH', len(data) + 7, reply, tag) + data)


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


@contextlib.contextmanager
def image_lock(image):
    """Sidecar advisory lock survives the image's atomic replacement."""
    with open(str(image) + '.9p-lock', 'a+b') as lock:
        if os.name == 'nt':
            import msvcrt
            lock.write(b'\0')
            lock.flush()
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def serve(image, host='127.0.0.1', port=5640, read_only=False):
    with image_lock(Path(image).resolve()):
        export = Export(image, read_only)
        with Server((host, port), Handler) as server:
            server.export = export
            print(f'FS2 9P2000: {image} on {host}:{server.server_address[1]} '
                  f'({"read-only" if read_only else "read-write"})', flush=True)
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
    return 0
