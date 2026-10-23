"""Real shell -> fs -> transferred kernel CHANNEL -> SGC, in VM and slots.

Run python -m unittest discover -s foundation/tests/sgc -v, also FND_MM=slot.
Uses the shell harness's boot builder and workspace-local TEMP on Windows.
"""
import os
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
import test_shell


class SgcTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        guest = cls.work / 'sgccheck.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), str(HERE / 'guest.c'),
                   '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.sgc',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=120)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/sgccheck.wax', guest.read_bytes(), mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def run_guest(self, *batches, cpu='cached', pixels=(), final=(), frames=660, running=False):
        run = self.work / self.id().split('.')[-1]
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', str(frames), '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', cpu)]
        for i, batch in enumerate(batches):
            command += ['--type', f'{30 + i * 40}={batch}']
        if running: command += ['--allow-running']
        for x, y, rgb in pixels: command += ['--expect-pixel', f'{x}:{y}={rgb:#x}']
        for address, value in final: command += ['--expect-final-mem-u8', f'{address:#x}={value}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_operations(self):
        text = self.run_guest('sgccheck\\n', 'echo status=$?\\n')
        self.assertIn('sgc passed', text)
        self.assertIn('status=0', text)

    def test_exit_with_open_stream_resets_sgc_and_releases_lease(self):
        text = self.run_guest('sgccheck x\\n', 'echo left=$?\\n', 'sgccheck\\n', 'echo status=$?\\n')
        self.assertIn('left=0', text)
        self.assertIn('sgc passed', text)
        self.assertIn('status=0', text)

    def test_rect_reaches_the_display(self):
        self.run_guest('sgccheck l\\n', frames=200, running=True,
                       pixels=[(25, 32, 0xFFFFFF), (19, 32, 0), (30, 32, 0), (25, 36, 0), (0, 0, 0)])
