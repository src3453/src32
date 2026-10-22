"""Wire-level tests with an independent TCP client (no mount privileges needed)."""
from pathlib import Path
import socket
import struct
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fs2
import ninep


def text(value):
    data = value.encode('utf-8')
    return struct.pack('<H', len(data)) + data


class Client:
    def __init__(self, address):
        self.sock = socket.create_connection(address, timeout=3)
        self.file = self.sock.makefile('rb')
        self.tag = 0
        self.call(100, struct.pack('<I', 8192) + text('9P2000'))
        self.call(104, struct.pack('<II', 1, 0xFFFFFFFF) + text('root') + text(''))

    def close(self):
        self.file.close()
        self.sock.close()

    def call(self, kind, data=b'', error=False):
        self.tag += 1
        self.sock.sendall(struct.pack('<IBH', len(data) + 7, kind, self.tag) + data)
        size, reply, tag = struct.unpack('<IBH', self.file.read(7))
        result = self.file.read(size - 7)
        assert tag == self.tag
        if error:
            assert reply == 107, (reply, result)
        elif reply != kind + 1:
            raise AssertionError(result)
        return result

    def walk(self, fid, newfid, *names, error=False):
        return self.call(110, struct.pack('<IIH', fid, newfid, len(names)) +
                         b''.join(text(name) for name in names), error=error)

    def create(self, fid, name, directory=False):
        mode = 0 if directory else 2
        return self.call(114, struct.pack('<I', fid) + text(name) +
                         struct.pack('<IB', (0x80000000 | 0o755) if directory else 0o644, mode))

    def write(self, fid, offset, data, error=False):
        return self.call(118, struct.pack('<IQI', fid, offset, len(data)) + data, error=error)

    def read(self, fid, offset=0, count=4096, error=False):
        data = self.call(116, struct.pack('<IQI', fid, offset, count), error=error)
        return data if error else data[4:]

    def wstat(self, fid, name='', length=0xFFFFFFFFFFFFFFFF, mode=0xFFFFFFFF, **kwargs):
        body = (b'\xff' * 19 + struct.pack('<IIIQ', mode, 0xFFFFFFFF, 0xFFFFFFFF, length) +
                text(name) + text('') * 3)
        stat = struct.pack('<H', len(body)) + body
        return self.call(126, struct.pack('<IH', fid, len(stat)) + stat, **kwargs)


class NinePTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.image = Path(self.tmp.name) / 'disk.img'
        self.image.write_bytes(fs2.format_image(120, inode_count=16))
        self.server = ninep.Server(('127.0.0.1', 0), ninep.Handler)
        self.server.export = ninep.Export(self.image)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.clients = []
        self.client = self.connect()

    def connect(self):
        client = Client(self.server.server_address)
        self.clients.append(client)
        return client

    def tearDown(self):
        for client in self.clients:
            client.close()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.tmp.cleanup()

    def checked_volume(self):
        image = self.image.read_bytes()
        self.assertEqual(fs2.check_image(image), [])
        return fs2.Volume(bytearray(image))

    def test_files_directories_unicode_sparse_write_and_persistence(self):
        c = self.client
        c.walk(1, 2)
        c.create(2, '資料', directory=True)
        c.walk(1, 3, '資料')
        c.create(3, 'Straße')
        c.write(3, 509, b'hello')
        self.assertEqual(c.read(3), bytes(509) + b'hello')
        c.walk(1, 4, '資料', 'STRASSE')
        c.call(112, struct.pack('<IB', 4, 0))
        self.assertEqual(c.read(4, 509), b'hello')
        c.wstat(3, name='新名', length=2, mode=0o600)
        self.assertEqual(c.read(3), b'\0\0')
        volume = self.checked_volume()
        inode = volume.resolve('/資料/新名')
        self.assertEqual(inode['mode'], 0x30)
        c.call(122, struct.pack('<I', 3))
        c.call(122, struct.pack('<I', 2))
        self.assertEqual(self.checked_volume().entries(self.checked_volume().resolve('/')), [])

    def test_failed_mutation_rolls_back_and_session_survives_bad_messages(self):
        c = self.client
        c.walk(1, 2)
        c.create(2, 'a')
        before = self.image.read_bytes()
        c.write(2, 115 * 508, bytes(508), error=True)
        self.assertEqual(self.image.read_bytes(), before)
        c.write(2, 2**63, b'x', error=True)
        c.call(118, struct.pack('<IQI', 2, 0, 500) + b'x', error=True)
        c.call(116, b'', error=True)
        c.write(2, 0, b'good')
        self.assertEqual(c.read(2), b'good')
        self.checked_volume()

    def test_directory_records_offsets_and_stat(self):
        c = self.client
        for i, name in enumerate(('a', 'b'), 2):
            c.walk(1, i)
            c.create(i, name)
        c.walk(1, 4)
        c.call(112, struct.pack('<IB', 4, 0))
        listing = c.read(4)
        first_length = struct.unpack_from('<H', listing)[0] + 2
        self.assertEqual(c.read(4, 0, first_length), listing[:first_length])
        self.assertEqual(c.read(4, first_length), listing[first_length:])
        self.assertEqual(c.read(4, len(listing)), b'')
        c.read(4, 1, error=True)
        c.read(4, 0, 1, error=True)
        result = c.call(124, struct.pack('<I', 2))
        self.assertEqual(struct.unpack_from('<H', result)[0], len(result) - 2)

    def test_rename_keeps_other_fids_and_removed_inode_never_aliases(self):
        c = self.client
        c.walk(1, 2)
        c.create(2, 'old')
        other = self.connect()
        other.walk(1, 2, 'old')
        c.wstat(2, name='new')
        other.call(112, struct.pack('<IB', 2, 0))
        c.write(2, 0, b'new data')
        self.assertEqual(other.read(2), b'new data')
        c.call(122, struct.pack('<I', 2))
        c.walk(1, 3)
        c.create(3, 'reused')
        other.read(2, error=True)
        other.call(122, struct.pack('<I', 2), error=True)
        other.call(120, struct.pack('<I', 2), error=True)
        self.checked_volume()

    def test_readonly_and_open_mode_enforcement(self):
        c = self.client
        c.walk(1, 2)
        c.create(2, 'a')
        c.walk(1, 3, 'a')
        c.read(3, error=True)
        c.call(112, struct.pack('<IB', 3, 0))
        c.write(3, 0, b'bad', error=True)
        self.server.export.read_only = True
        c.write(2, 0, b'bad', error=True)
        c.wstat(2, name='bad', error=True)
        c.call(122, struct.pack('<I', 3), error=True)
        c.call(124, struct.pack('<I', 3), error=True)  # remove clunks on failure
        c.walk(1, 4)
        c.call(114, struct.pack('<I', 4) + text('bad') + struct.pack('<IB', 0o644, 2), error=True)
        self.assertEqual(c.read(2), b'')
        self.checked_volume()

    def test_partial_walk_and_version_reset(self):
        c = self.client
        c.walk(1, 2)
        c.create(2, 'a', directory=True)
        reply = c.walk(1, 3, 'a', 'missing')
        self.assertEqual(struct.unpack_from('<H', reply)[0], 1)
        c.call(124, struct.pack('<I', 3), error=True)
        c.call(100, struct.pack('<I', 512) + text('9P2000.L'))
        c.call(124, struct.pack('<I', 1), error=True)
        c.call(100, struct.pack('<I', 512) + text('9P2000'))
        c.call(124, struct.pack('<I', 1), error=True)

    def test_image_lock_prevents_duplicate_servers(self):
        with ninep.image_lock(self.image):
            with self.assertRaises(OSError):
                with ninep.image_lock(self.image):
                    pass


if __name__ == '__main__':
    unittest.main()
