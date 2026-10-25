"""Opt-in real-WAD checks after tools/doomgeneric/run.py --build-only.

Set SRC32_DOOM_BUILD to that build directory. No WAD or engine is downloaded.
FOUNDATION_TEST_CPU selects the CPU (jit needs --features jit).
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
BUILD = os.environ.get('SRC32_DOOM_BUILD')
sys.path.insert(0, str(ROOT / 'tools/fs2'))
import fs2  # noqa: E402


@unittest.skipUnless(BUILD, 'set SRC32_DOOM_BUILD to a built private DOOM disk')
class DoomGuestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.build = Path(BUILD).resolve()
        cls.harness = Path(os.environ.get('SRC32_TESTBENCH', ROOT / 'target/release' /
                          ('src32_testbench.exe' if os.name == 'nt' else 'src32_testbench')))
        cls.temp = tempfile.TemporaryDirectory(prefix='doom-guest-')
        cls.addClassCleanup(cls.temp.cleanup)
        cls.work = Path(cls.temp.name)
        guest = cls.work / 'limits.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'),
                   str(Path(__file__).with_name('limits.c')), '-I', str(ROOT / 'foundation/include'),
                   '-I', str(ROOT / 'sdk/cpt32/include'), '--entry', 'main', '-O1', '--wax3',
                   '--pie', '--page-align', '--heap-hint', '0x400000', '--import-wal',
                   str(cls.build / 'boot/root/lib/fnd.1.wal'), '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        subprocess.run(command, check=True, capture_output=True, text=True)
        volume = fs2.Volume(bytearray((cls.build / 'boot/foundation.img').read_bytes()))
        volume.put('/bin/doomlimits.wax', guest.read_bytes(), mode=0x3D)
        volume.put('/bin/oversize.wax', bytes(0x300001), mode=0x3D)
        if fs2.check_image(volume.img):
            raise ValueError('invalid private FS2 disk')
        cls.disk = bytes(volume.img)

    def run_guest(self, types, frames, keys=()):
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text('memory:\n  ram_mib: 64\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.build / 'boot/foundation.wax'), '42', '--cpu',
                   os.environ.get('FOUNDATION_TEST_CPU', 'cached-relaxed'), '--frames', str(frames),
                   '--allow-running', '--skip-render-checks', '--dump-pcg',
                   '--dump-framebuffer', str(run / 'screen.ppm')]
        for frame, text in types:
            command += ['--type', f'{frame}={text}']
        for key in keys:
            command += ['--key', key]
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=1800)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout, (run / 'screen.ppm').read_bytes()

    def test_spawn_limits(self):
        text, _ = self.run_guest([(30, 'doomlimits\\noversize\\necho oversized=$?\\n')], 1800)
        self.assertIn('doom spawn limits passed', text)
        self.assertIn('oversized=-6', text)

    def test_real_wad_frames_and_return(self):
        text, _ = self.run_guest([(30, 'doom -iwad /doom.wad -fnd-frames 4\\n'),
                                  (3000, 'echo doom-status=$?\\n')], 3200)
        self.assertIn('doom: frame limit reached', text)
        self.assertIn('doom-status=0', text)
        self.assertNotIn('kernel panic', text.lower())

    def test_real_wad_gameplay_pixels(self):
        text, ppm = self.run_guest([(30, 'doom -iwad /doom.wad -warp 1 1\\n')], 2400)
        (self.build / 'gameplay.ppm').write_bytes(ppm)
        header, pixels = ppm.split(b'\n255\n', 1)
        self.assertIn(b'320 240', header)
        self.assertEqual(pixels[:320 * 20 * 3], bytes(320 * 20 * 3))
        self.assertEqual(pixels[320 * 220 * 3:], bytes(320 * 20 * 3))
        colors = {pixels[i:i + 3] for i in range(320 * 20 * 3, 320 * 220 * 3, 3)}
        self.assertGreater(len(colors), 32, 'map must contain visible, varied pixels\n' + text)


if __name__ == '__main__':
    unittest.main()
