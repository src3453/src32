"""Real shell -> fs -> transferred kernel CHANNEL -> VDP, in VM and slots.

Run python -m unittest discover -s foundation/tests/fb -v, also FND_MM=nommu.
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


class FramebufferTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        guest = cls.work / 'fbcheck.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), str(HERE / 'guest.c'),
                   '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.fb',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=120)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/fbcheck.wax', guest.read_bytes(), mode=0x3D)
        demo = cls.work / 'fbdemo.wax'
        command[2] = str(ROOT / 'examples/foundation-fb/main.c')
        command[command.index('--image-id') + 1] = 'example.fb'
        command[command.index('-o') + 1] = str(demo)
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=120)
        volume.put('/bin/fbdemo.wax', demo.read_bytes(), mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def run_guest(self, *batches, cpu='cached'):
        run = self.work / self.id().split('.')[-1]
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', '660', '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', cpu)]
        for i, batch in enumerate(batches):
            command += ['--type', f'{30 + i * 40}={batch}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_operations(self):
        text = self.run_guest('fbcheck\\n', 'echo status=$?\\n')
        self.assertIn('fb0 passed', text)
        self.assertIn('status=0', text)

    def test_process_exit_restores_console_and_releases_lease(self):
        text = self.run_guest('fbcheck leak\\n', 'echo leaked=$?\\n', 'fbcheck\\n', 'echo status=$?\\n')
        self.assertIn('leaked=0', text)
        self.assertIn('fb0 passed', text)
        self.assertIn('status=0', text)

    def test_user_mmio_fault_restores_console_and_releases_lease(self):
        text = self.run_guest('fbcheck fault\\n', 'echo fault=$?\\n', 'fbcheck\\n', 'echo status=$?\\n', cpu='pipeline')
        self.assertRegex(text, r'fault=-\d+')
        self.assertIn('fb0 passed', text)
        self.assertIn('status=0', text)

    def test_demo_draws_and_returns_to_shell(self):
        text = self.run_guest('fbdemo\\n', 'echo demo=$?\\n')
        self.assertIn('demo=0', text)


if __name__ == '__main__':
    unittest.main()
