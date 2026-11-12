"""SGUB output backend and Foundation command. Run with FND_MM=nommu too."""
import ctypes as C
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'foundation/tests/shell'))
import test_shell


def record(target, address, value):
    return bytes([target]) + address.to_bytes(3, 'big') + bytes([value])


def stream(version=1, repeat=False, target=2):
    if version == 1:
        init = record(2, 0x10, 0x55)
        frames = b'\0\1' + record(target, 0x11, 0x66) + b'\0\0'
        return b'SGUB' + bytes([1, 16, repeat, 0]) + struct.pack('>III', 2, 1, 20 + len(init + frames)) + init + frames
    def write(t, a, v):
        return b'\0' + bytes([t]) + a.to_bytes(4 if version == 3 else 3, 'big') + bytes([v])
    init = write(2, 0x10, 0x55)
    frames = b'\0\1' + write(target, 0x11, 0x66) + b'\0\0'
    return (b'SGUB' + bytes([version, 16, repeat, 0]) +
            struct.pack('>IIIIIIIHH', 2, 0, 1, 40, 40, 40 + len(init), 40 + len(init + frames), 40, 0) + init + frames)


def stream_v4(frames, initial=(), cards=(), pools=(), period=0, repeat=False):
    """SGUB v4 (doc/spec_SGUB_v4.md); ops are complete 7/11/16-byte operations."""
    body = bytearray()
    for slot, id_offset, ident, bulk_start, bulk_length in cards:
        body += bytes([slot, len(ident), 0, 0]) + struct.pack('>III', id_offset, bulk_start, bulk_length)
        body += ident.ljust(16, b'\0')
    directory = 48 + len(body)
    offset = directory + 16 * len(pools)
    for index, (kind, data) in enumerate(pools):
        body += struct.pack('>HBBIII', index, kind, 0, len(data), len(data), offset)
        offset += len(data)
    for _, data in pools:
        body += data
    initial_offset = 48 + len(body)
    body += b''.join(initial)
    frame_offset = 48 + len(body)
    for frame in frames:
        body += struct.pack('>H', len(frame)) + b''.join(frame)
    return (b'SGUB' + bytes([4, 16, repeat, 0]) +
            struct.pack('>IIIIIIIHBBII', len(frames), len(pools), len(initial), directory,
                        initial_offset, frame_offset, 48 + len(body), 48, len(cards), 0, period, 0)
            + bytes(body))


def write4(target, address, value):
    return bytes([0, target]) + address.to_bytes(4, 'big') + bytes([value])


WRITE = C.CFUNCTYPE(C.c_int32, C.c_void_p, C.c_uint8, C.c_uint32, C.POINTER(C.c_uint8), C.c_uint32)


class Output(C.Structure):
    _fields_ = [('context', C.c_void_p), ('targets', C.c_uint32), ('write', WRITE)]


class State(C.Structure):
    _fields_ = [('stream', C.c_void_p), ('size', C.c_size_t), ('frame_cursor', C.c_size_t),
                ('frame_count', C.c_uint32), ('frame_index', C.c_uint32),
                ('frame_period', C.c_uint32), ('version', C.c_uint8), ('repeat', C.c_uint8),
                ('status', C.c_int)]


class HostTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cc = os.environ.get('HOST_CC') or shutil.which('cc') or shutil.which('gcc') or shutil.which('clang')
        if not cc:
            raise unittest.SkipTest('host C compiler required (HOST_CC)')
        cls.temp = tempfile.TemporaryDirectory(prefix='sgub-host-')
        cls.addClassCleanup(cls.temp.cleanup)
        dll = Path(cls.temp.name) / ('sgub.dll' if os.name == 'nt' else 'sgub.so')
        # Windows clang rejects -fPIC and exports nothing from a DLL by default.
        pic = ([f'-Wl,/EXPORT:{name}' for name in ('sgub_init_output', 'sgub_init', 'sgub_step', 'sgub_state')]
               if os.name == 'nt' else ['-fPIC'])
        result = subprocess.run([cc, '-shared', *pic, '-O1', '-I', str(ROOT / 'sdk/cpt32/include'),
                                 str(ROOT / 'sdk/cpt32/src/sgub.c'), '-o', str(dll)],
                                capture_output=True, text=True, errors='replace', timeout=60)
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        cls.lib = C.CDLL(str(dll))
        if os.name == 'nt':
            import _ctypes
            cls.addClassCleanup(_ctypes.FreeLibrary, cls.lib._handle)
        cls.lib.sgub_init_output.argtypes = [C.c_void_p, C.c_size_t, C.POINTER(Output)]
        cls.lib.sgub_init_output.restype = C.c_int
        cls.lib.sgub_step.restype = C.c_int
        cls.lib.sgub_state.restype = C.POINTER(State)

    def init(self, data, fail=False, targets=7):
        self.events = []
        def write(context, target, address, data, size):
            self.events.append((target, address, C.string_at(data, size)))
            return -1 if fail else 0
        self.callback = WRITE(write)
        self.output = Output(None, targets, self.callback)
        self.data = C.create_string_buffer(data)
        return self.lib.sgub_init_output(self.data, len(data), C.byref(self.output))

    def test_versions_order_repeat_and_silence(self):
        for version in (1, 2, 3):
            for repeat in (False, True):
                with self.subTest(version=version, repeat=repeat):
                    self.assertEqual(self.init(stream(version, repeat)), 1)
                    self.assertEqual(self.events, [(2, 0x10, b'\x55')])
                    self.assertEqual(self.lib.sgub_step(), 0)
                    self.assertEqual(self.lib.sgub_step(), 0)
                    self.assertEqual(self.lib.sgub_step(), 0 if repeat else 2)
                    if repeat:
                        self.assertEqual(self.events, [(2, 0x10, b'\x55'), (2, 0x11, b'\x66'), (2, 0x11, b'\x66')])
                    else:
                        self.assertEqual(len(self.events), 34)
                        self.assertTrue(all(t < 2 and d == b'\0' for t, a, d in self.events[2:]))

    def test_reject_before_any_output(self):
        for data in (stream()[:-1], stream() + b'x', stream(3, target=3),
                     stream(2, target=4), b'not SGUB', stream()[:4] + b'\x04' + stream()[5:]):
            with self.subTest(data=data):
                self.assertLess(self.init(data), 0)
                self.assertEqual(self.events, [])
        self.assertEqual(self.init(stream(3, target=3)), -6)

    def test_io_error_aborts_output(self):
        self.assertEqual(self.init(stream(), fail=True), -7)
        self.assertEqual(len(self.events), 1)
        self.assertEqual(self.lib.sgub_step(), -5)

    def test_frame_output_error(self):
        self.assertEqual(self.init(stream()), 1)
        self.callback = WRITE(lambda *args: -1)
        self.output.write = self.callback
        self.assertEqual(self.lib.sgub_step(), -7)
        self.assertEqual(self.lib.sgub_step(), -5)

    def test_duplicate_register_writes_preserved(self):
        data = bytearray(stream())
        data[12:16] = struct.pack('>I', 2)
        data[16:20] = struct.pack('>I', len(data) + 5)
        data[20:25] = record(0, 0x80A, 0)
        data[25:25] = record(0, 0x80A, 0)
        self.assertEqual(self.init(bytes(data)), 1)
        self.assertEqual(self.events, [(0, 0x80A, b'\0'), (0, 0x80A, b'\0')])

    def test_completion_respects_output_targets(self):
        self.assertEqual(self.init(stream()), 1)
        self.output.targets = 4
        self.assertEqual(self.lib.sgub_step(), 0)
        self.assertEqual(self.lib.sgub_step(), 0)
        self.assertEqual(self.lib.sgub_step(), 2)
        self.assertEqual(len(self.events), 2)

    def test_v4_si22_cards_period_and_stop(self):
        hs816 = (5, 0x4F8, b'3HS816', 0, 0)
        s3hs = (0, 0x4003F0, b'3WS88PWN4', 0, 0x400000)
        copy = bytes([2, 3]) + struct.pack('>IHII', 0x20, 0, 0, 3)
        data = stream_v4([[write4(11, 0x0c, 0x0f), write4(8, 0x1e, 1), write4(8, 0x1e, 1)]],
                         initial=[copy], cards=[s3hs, hs816], pools=[(3, b'abc')], period=400000)
        self.assertEqual(self.init(data, targets=0xfff), 1)
        state = self.lib.sgub_state().contents
        self.assertEqual((state.version, state.frame_period), (4, 400000))
        self.assertEqual(self.events, [(3, 0x20, b'abc')])
        self.assertEqual(self.lib.sgub_step(), 0)
        self.assertEqual(self.events[1:], [(11, 0x0c, b''), (8, 0x1e, b''), (8, 0x1e, b'')])
        self.assertEqual(self.lib.sgub_step(), 2)
        # SGU silence, then the 34SI22 stop because the stream used it.
        self.assertEqual(self.events[-1], (11, 0x0c, b'\0'))
        self.assertEqual(len(self.events), 4 + 32 + 1)

    def test_v4_rejects_before_any_output(self):
        hs816 = (2, 0x4F8, b'3HS816', 0, 0)
        valid = stream_v4([[write4(5, 0x1e, 1)]], cards=[hs816])
        self.assertEqual(self.init(valid, targets=0xfff), 1)
        fill = bytes([1, 5]) + struct.pack('>IIB', 0, 1, 0)
        si22_fill = bytes([1, 11]) + struct.pack('>IIB', 0, 1, 0)
        bad = [
            stream_v4([[write4(3, 0, 0)]], cards=[hs816]),         # slot without requirement
            stream_v4([[fill]], cards=[hs816]),                     # no bulk window
            stream_v4([[si22_fill]]),                               # 34SI22 is WRITE only
            stream_v4([[write4(11, 0x40, 0)]]),                     # 34SI22 range
            stream_v4([[write4(12, 0, 0)]]),                        # reserved target
            stream_v4([[]], period=47999),                          # period range
            stream_v4([[]], period=48000001),
            stream_v4([[]], cards=[hs816, hs816]),                  # slots must ascend
            stream_v4([[]], cards=[(8, 0, b'X', 0, 0)]),
            stream_v4([[]], cards=[(0, 0x1FFFFFF, b'XY', 0, 0)]),  # ID outside the aperture
            valid + b'x',
        ]
        for data in bad:
            with self.subTest(data=data):
                self.assertLess(self.init(data, targets=0xfff), 0)
                self.assertEqual(self.events, [])
        # Unsupported targets are rejected by the output mask.
        self.assertEqual(self.init(valid), -6)
        self.assertEqual(self.init(stream_v4([[write4(11, 0, 0)]])), -6)
        self.assertEqual(self.events, [])

    def test_real_furnace_pool_initialization(self):
        for name in ('furnace_minimal_v2.sgub', 'furnace_pcm_v2.sgub'):
            with self.subTest(name=name):
                data = (ROOT / 'tests/fixtures' / name).read_bytes()
                self.assertEqual(self.init(data), 1)
                # Real pooled PCM includes a 1 MiB silence FILL, sent in
                # bounded chunks, and wave/PCM COPY_POOL operations.
                self.assertTrue(any(len(d) >= 256 for t, a, d in self.events))
                for _ in range(struct.unpack_from('>I', data, 8)[0]):
                    self.assertEqual(self.lib.sgub_step(), 0)


class PlayerTests(unittest.TestCase):
    screen = test_shell.ShellTests.screen
    free_kib = test_shell.ShellTests.free_kib

    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        for version in (1, 2, 3):
            volume.put(f'/v{version}.sgub', stream(version))
        volume.put('/repeat.sgub', stream(repeat=True))
        volume.put('/bad.sgub', stream()[:-1])
        volume.put('/card.sgub', stream(3, target=3))
        for name in ('furnace_minimal_v2.sgub', 'furnace_pcm_v2.sgub'):
            volume.put('/' + name, (ROOT / 'tests/fixtures' / name).read_bytes())
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def test_versions_and_device_reopen(self):
        rows = self.screen('sgubplay v1.sgub\\n', 'sgubplay v2.sgub\\n', 'sgubplay v3.sgub\\n',
                           'echo status=$?\\n', at_frames=[30, 240, 360, 480],
                           final=[(0x18000010, 0x55), (0x18000011, 0x66), (0x80020803, 0), (0x80021819, 0)])
        self.assertEqual(rows.count('sgubplay: complete'), 3, rows)
        self.assertIn('status=0', rows)

    def test_reject_and_missing(self):
        rows = self.screen('sgubplay bad.sgub\\n', 'sgubplay card.sgub\\n',
                           'sgubplay missing.sgub\\n', 'sgubplay\\n', step=60,
                           final=[(0x18000010, 0), (0x18000011, 0)])
        self.assertTrue(any('bad.sgub:' in r and '(-12)' in r for r in rows), rows)
        self.assertTrue(any('card.sgub:' in r and '(-13)' in r for r in rows), rows)
        self.assertIn('usage: sgubplay FILE', rows)

    def test_repeat_stop_and_reopen(self):
        rows = self.screen('sgubplay repeat.sgub\\n', 'q', 'sgubplay v2.sgub\\n', 'echo status=$?\\n',
                           at_frames=[30, 260, 300, 400], final=[(0x80020803, 0), (0x80021819, 0)])
        self.assertIn('sgubplay: stopped', rows)
        self.assertIn('sgubplay: complete', rows)
        self.assertIn('status=0', rows)

    def test_escape_stop(self):
        rows = self.screen('sgubplay repeat.sgub\\n', 'echo status=$?\\n',
                           at_frames=[30, 400], keys=['260:41:1', '261:41:0'],
                           final=[(0x80020803, 0), (0x80021819, 0)])
        self.assertIn('sgubplay: stopped', rows)
        self.assertIn('status=0', rows)

    def test_kill_releases_device(self):
        rows = self.screen('sgubplay repeat.sgub\\n', '\\C', 'sgubplay v2.sgub\\n',
                           at_frames=[30, 260, 360],
                           final=[(0x80020803, 0), (0x80021819, 0)])
        self.assertTrue(any('^C' in r for r in rows), rows)
        self.assertIn('sgubplay: complete', rows)

    def test_real_furnace_playback(self):
        for name in ('furnace_minimal_v2.sgub', 'furnace_pcm_v2.sgub'):
            with self.subTest(name=name):
                rows = self.screen(f'sgubplay {name}\\n', 'q', at_frames=[30, 350],
                                   final=[(0x80020803, 0), (0x80021819, 0)])
                self.assertTrue(any(r.startswith('sgubplay: playing ') for r in rows), rows)
                self.assertTrue(any(r in ('sgubplay: complete', 'sgubplay: stopped') for r in rows), rows)


if __name__ == '__main__':
    unittest.main()
