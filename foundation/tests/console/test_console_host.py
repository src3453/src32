"""Actual console driver, with in-memory MMIO and a host C compiler."""
import os
from pathlib import Path
import shutil
import sys
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class ConsoleHostTests(unittest.TestCase):
    def test_driver(self):
        cc = os.environ.get('CC') or shutil.which('cc') or shutil.which('clang')
        if not cc and os.name == 'nt':
            candidate = Path('C:/msys64/ucrt64/bin/gcc.exe')
            if candidate.is_file():
                cc = str(candidate)
        if not cc:
            self.skipTest('host C compiler required')
        env = os.environ.copy()
        env['PATH'] = str(Path(cc).resolve().parent) + os.pathsep + env.get('PATH', '')
        with tempfile.TemporaryDirectory(prefix='foundation-console-') as tmp:
            exe = Path(tmp) / ('console.exe' if os.name == 'nt' else 'console')
            sys.path.insert(0, str(ROOT / 'foundation/boot'))
            import wide_font
            font_dir = wide_font.write_header(Path(tmp) / 'font' / 'con_wide_font.h').parent
            build = subprocess.run([cc, '-std=c11', '-Wall', '-Wextra', '-Werror', '-O2',
                            '-I', str(ROOT / 'foundation/include'), '-I', str(font_dir),
                            '-I', str(ROOT / 'sdk/cpt32/include'),
                            str(Path(__file__).with_name('host.c')), '-o', str(exe)],
                            env=env, capture_output=True, text=True)
            self.assertEqual(build.returncode, 0, build.stdout + build.stderr)
            result = subprocess.run([str(exe)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('console host passed', result.stdout)


if __name__ == '__main__':
    unittest.main()
