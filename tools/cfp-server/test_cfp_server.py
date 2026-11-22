"""Tests of the CFP server (doc/spec_NET_FP.md §14) over localhost UDP.

    python -m unittest discover -s tools/cfp-server -v
"""
import os
from pathlib import Path
import struct
import sys
import tempfile
import time
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / 'l3p'))
import cfp_server as cfp  # noqa: E402
import l3p  # noqa: E402
import fs2  # noqa: E402

sys.path.insert(0, str(HERE.parent / 'l3p'))
from test_l3p import Lossy, make_pair  # noqa: E402

RW = cfp.O_READ | cfp.O_WRITE


class Base:
    """Runs the same protocol tests against one backend (set in subclasses)."""

    def make_backend(self, tmp):
        raise NotImplementedError

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='cfp-')
        tmp = Path(self.tmp.name)
        self.backend = self.make_backend(tmp)
        (tmp / 'ro').mkdir()
        (tmp / 'ro' / 'hello.txt').write_bytes(b'hello')
        self.seg_a, self.seg_b, self.cnode, self.snode = make_pair()
        self.server = cfp.CfpServer([cfp.Export('share', self.backend),
                                     cfp.Export('ro', cfp.HostDir(tmp / 'ro', read_only=True))], max_fids=32)
        self.sep = l3p.Endpoint(self.snode, cfp.CFP_SOCKET)
        self.sep.listen(l3p.KIND_TXN, self.server)
        self.cep = l3p.Endpoint(self.cnode)
        self.c = cfp.CfpClient(self.cep, 1, 0xFF02)
        self.c.attach(1, 'share')

    def tearDown(self):
        self.cep.close()
        self.sep.close()
        self.seg_a.close()
        self.seg_b.close()
        self.tmp.cleanup()

    def assertCode(self, code, fn, *args, **kw):
        with self.assertRaises(cfp.CfpRemoteError) as cm:
            fn(*args, **kw)
        self.assertEqual(cm.exception.code, code)

    def make_file(self, name, data=b'', fid=10):
        self.c.walk(1, fid, [])
        self.c.create(fid, name)
        if data:
            for i in range(0, len(data), cfp.IOUNIT):
                self.c.write(fid, i, data[i:i + cfp.IOUNIT])
        self.c.clunk(fid)

    # -- session --

    def test_hello_and_order(self):
        self.assertEqual((self.c.version, self.c.iounit, self.c.max_fids), (1, 960, 32))
        assoc = self.cep.open_txn(1, 0xFF02, 9)
        rsp = assoc.call(cfp.HDR.pack(cfp.T_STAT, 0, 0) + struct.pack('>I', 1))
        self.assertCode('BAD_ARG', cfp.CfpClient.parse, cfp.T_STAT, rsp)
        rsp = assoc.call(cfp.HDR.pack(cfp.T_HELLO, 0, 0) + struct.pack('>IHH', 2, 0, 0))
        self.assertCode('UNSUPPORTED', cfp.CfpClient.parse, cfp.T_HELLO, rsp)
        rsp = assoc.call(cfp.HDR.pack(cfp.T_HELLO, 0, 1) + struct.pack('>IHH', 1, 0, 0))
        self.assertCode('BAD_ARG', cfp.CfpClient.parse, cfp.T_HELLO, rsp)
        rsp = assoc.call(cfp.HDR.pack(0x30, 0, 0))
        self.assertCode('UNSUPPORTED', cfp.CfpClient.parse, 0x30, rsp)

    def test_exports(self):
        found = cfp.list_exports(self.cep, 1, 0xFF02)
        self.assertEqual(found[0][1], [('share', 0), ('ro', 1)])
        found = cfp.list_exports(self.cep, 1, l3p.BROADCAST)
        self.assertEqual(len(found), 1)

    def test_attach_errors(self):
        self.assertCode('NOT_FOUND', self.c.attach, 2, 'nope')
        self.assertCode('ACCESS', self.c.attach, 2, 'share', uid=5)
        self.assertCode('EXISTS', self.c.attach, 1, 'share')
        qid, flags = self.c.attach(2, 'SHARE')  # case-insensitive
        self.assertEqual((qid[0], flags), (cfp.DIR, 0))

    # -- files --

    def test_create_write_read_append_stat(self):
        self.c.walk(1, 2, [])
        self.c.create(2, 'a.txt')
        self.assertEqual(self.c.write(2, 0, b'hello'), (5, 5))
        self.assertEqual(self.c.write(2, 8, b'!'), (1, 9))  # hole filled with zeros
        self.assertEqual(self.c.read(2, 0, 960), b'hello\0\0\0!')
        self.assertEqual(self.c.read(2, 100, 10), b'')
        qid, st = self.c.stat(2)
        kind, mode, uid, flags, hi, lo = struct.unpack_from('>6I', st)
        self.assertEqual((kind, uid, lo, flags & 2), (cfp.FILE, 0, 9, 2))
        self.c.clunk(2)
        self.c.walk(1, 3, ['a.txt'])
        self.c.open(3, cfp.O_WRITE | cfp.O_APPEND)
        self.assertEqual(self.c.write(3, 0, b'xy'), (2, 11))
        self.assertCode('ACCESS', self.c.read, 3, 0, 1)
        self.c.clunk(3)
        self.c.walk(1, 3, ['A.TXT'])  # FS2 case folding
        self.c.open(3, cfp.O_READ | cfp.O_WRITE | cfp.O_TRUNC)
        self.assertEqual(self.c.read(3, 0, 10), b'')
        self.assertCode('BAD_ARG', self.c.read, 3, 0, 961)
        self.assertCode('BAD_ARG', self.c.read, 3, 0, 0)

    def test_open_rules(self):
        self.make_file('f')
        self.c.walk(1, 2, ['f'])
        self.assertCode('BAD_ARG', self.c.open, 2, cfp.O_READ | cfp.O_CREATE)
        self.assertCode('BAD_ARG', self.c.open, 2, cfp.O_TRUNC)
        self.c.walk(1, 3, [])
        self.assertCode('IS_DIR', self.c.open, 3, RW)
        self.c.open(3, cfp.O_READ)
        self.assertCode('BAD_ARG', self.c.walk, 3, 4, ['f'])  # open fids cannot walk
        self.assertCode('EXISTS', self.c.create, 1, 'f')
        self.assertCode('BAD_HANDLE', self.c.stat, 99)
        self.assertCode('EXISTS', self.c.walk, 1, 2, [])

    def test_walk(self):
        self.c.walk(1, 2, [])
        self.c.create(2, 'd', cfp.DIR, 0)  # fid 2 now names d
        self.c.create(2, 'e', cfp.DIR, 0)  # so this is d/e
        self.c.clunk(2)
        self.assertEqual(self.c.walk(1, 3, ['d', 'e'])[:2], (2, 0))
        count, status, qids = self.c.walk(1, 6, ['d', 'zz'])
        self.assertEqual((count, status), (1, cfp.E['NOT_FOUND']))
        self.assertCode('BAD_HANDLE', self.c.stat, 6)  # not created
        self.assertCode('NOT_FOUND', self.c.walk, 1, 6, ['zz'])
        self.assertCode('BAD_PATH', self.c.walk, 1, 6, ['.'])
        self.assertCode('BAD_PATH', self.c.walk, 1, 6, [''])
        self.assertEqual(self.c.walk(1, 6, ['..', '..'])[0], 2)  # the root's parent is the root
        self.assertEqual(self.c.stat(6)[0], self.c.stat(1)[0])
        self.assertEqual(self.c.walk(1, 4, ['d'] + ['..', 'd'] * 7 + ['..'])[:2], (16, 0))
        self.assertCode('BAD_ARG', self.c.walk, 1, 5, ['d'] * 17)
        self.make_file('plain')
        self.assertEqual(self.c.walk(1, 5, ['plain', 'x'])[:2], (1, cfp.E['NOT_DIR']))
        self.c.walk(1, 1, ['d'])  # NEWFID = FID replaces it
        self.assertEqual(self.c.listdir(self._opendir(1)), ['e'])

    def _opendir(self, fid):
        self.c.walk(fid, 20, [])
        self.c.open(20, cfp.O_READ)
        return 20

    def test_readdir_cookies(self):
        names = [f'file{i:03d}' for i in range(40)]
        for name in names:
            self.make_file(name)
        d = self._opendir(1)
        got = []
        cookie = 0
        while True:
            cookie, entries = self.c.readdir(d, cookie, 300)
            if not entries:
                break
            self.assertLessEqual(len(entries), 3)
            got += [n for n, _ in entries]
        self.assertEqual(sorted(got), names)
        self.assertCode('LIMIT', self.c.readdir, d, 0, 76)
        self.assertCode('BAD_ARG', self.c.readdir, d, 9999, 900)
        self.assertCode('BAD_ARG', self.c.readdir, d, 0, 75)

    def test_names_limits(self):
        long900 = '😀' * 225  # 900 bytes, 225 characters (FS2 allows 256)
        if not (os.name == 'nt' and isinstance(self.backend, cfp.HostDir)):  # NTFS: 255 UTF-16 units
            self.c.walk(1, 2, [])
            self.c.create(2, long900)
            self.c.clunk(2)
            self.assertEqual(self.c.walk(1, 3, [long900])[:2], (1, 0))
        self.c.walk(1, 4, [])
        self.assertCode('NAME_TOO_LONG', self.c.create, 4, long900 + 'x')
        self.assertCode('BAD_PATH', self.c.create, 4, 'a/b')
        self.assertCode('BAD_PATH', self.c.create, 4, '..')

    def test_remove_rename_busy(self):
        self.c.walk(1, 2, [])
        self.c.create(2, 'dir', cfp.DIR, 0)
        self.c.create(2, 'f', cfp.FILE, RW)  # fid 2 is now the open file dir/f
        self.c.walk(1, 3, ['dir'])
        self.c.walk(3, 4, ['f'])
        self.assertCode('BUSY', self.c.remove, 4)  # removed fid even on error
        self.assertCode('BAD_HANDLE', self.c.stat, 4)
        self.assertCode('BUSY', self.c.rename, 3, 'f', 3, 'g')
        self.c.clunk(2)
        self.c.rename(3, 'f', 1, 'moved')
        self.assertEqual(self.c.walk(1, 5, ['moved'])[:2], (1, 0))
        self.c.walk(1, 6, ['dir'])
        self.assertCode('IS_DIR', self.c.remove, 6)
        self.c.walk(1, 6, ['moved'])
        self.assertCode('NOT_DIR', self.c.remove, 6, True)
        self.make_file('x')
        self.assertCode('EXISTS', self.c.rename, 1, 'moved', 1, 'X')
        self.c.rename(1, 'moved', 1, 'MOVED')  # change of case only
        self.c.walk(1, 7, ['dir'])
        self.c.walk(7, 8, [])
        self.c.create(8, 'sub', cfp.DIR, 0)
        self.c.walk(1, 9, ['dir'])
        self.assertCode('NOT_EMPTY', self.c.remove, 9, True)
        self.assertCode('BAD_PATH', self.c.rename, 1, 'dir', 7, 'loop')
        self.c.walk(1, 10, [])
        self.assertCode('ACCESS', self.c.remove, 10, True)
        self.c.attach(11, 'ro')
        self.assertCode('CROSS_DEVICE', self.c.rename, 1, 'x', 11, 'x')

    def test_setattr_sync_statfs(self):
        self.make_file('f', b'0123456789')
        self.c.walk(1, 2, ['f'])
        self.c.setattr(2, mode=0x30)
        self.assertEqual(struct.unpack_from('>I', self.c.stat(2)[1], 4)[0] & 0x30, 0x30)
        self.assertCode('ACCESS', self.c.setattr, 2, size=3)
        self.assertCode('BAD_ARG', self.c.setattr, 2)
        self.c.open(2, RW)
        self.c.setattr(2, size=3)
        self.assertEqual(self.c.read(2, 0, 100), b'012')
        self.c.sync(2)
        fs_type, flags, *_, slot, _ = self.c.statfs(1)
        self.assertEqual((fs_type, flags, slot), (self.backend.fs_type, 0, 0xFFFFFFFF))

    def test_read_only_export(self):
        self.c.attach(2, 'ro')
        self.c.walk(2, 3, ['hello.txt'])
        self.assertCode('READ_ONLY', self.c.open, 3, RW)
        self.c.open(3, cfp.O_READ)
        self.assertEqual(self.c.read(3, 0, 100), b'hello')
        self.assertCode('READ_ONLY', self.c.create, 2, 'n')
        self.c.walk(2, 4, ['hello.txt'])
        self.assertCode('READ_ONLY', self.c.remove, 4)
        self.assertEqual(self.c.statfs(2)[1], 1)

    def test_bad_input(self):
        self.assertCode('BAD_ARG', self.c.call, cfp.T_STAT, b'\0\0')
        self.assertCode('BAD_ARG', self.c.call, cfp.T_STAT, b'\0\0\0\1\0')
        self.assertCode('BAD_PATH', self.c.call, cfp.T_WALK, struct.pack('>IIHH', 1, 2, 1, 0) + b'\0\2\xff\xfe')
        self.assertCode('BAD_ARG', self.c.call, cfp.T_SETATTR, struct.pack('>IIIIQqHH', 1, 0x10, 0, 0, 0, 0, 0, 0))

    def test_fid_limit_and_session_cleanup(self):
        for i in range(2, 33):
            self.c.walk(1, i, [])
        self.assertCode('LIMIT', self.c.walk, 1, 40, [])
        # a file left open by one session is busy for another until it closes
        self.c.clunk(32)
        self.c.walk(1, 32, [])
        self.c.create(32, 'held')
        other = cfp.CfpClient(self.cep, 1, 0xFF02)
        other.attach(1, 'share')
        other.walk(1, 2, ['held'])
        self.assertCode('BUSY', other.remove, 2)
        self.c.close()
        time.sleep(0.3)
        other.walk(1, 2, ['held'])
        other.remove(2)

    def test_exactly_once_under_loss(self):
        self.seg_a.mangle = Lossy(21, drop=0.2, dup=0.1, reorder=0.1)
        self.seg_b.mangle = Lossy(22, drop=0.2, dup=0.1, reorder=0.1)
        self.c.walk(1, 2, [])
        self.c.create(2, 'log', cfp.FILE, cfp.O_WRITE | cfp.O_APPEND)
        for i in range(100):
            self.assertEqual(self.c.write(2, 0, b'%03d,' % i)[1], 4 * (i + 1))
        self.c.clunk(2)
        for i in range(20):
            self.c.walk(1, 3, [])
            self.c.create(3, f'n{i}', cfp.DIR, 0)
            self.c.clunk(3)
        self.c.walk(1, 4, ['log'])
        self.c.open(4, cfp.O_READ)
        self.assertEqual(self.c.read(4, 0, 960), b''.join(b'%03d,' % i for i in range(100)))

    def test_bulk_transfer_with_loss(self):
        # 256 KiB keeps the run short: every loss costs the 300 ms RTO, so 1 MiB
        # at 20% loss takes about four minutes (measured; it also passes).
        data = os.urandom(256 * 1024)
        self.seg_a.mangle = Lossy(None, drop=0.2)
        self.seg_b.mangle = Lossy(None, drop=0.2)
        self.c.walk(1, 2, [])
        self.c.create(2, 'big')
        pending = []
        for offset in range(0, len(data), cfp.IOUNIT):
            chunk = data[offset:offset + cfp.IOUNIT]
            body = struct.pack('>IQHH', 2, offset, len(chunk), 0) + chunk
            pending.append(self.c.assoc.send(cfp.HDR.pack(cfp.T_WRITE, 0, 0) + body))
            if len(pending) == 4:
                cfp.CfpClient.parse(cfp.T_WRITE, self.c.assoc.wait(pending.pop(0)))
        for xid in pending:
            cfp.CfpClient.parse(cfp.T_WRITE, self.c.assoc.wait(xid))
        got = bytearray()
        offsets = list(range(0, len(data), cfp.IOUNIT))
        pending = []
        while offsets or pending:
            while offsets and len(pending) < 4:
                body = struct.pack('>IQHH', 2, offsets.pop(0), cfp.IOUNIT, 0)
                pending.append(self.c.assoc.send(cfp.HDR.pack(cfp.T_READ, 0, 0) + body))
            rsp = cfp.CfpClient.parse(cfp.T_READ, self.c.assoc.wait(pending.pop(0)))
            got += rsp[4:4 + struct.unpack_from('>H', rsp)[0]]
        self.assertEqual(bytes(got), data)


class HostDirTests(Base, unittest.TestCase):
    def make_backend(self, tmp):
        (tmp / 'share').mkdir()
        self.root = tmp / 'share'
        return cfp.HostDir(self.root)

    def test_host_names(self):
        (self.root / 'bad\x01name').write_bytes(b'') if os.name != 'nt' else None
        (self.root / 'ok').write_bytes(b'1')
        names = self.c.listdir(self._opendir(1))
        self.assertEqual(names, ['ok'])
        if os.name != 'nt':
            (self.root / 'OK').write_bytes(b'2')  # case-only duplicate: hidden, exact match wins
            os.symlink('/etc', self.root / 'outside')
            self.assertEqual(len(self.c.listdir(self._opendir(1))), 1)  # one of ok / OK
            self.assertCode('NOT_FOUND', self.c.walk, 1, 2, ['outside'])


class Fs2Tests(Base, unittest.TestCase):
    def make_backend(self, tmp):
        path = tmp / 'disk.fs2'
        path.write_bytes(fs2.format_image(4096, inode_count=512))
        self.path = path
        return cfp.Fs2Image(path)

    def test_changes_reach_the_image(self):
        self.make_file('saved', b'data')
        image = fs2.Volume(bytearray(self.path.read_bytes()))
        self.assertEqual(image.read(image.resolve('/saved')), b'data')
        self.assertEqual(fs2.check_image(bytearray(self.path.read_bytes())), [])


if __name__ == '__main__':
    unittest.main()
