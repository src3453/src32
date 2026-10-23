"""Host tests execute the same C interpreter included by the WAX command."""
import ctypes
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

CLANG = os.environ.get('CC') or shutil.which('clang') or 'C:/Program Files/AMD/ROCm/6.4/bin/clang.exe'

class CoreTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='chip8-core-')
        cls.addClassCleanup(cls.temp.cleanup)
        lib = Path(cls.temp.name) / ('chip8.dll' if os.name == 'nt' else 'chip8.so')
        names = ['test_arithmetic', 'test_memory_control', 'test_draw_timing_keys']
        command = [CLANG, '-shared', '-O2', '-Wall', '-Wextra', str(Path(__file__).with_name('core.c')), '-o', str(lib)]
        if os.name == 'nt':
            for name in names:
                command += ['-Xlinker', '/export:' + name]
        else:
            command += ['-fPIC']
        subprocess.run(command, check=True, capture_output=True, text=True)
        cls.lib = ctypes.CDLL(str(lib))
        if os.name == 'nt':
            import _ctypes
            cls.addClassCleanup(_ctypes.FreeLibrary, cls.lib._handle)

    def test_arithmetic(self):
        self.assertEqual(self.lib.test_arithmetic(), 0, 'failing core.c line')

    def test_memory_control(self):
        self.assertEqual(self.lib.test_memory_control(), 0, 'failing core.c line')

    def test_draw_timing_keys(self):
        self.assertEqual(self.lib.test_draw_timing_keys(), 0, 'failing core.c line')
