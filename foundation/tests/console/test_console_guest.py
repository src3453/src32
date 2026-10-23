"""ST_WRITE over real libfnd/IPC with PCG text and attribute checks.

Run also with FND_MM=slot. Uses the shell boot builder and testbench.
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


class ConsoleGuestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        guest = cls.work / 'concheck.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), str(HERE / 'guest.c'),
                   '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.console',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        result = subprocess.run(command, capture_output=True, text=True, timeout=120)
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/concheck.wax', guest.read_bytes(), mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def test_ansi_via_ipc(self):
        run = self.work / 'ansi-run'
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', '660', '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached'),
                   '--type', '30=concheck\\n', '--type', '200=echo status=$?\\n']
        for address, value in [(0x10001000, 32), (0x10002000, 2),
                               (0x10001001, 2), (0x10002001, 53),
                               (0x10001002, 63), (0x10002002, 0),
                               (0x8000F007, 1)]:
            command += ['--expect-final-mem-u8', f'{address:#x}={value}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('CRD', result.stdout)
        self.assertIn('console guest passed', result.stdout)
        self.assertIn('status=0', result.stdout)


if __name__ == '__main__':
    unittest.main()
