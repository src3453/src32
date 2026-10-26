"""FPGA console MMIO bounds, startup resources and supported display modes."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]


def build_fixture(out):
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, str(ROOT / 'foundation/boot'))
    import tang_font
    tang_font.write_header(out / 'con_tang_font.h')
    cc = os.environ.get('CC') or shutil.which('cc') or shutil.which('clang')
    if not cc and Path('C:/msys64/ucrt64/bin/gcc.exe').is_file():
        cc = 'C:/msys64/ucrt64/bin/gcc.exe'
    if not cc:
        raise RuntimeError('host C compiler required')
    env = os.environ.copy()
    env['PATH'] = str(Path(cc).parent) + os.pathsep + env.get('PATH', '')
    exe = out / ('tang-console.exe' if os.name == 'nt' else 'tang-console')
    subprocess.run([cc, '-std=c11', '-O2', '-I', str(out),
                    '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                    str(Path(__file__).with_name('tang.c')), '-o', str(exe)], check=True, env=env)
    subprocess.run([str(exe), str(out)], check=True, env=env)
    return out


class TangConsoleTests(unittest.TestCase):
    def test_backend(self):
        build_fixture(ROOT / 'target/tang-console-host')

    def test_grants(self):
        spec = importlib.util.spec_from_file_location('console_boot', ROOT / 'foundation/boot/build.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        grants = module.console_grants(tang=True)
        self.assertIn((0x10020000, 0x10022000, 3), grants)
        self.assertIn((0x10012c00, 0x10012cc0, 3), grants)
        self.assertFalse(any(a <= 0x80040034 < b or a <= 0x10200000 < b for a, b, _ in grants))
        self.assertEqual(module.console_grants(uart_only=True), [(0x80040000, 0x80040010, 3)])


if __name__ == '__main__':
    unittest.main()
