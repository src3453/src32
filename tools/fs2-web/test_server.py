import importlib.util
import pathlib
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('fs2web', pathlib.Path(__file__).with_name('server.py'))
web = importlib.util.module_from_spec(spec)
spec.loader.exec_module(web)

class FakeMonitor:
    def __init__(self, image, fail=None):
        self.image = bytearray(image); self.writes = []; self.reads = []; self.fail = fail
    def connect(self): return len(self.image) // 512
    def read_block(self, lba):
        self.reads.append(lba)
        return bytes(self.image[lba*512:(lba+1)*512])
    def write_block(self, lba, data):
        self.writes.append((lba, bytes(data)))
        if self.fail == len(self.writes): raise IOError('injected failure')
        self.image[lba*512:(lba+1)*512] = data

class UpdateTests(unittest.TestCase):
    def test_update_sets_dirty_before_changes_and_clears_last(self):
        m = FakeMonitor(web.fs2.format_image(128, inode_count=16))
        web.update(m, '/test.bin', b'hello')
        self.assertEqual(m.writes[0][0], 1)
        self.assertEqual(m.writes[-1][0], 1)
        self.assertEqual(struct.unpack_from('>H', m.writes[0][1], 6)[0], 1)
        self.assertEqual(struct.unpack_from('>H', m.writes[-1][1], 6)[0], 0)
        self.assertEqual(web.fs2.check_image(m.image), [])
        v = web.fs2.Volume(m.image)
        self.assertEqual(v.read(v.resolve('/test.bin')), b'hello')
    def test_rescue_repairs_bitmap_and_orphan_with_backup(self):
        img = web.fs2.format_image(128, inode_count=16)
        v = web.fs2.Volume(img)
        v.put('/keep', b'kept')
        orphan = v.put('/orphan', b'lost')
        v.set_entry(v.resolve('/'), v.find(v.resolve('/'), 'orphan')[0], 0, '')
        v.set_bit(v.resolve('/keep')['first_block'], False)
        img[519] = 1
        before = bytes(img)
        m = FakeMonitor(img)
        with tempfile.TemporaryDirectory() as directory:
            backup, count, _ = web.rescue(m, directory)
            self.assertEqual(pathlib.Path(backup).read_bytes(), before)
        self.assertEqual(count, 1)
        self.assertEqual(web.fs2.check_image(m.image), [])
        v = web.fs2.Volume(m.image)
        self.assertEqual(v.read(v.resolve('/keep')), b'kept')
        self.assertEqual(v.read_inode(orphan['number'])['type'], web.fs2.FREE)
        self.assertEqual(m.writes[-1][0], 1)
        self.assertEqual(m.writes[-1][1][7] & 1, 0)

    def test_rescue_refuses_shared_blocks_and_keeps_backup(self):
        img = web.fs2.format_image(128, inode_count=16)
        v = web.fs2.Volume(img); a = v.put('/a', b'a'); b = v.put('/b', b'b')
        b['first_block'] = a['first_block']; v.write_inode(b['number'], b)
        img[519] = 1; m = FakeMonitor(img)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(IOError, '共有'):
                web.rescue(m, directory)
            self.assertEqual(next(pathlib.Path(directory).iterdir()).read_bytes(), img)
        self.assertEqual(m.writes, [])

    def test_rescue_write_failure_does_not_clear_dirty(self):
        img = web.fs2.format_image(128, inode_count=16)
        v = web.fs2.Volume(img); v.set_bit(2, False); img[519] = 1
        m = FakeMonitor(img, fail=1)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(IOError): web.rescue(m, directory)
        self.assertEqual(m.image[519] & 1, 1)

    def test_rescue_clean_volume_rejected(self):
        m = FakeMonitor(web.fs2.format_image(128, inode_count=16))
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError): web.rescue(m, directory)
            self.assertEqual(list(pathlib.Path(directory).iterdir()), [])
        self.assertEqual(m.writes, [])

    def test_restore_clean_and_dirty_backups(self):
        for was_dirty in (False, True):
            img = web.fs2.format_image(64, inode_count=16)
            web.fs2.Volume(img).put('/restored', b'restore data')
            if was_dirty: img[519] |= 1
            m = FakeMonitor(web.fs2.format_image(128, inode_count=16))
            count, dirty = web.restore(m, img)
            self.assertEqual((count, dirty), (64, was_dirty))
            self.assertEqual(m.image[:len(img)], img)
            self.assertEqual(m.writes[0][0], 1)
            self.assertEqual(m.writes[0][1][7] & 1, 1)
            if not was_dirty:
                self.assertEqual(m.writes[-1][0], 1)
                self.assertEqual(m.writes[-1][1][7] & 1, 0)
            self.assertEqual(web.fs2.check_image(m.image, allow_dirty=was_dirty), [])

    def test_restore_rejects_invalid_or_oversized_before_write(self):
        img = web.fs2.format_image(64, inode_count=16)
        m = FakeMonitor(web.fs2.format_image(32, inode_count=8))
        with self.assertRaises(ValueError): web.restore(m, img)
        self.assertEqual(m.writes, [])
        bad = bytearray(img); bad[512] = 0
        with self.assertRaises(ValueError): web.restore(m, bad)
        self.assertEqual(m.writes, [])
        with self.assertRaises(ValueError): web.validate_backup(img + bytes(512))

    def test_restore_failure_keeps_dirty(self):
        img = web.fs2.format_image(64, inode_count=16)
        m = FakeMonitor(web.fs2.format_image(64, inode_count=16), fail=3)
        with self.assertRaisesRegex(IOError, 'DIRTY'): web.restore(m, img)
        self.assertEqual(m.image[519] & 1, 1)

    def test_listing_rejects_dirty_and_corrupt_chain(self):
        img = web.fs2.format_image(128, inode_count=16)
        web.fs2.Volume(img).put('/file', b'x')
        dirty = bytearray(img); dirty[519] = 1
        with self.assertRaises(ValueError): web.list_directory(FakeMonitor(dirty), '/')
        volume = web.fs2.Volume(img)
        root = volume.resolve('/')
        struct.pack_into('>I', img, root['first_block'] * 512, root['first_block'])
        with self.assertRaises(ValueError): web.list_directory(FakeMonitor(img), '/')

    def test_backup_reads_dirty_volume_without_writes(self):
        img = web.fs2.format_image(128, inode_count=16)
        img[519] = 1
        m = FakeMonitor(img)
        self.assertEqual(web.read_volume(m), img)
        self.assertEqual(m.writes, [])

    def test_directory_listing(self):
        img = web.fs2.format_image(128, inode_count=16)
        volume = web.fs2.Volume(img)
        volume.mkdir('/dir')
        volume.put('/dir/hello.txt', b'abc')
        m = FakeMonitor(img)
        entries = web.list_directory(m, '/')
        self.assertEqual([e['name'] for e in entries], ['dir'])
        entries = web.list_directory(m, '/dir')
        self.assertEqual(entries[0]['name'], 'hello.txt')
        self.assertEqual(entries[0]['size'], 3)
        self.assertLess(len(m.reads), 256)
        self.assertEqual(m.writes, [])

    def test_volume_usage_counts_only_valid_bitmap_bits(self):
        img = web.fs2.format_image(4101, inode_count=16)
        volume = web.fs2.Volume(img)
        volume.put('/file', b'x' * 1000)
        expected = sum(volume.bit(block) for block in range(4101))
        # Out-of-volume padding is excluded even if those bits are set.
        bitmap_end = (volume.sb['bitmap_start'] + volume.sb['bitmap_blocks']) * 512
        img[bitmap_end - 1] = 0xff
        m = FakeMonitor(img)
        entries, usage = web.list_directory(m, '/', with_usage=True)
        self.assertEqual(entries[0]['name'], 'file')
        self.assertEqual(usage, {'total_sectors': 4101, 'used_sectors': expected,
                                 'free_sectors': 4101 - expected, 'sector_bytes': 512})
        self.assertIn(volume.sb['bitmap_start'] + 1, m.reads)
        self.assertLess(len(m.reads), 30)
        self.assertEqual(m.writes, [])

    def test_download_does_not_write(self):
        img = web.fs2.format_image(128, inode_count=16)
        web.fs2.Volume(img).put('/test', b'content')
        m = FakeMonitor(img)
        self.assertEqual(web.download(m, '/test'), b'content')
        self.assertEqual(m.writes, [])

    def test_interrupted_update_keeps_dirty(self):
        m = FakeMonitor(web.fs2.format_image(128, inode_count=16), fail=2)
        with self.assertRaises(IOError): web.update(m, '/test', b'hello')
        self.assertEqual(m.image[519] & 1, 1)
    def test_dirty_volume_refused_without_writes(self):
        img = web.fs2.format_image(128, inode_count=16); img[519] = 1
        m = FakeMonitor(img)
        with self.assertRaises(web.fs2.Fs2Error): web.update(m, '/test', b'hello')
        self.assertEqual(m.writes, [])
    def test_upload_directory_path_rejected_before_connect(self):
        class NoConnect(FakeMonitor):
            def connect(self): raise AssertionError('must not connect')
        for path in ('/', '/bin/', 'relative', '/.', '/..'):
            with self.subTest(path=path), self.assertRaises((ValueError, web.fs2.Fs2Error)):
                web.update(NoConnect(web.fs2.format_image(128, inode_count=16)), path, b'x')

    def test_invalid_path_does_not_write(self):
        m = FakeMonitor(web.fs2.format_image(128, inode_count=16))
        with self.assertRaises(web.fs2.Fs2Error): web.update(m, '/missing/test', b'hello')
        self.assertEqual(m.writes, [])

if __name__ == '__main__': unittest.main()
