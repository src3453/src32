"""Disk service cache: data, LRU retention and write/media/error invalidation.

python -m unittest discover -s foundation/tests/disk_cache -v
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class DiskCacheTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cc = os.environ.get('CC') or shutil.which('cc') or shutil.which('gcc') or shutil.which('clang')
        if not cc:
            raise unittest.SkipTest('no host C compiler (set CC)')
        cls.temp = tempfile.TemporaryDirectory(prefix='disk-cache-')
        cls.exe = Path(cls.temp.name) / ('cache.exe' if os.name == 'nt' else 'cache')
        subprocess.run([cc, '-std=c11', '-O1', '-Wall', '-Wextra', '-Werror',
                        '-I', str(ROOT / 'foundation/include'),
                        '-I', str(ROOT / 'foundation/services/disk'),
                        str(Path(__file__).with_name('host.c')),
                        str(ROOT / 'foundation/services/disk/cache.c'), '-o', str(cls.exe)], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def test_cache_contract(self):
        for scenario in ('working_set', 'replacement', 'writes', 'identify', 'failed_prefetch',
                         'failed_read', 'boundaries'):
            with self.subTest(scenario=scenario):
                result = subprocess.run([str(self.exe), scenario], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(result.stdout.strip(), 'PASS')


if __name__ == '__main__':
    unittest.main()
