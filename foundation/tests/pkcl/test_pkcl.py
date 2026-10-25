"""pkcl の POSIX I/O を実際の Foundation（fs・console サービス）で確かめる。

foundation/boot/build.py で起動イメージを作り、pkcl を静的にリンクしたゲストを
ディスクの /bin に置いて、シェルから実行する。console のゲストテストと同じ構成。

    python -m unittest discover -s foundation/tests/pkcl -v

CLANG と SRC32_TESTBENCH で実行ファイルを指定できます。
"""
import os
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
import test_shell  # noqa: E402

PKCL = ROOT / 'foundation/lib/pkcl'
INCLUDES = ['-I', str(ROOT / 'foundation/include/pkcl-nolibc'), '-I', str(ROOT / 'foundation/include'),
            '-I', str(ROOT / 'sdk/cpt32/include')]


class PkclGuestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        guest = cls.work / 'pkclcheck.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), str(HERE / 'guest.c'),
                   *map(str, sorted(PKCL.glob('*.c'))), *INCLUDES,
                   '--entry', 'pkcl_start', '-O1', '--wax3', '--page-align', '--image-id', 'test.pkcl',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/pkclcheck.wax', guest.read_bytes(), mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def test_posix_io(self):
        run = self.work / 'run'
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', '660', '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached'),
                   '--type', '30=pkclcheck\\n', '--type', '230=echo status=$?\\n']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('pkcl: hello', result.stdout)
        self.assertIn('pkcl guest passed', result.stdout)
        self.assertIn('status=0', result.stdout)


NEWLIB = os.environ.get('SRC32_NEWLIB_SOURCE')


@unittest.skipUnless(NEWLIB, 'SRC32_NEWLIB_SOURCE が必要')
class PkclPosixTests(unittest.TestCase):
    """fork・waitpid・pipe・setjmp。VM の構成（既定）で走る。"""

    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        guest = cls.work / 'posixcheck.wax'
        sources = [HERE / 'guest_posix.c', *sorted(PKCL.glob('*.c')), *sorted((PKCL / 'posix').glob('*.c')),
                   PKCL / 'newlib/syscalls.c']
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), *map(str, sources),
                   '-I', str(ROOT / 'foundation/include/pkcl-posix'), '-I', str(ROOT / 'foundation/include'),
                   '-I', str(ROOT / 'sdk/cpt32/include'), '-DPKCL_LIBC',
                   '--newlib-source', NEWLIB, '--newlib-profile', 'posix', '--newlib-cache', str(cls.work / 'newlib'),
                   '--entry', 'pkcl_start', '-O1', '--wax3', '--page-align', '--image-id', 'test.posix',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '--user-stack-size', '0x10000',
                   '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        result = subprocess.run(command, capture_output=True, text=True, timeout=1800)
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/posixcheck.wax', guest.read_bytes(), mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def test_posix_process_and_pipe(self):
        run = self.work / 'run'
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', '1500', '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached'),
                   '--type', '30=posixcheck\\n', '--type', '330=echo status=$?\\n']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=900)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('pkcl posix passed', result.stdout)
        self.assertIn('status=0', result.stdout)


if __name__ == '__main__':
    unittest.main()
