"""Headless checks for the bare-metal C flappy game (examples/flappy-c).

Builds main.c with tools/llvm-src32/driver.py (normal, FLAPPY_PROFILE and
FLAPPY_AUTOPILOT variants), runs them on src32_testbench from a directory
without a BIOS config and checks GP0 pixels. The profile build asserts the
contract the renderer relies on: every draw() fits in one 800,000-cycle frame,
so the VDP never samples a half-drawn screen.

    python -m unittest discover -s examples/flappy-c -v

CLANG and SRC32_TESTBENCH override executable discovery; without them the
tests are skipped (a skip is not a pass).
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SPACE = 44  # HID usage
FRAME_CYCLES = 800_000

SKY = '0x70c5ce'
SAND = '0xded696'
TITLE_YELLOW = '0xfad028'
PANEL_EDGE = '0x7c5834'
GAME_OVER_RED = '0xe24838'
WHITE = '0xffffff'


def find_testbench():
    suffix = '.exe' if os.name == 'nt' else ''
    if os.environ.get('SRC32_TESTBENCH'):
        return Path(os.environ['SRC32_TESTBENCH'])
    for profile in ('release', 'debug'):
        candidate = ROOT / 'target' / profile / ('src32_testbench' + suffix)
        if candidate.is_file():
            return candidate
    return None


class FlappyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.clang = os.environ.get('CLANG') or shutil.which('clang')
        cls.testbench = find_testbench()
        if not cls.clang or not cls.testbench:
            raise unittest.SkipTest('needs Clang (CLANG) and src32_testbench (SRC32_TESTBENCH)')
        cls.temp = tempfile.TemporaryDirectory(prefix='flappy-c-')
        cls.work = Path(cls.temp.name)
        cls.builds = {}
        for name, defines, extra in (
                ('game', [], []),
                ('profile', ['FLAPPY_PROFILE'], ['print.c']),
                ('auto', ['FLAPPY_AUTOPILOT'], [])):
            output = cls.work / (name + '.wax')
            sources = [HERE / 'main.c'] + [
                ROOT / 'sdk/cpt32/src' / s for s in ['dmac.c', 'irqc.c'] + extra]
            command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'),
                       '--clang', cls.clang, '-O2', '-I', str(ROOT / 'sdk/cpt32/include'),
                       '--origin', '0', '--stack-top', '0x00100000',
                       '-o', str(output)] + [str(s) for s in sources]
            for define in defines:
                command += ['-D', define]
            result = subprocess.run(command, capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
            cls.builds[name] = output

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def run_game(self, name, frames, *options):
        # The work directory has no config.yaml, so no BIOS replaces the guest.
        command = [str(self.testbench), str(self.builds[name]), '--allow-running',
                   '--frames', str(frames)] + list(options)
        result = subprocess.run(command, cwd=self.work, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout[-2000:] + result.stderr[-2000:])
        return result

    @staticmethod
    def pixels(*checks):
        options = []
        for (x, y), color in checks:
            options += ['--expect-pixel', f'{x}:{y}={color}']
        return options

    @staticmethod
    def flap(frame):
        return ['--key', f'{frame}:{SPACE}:1', '--key', f'{frame + 2}:{SPACE}:0']

    def test_title_screen(self):
        self.run_game('game', 90, *self.pixels(
            ((5, 5), SKY), ((160, 230), SAND), ((63, 45), TITLE_YELLOW)))

    def test_falls_to_game_over(self):
        # One flap starts the game; the bird then falls to the ground.
        self.run_game('game', 260, *self.flap(60), *self.pixels(
            ((79, 95), PANEL_EDGE), ((100, 60), GAME_OVER_RED), ((160, 230), SAND)))

    def test_autopilot_scores_through_pipes(self):
        # Score "2" centered at scale 4: only 2 sets row 6, column 0.
        self.run_game('auto', 420, *self.flap(60), *self.pixels(((151, 42), WHITE)))

    def test_every_frame_fits_in_one_vsync(self):
        uart = self.work / 'profile-uart.txt'
        self.run_game('profile', 900, *self.flap(60), *self.flap(400),
                      '--dump-uart', str(uart))
        cycles = [int(line) for line in uart.read_text().split() if line.strip()]
        self.assertGreater(len(cycles), 800)
        self.assertLess(max(cycles), FRAME_CYCLES // 2, max(cycles))


if __name__ == '__main__':
    unittest.main()
