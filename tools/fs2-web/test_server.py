import importlib.util
import pathlib
import struct
import unittest

spec = importlib.util.spec_from_file_location('fs2web', pathlib.Path(__file__).with_name('server.py'))
web = importlib.util.module_from_spec(spec)
spec.loader.exec_module(web)

class FakeMonitor:
    def __init__(self, image, fail=None):
        self.image = bytearray(image); self.writes = []; self.fail = fail
    def connect(self): return len(self.image) // 512
    def read_block(self, lba): return bytes(self.image[lba*512:(lba+1)*512])
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
    def test_invalid_path_does_not_write(self):
        m = FakeMonitor(web.fs2.format_image(128, inode_count=16))
        with self.assertRaises(web.fs2.Fs2Error): web.update(m, '/missing/test', b'hello')
        self.assertEqual(m.writes, [])

if __name__ == '__main__': unittest.main()
