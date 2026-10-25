"""WAV parsing/conversion and real Foundation -> FIFO -> stereo capture.

Run with FND_MM=nommu too. --dump-audio advances the SGU at 48 kHz; without
it the historical testbench never drains a DAC FIFO.
"""
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
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / 'foundation/tests/shell'))
import test_shell


def chunk(tag, data):
    return tag + struct.pack('<I', len(data)) + data + bytes(len(data) & 1)


def wave(data, channels=1, bits=16, rate=48000, extra=b'', tag=1):
    align = channels * bits // 8
    body = b'WAVE' + extra + chunk(b'fmt ', struct.pack('<HHIIHH', tag, channels, rate,
                                                     rate * align, align, bits)) + chunk(b'data', data)
    return b'RIFF' + struct.pack('<I', len(body)) + body


class HostTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cc = os.environ.get('HOST_CC') or shutil.which('clang') or shutil.which('gcc')
        if not cc:
            candidates = sorted(Path('C:/Program Files/AMD/ROCm').glob('*/bin/clang.exe'), reverse=True)
            cc = str(candidates[0]) if candidates else None
        if not cc:
            raise unittest.SkipTest('host C compiler required')
        cls.temp = tempfile.TemporaryDirectory(prefix='wav-host-')
        cls.addClassCleanup(cls.temp.cleanup)
        dll = Path(cls.temp.name) / ('wav.dll' if os.name == 'nt' else 'wav.so')
        cmd = [cc, '-shared', '-O2', '-Wall', '-Wextra', '-I', str(ROOT / 'foundation/include'),
               str(HERE / 'host.c'), '-o', str(dll)]
        if os.name == 'nt':
            cmd += ['-Xlinker', '/export:wav_test_parse', '-Xlinker', '/export:wav_test_decode']
        else:
            cmd += ['-fPIC']
        subprocess.run(cmd, check=True, capture_output=True, text=True)
        cls.lib = C.CDLL(str(dll))
        if os.name == 'nt':
            import _ctypes
            cls.addClassCleanup(_ctypes.FreeLibrary, cls.lib._handle)
        cls.lib.wav_test_parse.argtypes = [C.c_void_p, C.c_uint32]
        cls.lib.wav_test_decode.argtypes = [C.c_void_p, C.c_uint32, C.c_void_p, C.c_uint32, C.c_uint32]

    def decode(self, data, block=4096):
        buf = C.create_string_buffer(200000)
        n = self.lib.wav_test_decode(data, len(data), buf, len(buf), block)
        self.assertGreaterEqual(n, 0)
        return buf.raw[:n]

    def test_pcm_sign_endian_and_stereo(self):
        self.assertEqual(self.decode(wave(bytes([0, 128, 255]), bits=8)),
                         bytes.fromhex('8000 0000 7f00 0000'))
        data = struct.pack('<hhhh', -32768, 32767, -1, 1)
        self.assertEqual(self.decode(wave(data, channels=2), block=4),
                         struct.pack('>hhhhhh', -32768, 32767, -1, 1, 0, 0))

    def test_rates_and_buffer_boundaries(self):
        source = [i % 32768 for i in range(5001)]
        data = struct.pack('<' + 'h' * len(source), *source)
        for channels in (1, 2):
            interleaved = [v for x in source for v in ([x] if channels == 1 else [x, -x])]
            data = struct.pack('<' + 'h' * len(interleaved), *interleaved)
            for rate in (8000, 22050, 44100, 48000, 96000, 192000):
                with self.subTest(channels=channels, rate=rate):
                    n = (len(source) * 48000 + rate - 1) // rate
                    expected = []
                    for i in range(n):
                        x = source[i * rate // 48000]
                        expected += [x] if channels == 1 else [x, -x]
                    expected += [0] * channels
                    decoded = self.decode(wave(data, channels=channels, rate=rate), block=28)
                    self.assertEqual(decoded, struct.pack('>' + 'h' * len(expected), *expected))

    def test_chunks_and_rejection(self):
        valid = wave(b'\0\0', extra=chunk(b'JUNK', b'odd'))
        self.assertEqual(self.lib.wav_test_parse(valid, len(valid)), 0)
        bad = [valid[:n] for n in range(len(valid))]
        bad += [wave(b''), wave(b'x'), wave(b'\0\0', tag=3),
                wave(b'\0\0', channels=3), wave(b'\0\0', bits=24),
                wave(b'\0\0', rate=0), wave(b'\0\0', extra=chunk(b'data', b'\0\0'))]
        overflow = bytearray(valid)
        overflow[16:20] = b'\xff' * 4
        bad.append(bytes(overflow))
        bad.append(valid + b'x')
        for data in bad:
            self.assertLess(self.lib.wav_test_parse(data, len(data)), 0, data)


class PlayerTests(unittest.TestCase):
    screen = test_shell.ShellTests.screen
    free_kib = test_shell.ShellTests.free_kib

    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        cls.addClassCleanup(cls.temp.cleanup)
        vol = test_shell.fs2.Volume(bytearray(cls.disk))
        # Distinct constant levels make exact routing and loss visible.
        vol.put('/stereo.wav', wave(struct.pack('<hh', 8192, -4096) * 12000, channels=2))
        vol.put('/mono.wav', wave(bytes([192]) * 4000, bits=8, rate=22050))
        vol.put('/cd.wav', wave(struct.pack('<hh', 8192, -4096) * 11025, channels=2, rate=44100))
        vol.put('/stereo8.wav', wave(bytes([160, 112]) * 5512, channels=2, bits=8, rate=22050))
        vol.put('/long.wav', wave(struct.pack('<hh', 8192, -4096) * 144000, channels=2))
        vol.put('/bad.wav', wave(b'\0\0')[:-1])
        vol.put('/float.wav', wave(b'\0\0', tag=3))
        out = cls.work / 'dacdev.wax'
        command = [sys.executable, ROOT / 'tools/llvm-src32/driver.py', HERE / 'devices.c',
                   '-I', ROOT / 'foundation/include', '-I', ROOT / 'sdk/cpt32/include',
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.wav.devices',
                   '--import-wal', cls.work / 'root/lib/fnd.1.wal',
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', out]
        subprocess.run(list(map(str, command)), check=True, capture_output=True, text=True, timeout=120)
        vol.put('/bin/dacdev.wax', out.read_bytes(), mode=0x3D)
        cls.disk = bytes(vol.img)

    def audio(self):
        data = (self.last_run / 'audio.pcm').read_bytes()
        return list(struct.iter_unpack('<hh', data))

    def test_stereo_completion_and_mono_reopen(self):
        rows = self.screen('wavplay stereo.wav\\n', 'wavplay mono.wav\\n', 'echo status=$?\\n',
                           at_frames=[30, 310, 440], audio=True,
                           final=[(0x80020803, 0), (0x80020819, 0), (0x80020823, 0), (0x80020839, 0)])
        self.assertEqual(rows.count('wavplay: complete'), 2, rows)
        self.assertIn('status=0', rows)
        pairs = self.audio()
        self.assertTrue(any(l > 0 and r < 0 and l == -2 * r for l, r in pairs))
        self.assertTrue(any(l > 0 and l == r for l, r in pairs))
        self.assertEqual(sum(l > 0 and r < 0 for l, r in pairs), 12000)
        self.assertEqual(sum(l > 0 and l == r for l, r in pairs), (4000 * 48000 + 22049) // 22050)
        self.assertEqual(pairs[-1], (0, 0))

    def test_reject_and_legacy(self):
        rows = self.screen('wavplay bad.wav\\n', 'wavplay float.wav\\n', 'wavplay absent.wav\\n',
                           'wavplay\\n', step=80)
        self.assertTrue(any('bad.wav:' in r and '(-12)' in r for r in rows), rows)
        self.assertTrue(any('float.wav:' in r and '(-13)' in r for r in rows), rows)
        self.assertIn('usage: wavplay FILE', rows)
        rows = self.screen('wavplay stereo.wav\\n', config='sgu:\n  version: "2"\n', audio=True)
        self.assertTrue(any('stereo.wav:' in r and '(-13)' in r for r in rows), rows)

    def test_stereo_resampling(self):
        rows = self.screen('wavplay cd.wav\\n', 'wavplay stereo8.wav\\n',
                           at_frames=[30, 310], audio=True)
        self.assertEqual(rows.count('wavplay: complete'), 2, rows)
        pairs = self.audio()
        self.assertEqual(sum(l > 0 and r < 0 for l, r in pairs),
                         12000 + (5512 * 48000 + 22049) // 22050)
        self.assertTrue(all(l == -2 * r for l, r in pairs))

    def test_stop_and_kill_reopen(self):
        for stop in ('q', '\\C', '\\e'):
            with self.subTest(stop=stop):
                rows = self.screen('wavplay long.wav\\n', stop, 'wavplay stereo.wav\\n',
                                   at_frames=[30, 280, 350], audio=True,
                                   final=[(0x80020803, 0), (0x80020823, 0)])
                self.assertIn('wavplay: complete', rows)
                if stop != '\\C':
                    self.assertIn('wavplay: stopped', rows)

    def test_dac_api_backpressure(self):
        rows = self.screen('dacdev\\n', audio=True)
        self.assertIn('DAC API passed', rows)


if __name__ == '__main__':
    unittest.main()
