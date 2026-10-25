"""Host checks for the FPGA console and RAM-disk syscall."""
import os
from pathlib import Path
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[3]


class HeadlessTests(unittest.TestCase):
    def test_no_mmio(self):
        self.host('headless')

    def test_ramdisk_validation(self):
        self.host('ramdisk_host')

    def host(self, name):
        cc = os.environ.get('CC') or shutil.which('cc') or shutil.which('clang')
        if not cc and os.name == 'nt' and Path('C:/msys64/ucrt64/bin/gcc.exe').is_file():
            cc = 'C:/msys64/ucrt64/bin/gcc.exe'
        if not cc:
            self.skipTest('host C compiler required')
        work = ROOT / 'target/headless-host'
        work.mkdir(parents=True, exist_ok=True)
        exe = work / (name + ('.exe' if os.name == 'nt' else ''))
        env = os.environ.copy()
        env['PATH'] = str(Path(cc).parent) + os.pathsep + env.get('PATH', '')
        cmd = [cc, '-std=c11', '-O2', '-I', str(ROOT / 'foundation/include'),
               '-I', str(ROOT / 'sdk/cpt32/include'),
               str(Path(__file__).with_name(f'{name}.c')), '-o', str(exe)]
        subprocess.run(cmd, check=True, env=env)
        subprocess.run([str(exe)], check=True, env=env)


if __name__ == '__main__':
    unittest.main()
