"""kernel_panic draws its diagnosis on the PCG (Issue #130).

Run: python3 -m unittest discover -s foundation/tests/panic -v
Needs src32_testbench (SRC32_TESTBENCH or target/release|debug) and Clang
(CLANG, as for the other guest tests).
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
KERNEL = ('sched.c', 'proc.c', 'mm.c', 'trap.c', 'ipc.c', 'fb.c', 'sgu.c', 'sgc.c', 'pec.c', 'blk.c', 'wax.c')
BUS_ERROR_HALT = 0x202  # 0x200 | exception code 2


def find_testbench():
    if os.environ.get('SRC32_TESTBENCH'):
        return Path(os.environ['SRC32_TESTBENCH'])
    for profile in ('release', 'debug'):
        for name in ('src32_testbench', 'src32_testbench.exe'):
            path = ROOT / 'target' / profile / name
            if path.exists():
                return path
    return None


class PanicScreenTests(unittest.TestCase):
    def test_bus_error_in_kernel_shows_diagnosis(self):
        harness = find_testbench()
        if harness is None:
            self.skipTest('src32_testbench not built')
        with tempfile.TemporaryDirectory() as work:
            image = Path(work) / 'panic.wax'
            command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'),
                       str(ROOT / 'foundation/tests/panic/main.c'),
                       *(str(ROOT / 'foundation/kernel' / name) for name in KERNEL),
                       '-I', str(ROOT / 'sdk/cpt32/include'), '-I', str(ROOT / 'foundation/include'),
                       '--origin', '0x10000', '--stack-top', '0x100000',
                       '-D', 'FND_MM_NOMMU_ONLY', '-O1', '-o', str(image)]
            if os.environ.get('CLANG'):
                command += ['--clang', os.environ['CLANG']]
            subprocess.run(command, check=True, capture_output=True, text=True, timeout=300)
            result = subprocess.run(
                [str(harness), str(image), str(BUS_ERROR_HALT), '--frames', '30', '--dump-pcg'],
                cwd=work, capture_output=True, text=True, encoding='utf-8',
                errors='replace', timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('Foundation: kernel panic', result.stdout)
        self.assertIn('cause=80020000 (bus error)', result.stdout)
        self.assertIn('last saved context:', result.stdout)


if __name__ == '__main__':
    unittest.main()
