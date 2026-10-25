"""Adapter checks with external headers: SRC32_DOOMGENERIC_SOURCE=target/doomgeneric."""
import ctypes
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SOURCE = os.environ.get('SRC32_DOOMGENERIC_SOURCE')


@unittest.skipUnless(SOURCE, 'set SRC32_DOOMGENERIC_SOURCE to an external checkout')
class DoomPlatformTests(unittest.TestCase):
    def test_device_adapter(self):
        compiler = os.environ.get('CC') or shutil.which('clang') or 'C:/Program Files/AMD/ROCm/6.4/bin/clang.exe'
        with tempfile.TemporaryDirectory(prefix='doom-host-') as directory:
            lib = Path(directory) / ('doom.dll' if os.name == 'nt' else 'doom.so')
            command = [compiler, '-shared', '-O2', '-DCMAP256',
                       '-I', str(Path(SOURCE).resolve() / 'doomgeneric'),
                       '-I', str(ROOT / 'foundation/include'),
                       str(Path(__file__).with_name('host.c')), '-o', str(lib)]
            command += ['-Xlinker', '/export:test_platform', '-Xlinker', '/export:test_wad'] if os.name == 'nt' else ['-fPIC']
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            dll = ctypes.CDLL(str(lib))
            try:
                self.assertEqual(dll.test_platform(), 0, 'failing host.c line')
                wad = Path(directory) / 'test.wad'
                wad.write_bytes(bytes(range(256)))
                dll.test_wad.argtypes = [ctypes.c_char_p]
                self.assertEqual(dll.test_wad(os.fsencode(wad)), 0, 'failing host.c WAD line')
            finally:
                if os.name == 'nt':
                    import _ctypes
                    _ctypes.FreeLibrary(dll._handle)


if __name__ == '__main__':
    unittest.main()
