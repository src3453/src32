"""Shell -> /bin/bmpview -> /dev/fb0: pixels of 8-bit and 24-bit BMPs reach the screen.

Build cargo build --release --bin src32_testbench first; run again with FND_MM=nommu.
"""
from pathlib import Path
import subprocess
import sys
import os
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
sys.path.insert(0, str(ROOT / 'examples/foundation-bmp'))
import test_shell
from make_bmp import make_bmp


def colour(bpp, x, y, w=160, h=120):
    """The sample image's pixel as the screen shows it."""
    if x in (0, w - 1) or y in (0, h - 1):
        return 0xFFFFFF
    r, g, b = x * 255 // (w - 1), y * 255 // (h - 1), 255 - (x + y) * 255 // (w + h - 2)
    if bpp == 8:
        r, g, b = (r >> 6) * 85, (g >> 5) * 255 // 7, (b >> 5) * 255 // 7
    return r << 16 | g << 8 | b


class BmpViewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        cls.addClassCleanup(cls.temp.cleanup)
        vol = test_shell.fs2.Volume(bytearray(cls.disk))
        vol.put('/rgb.bmp', make_bmp(bpp=24), mode=0x24)
        vol.put('/idx.bmp', make_bmp(bpp=8), mode=0x24)
        vol.put('/trunc.bmp', make_bmp(bpp=24)[:1000], mode=0x24)
        vol.put('/text.bmp', b'not a bitmap' * 8, mode=0x24)
        cls.disk = bytes(vol.img)

    def run_guest(self, batches, pixels=(), final=(), frames=660):
        run = self.work / self.id().split('.')[-1]
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text('memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', str(frames), '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached')]
        if pixels or final: command += ['--allow-running']
        for frame, text in batches: command += ['--type', f'{frame}={text}']
        for x, y, rgb in pixels: command += ['--expect-pixel', f'{x}:{y}={rgb:#x}']
        for address, value in final: command += ['--expect-final-mem-u8', f'{address:#x}={value}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def pixels(self, bpp):  # the 160x120 image sits at (80, 60)
        return [(80 + x, 60 + y, colour(bpp, x, y)) for x, y in ((0, 0), (159, 0), (0, 119), (80, 60), (159, 119), (20, 90))] + \
               [(10, 10, 0), (319, 239, 0)]

    def test_24bit_is_centred_in_rgb888(self):
        self.run_guest([(30, 'bmpview /rgb.bmp\\n')], pixels=self.pixels(24))

    def test_8bit_uses_its_own_palette(self):
        self.run_guest([(30, 'bmpview /idx.bmp\\n')], pixels=self.pixels(8))

    def test_key_returns_to_the_shell(self):
        text = self.run_guest([(30, 'bmpview /rgb.bmp\\n'), (400, 'x'), (480, 'echo done=$?\\n')])
        self.assertIn('done=0', text)

    def test_errors(self):
        text = self.run_guest([(30, 'bmpview /text.bmp\\necho text=$?\\nbmpview /trunc.bmp\\necho trunc=$?\\n'
                                   'bmpview /absent.bmp\\necho absent=$?\\nbmpview\\necho usage=$?\\n')])
        for expected in ('text=-12', 'trunc=-12', 'absent=-10'):
            self.assertIn(expected, text)
        self.assertIn('usage:', text)


if __name__ == '__main__':
    unittest.main()
