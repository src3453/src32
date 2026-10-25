"""libsgc: the command builder on the host, and the WAL "sgc" in Foundation.

Run python -m unittest discover -s foundation/tests/libsgc -v, also FND_MM=nommu.
The guest tests need src32_testbench and Clang (see tests/shell).
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
import test_shell


class HostTests(unittest.TestCase):
    def test_command_stream(self):
        cc = os.environ.get('HOST_CC') or shutil.which('cc') or shutil.which('gcc') or shutil.which('clang')
        if not cc:
            self.skipTest('host C compiler required')
        with tempfile.TemporaryDirectory(prefix='libsgc-host-') as directory:
            exe = Path(directory) / ('host.exe' if os.name == 'nt' else 'host')
            subprocess.run([cc, '-O1', '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                            str(HERE / 'host.c'), str(ROOT / 'foundation/lib/sgc/sgc.c'), '-o', str(exe)],
                           check=True, capture_output=True, text=True, timeout=120)
            result = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('libsgc host passed', result.stdout)


class GuestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        guest = cls.work / 'libsgccheck.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), str(HERE / 'guest.c'),
                   '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.libsgc',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--import-wal', str(cls.work / 'root/lib/sgc.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=120)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/libsgccheck.wax', guest.read_bytes(), mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def run_guest(self, *batches, cpu='cached', pixels=(), frames=660, running=False):
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
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_wal_calls(self):
        text = self.run_guest('libsgccheck\\n', 'echo status=$?\\n')
        self.assertIn('libsgc passed', text)
        self.assertIn('status=0', text)

    def test_drawing_reaches_the_display(self):
        white, red, green, blue = 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF
        self.run_guest('libsgccheck k\\n', frames=400, running=True, pixels=[
            (125, 122, white), (119, 122, 0), (130, 122, 0), (125, 126, 0),
            (220, 120, red), (60, 110, green), (100, 150, 0), (30, 200, blue),
            (0, 220, white), (2, 220, 0), (3, 220, red), (6, 220, white), (297, 220, red)])


if __name__ == '__main__':
    unittest.main()
