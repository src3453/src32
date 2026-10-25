"""Real DMAC/copy boundary checks. Requires Clang and src32_testbench."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


class CopyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.clang = os.environ.get('CLANG') or shutil.which('clang')
        candidate = Path('C:/Program Files/AMD/ROCm/6.4/bin/clang.exe')
        if not cls.clang and candidate.is_file():
            cls.clang = str(candidate)
        suffix = '.exe' if os.name == 'nt' else ''
        cls.harness = Path(os.environ.get('SRC32_TESTBENCH', str(ROOT / 'target/release' /
                           ('src32_testbench' + suffix))))
        if not os.environ.get('SRC32_TESTBENCH') and not cls.harness.is_file():
            cls.harness = ROOT / 'target/debug' / ('src32_testbench' + suffix)
        if not cls.clang or not cls.harness.is_file():
            raise RuntimeError('Install Clang and build src32_testbench (or set CLANG/SRC32_TESTBENCH)')

    def test_copy_boundaries_and_channels(self):
        work = ROOT / 'target/foundation-copy-check'
        work.mkdir(parents=True, exist_ok=True)
        temp = work / 'tmp'
        temp.mkdir(exist_ok=True)
        env = dict(os.environ, TEMP=str(temp), TMP=str(temp), TMPDIR=str(temp))
        (work / 'config.yaml').write_text('memory:\n  ram_mib: 16\n  vram_mib: 4\n')
        kernel = ('sched', 'proc', 'mm', 'trap', 'ipc', 'fb', 'sgu', 'sgc', 'pec', 'blk', 'wax')
        # Also force a one-byte threshold to exercise DMA head/tail handling
        # for lengths shorter than the alignment prefix. It is not a default.
        for threshold in (256, 1):
            image = work / f'copy-{threshold}.wax'
            command = [sys.executable, ROOT / 'tools/llvm-src32/driver.py', '--clang', self.clang,
                       HERE / 'copy_check.c', *(ROOT / 'foundation/kernel' / (n + '.c') for n in kernel),
                       '-I', ROOT / 'sdk/cpt32/include', '-I', ROOT / 'foundation/include',
                       '-I', ROOT / 'foundation/kernel', '--origin', '0x10000', '--stack-top', '0x100000',
                       '-O1', '-D', f'FND_COPY_DMA_MIN={threshold}', '-o', image]
            result = subprocess.run(list(map(str, command)), cwd=work, env=env, capture_output=True,
                                    text=True, timeout=300)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            for cpu in ('cached', 'pipeline'):
                with self.subTest(threshold=threshold, cpu=cpu):
                    result = subprocess.run([str(self.harness), str(image), '42', '--cpu', cpu,
                                             '--frames', '600'], cwd=work, env=env,
                                            capture_output=True, text=True, timeout=180)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
