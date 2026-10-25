"""BusyBox（外部ソース）を pkcl と Newlib で組み、Foundation 上で applet を動かす。

BusyBox は GPL-2.0 でリポジトリに含めないので、ソースの場所を環境変数で渡したときだけ走る。

    SRC32_BUSYBOX_SOURCE=/path/to/busybox-1.36.1 SRC32_NEWLIB_SOURCE=/path/to/newlib-4.4.0.20231231 \\
        python -m unittest discover -s foundation/tests/pkcl -k busybox -v

CLANG と SRC32_TESTBENCH で実行ファイルを指定できます。BusyBox のビルドは数分かかります。
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

BUSYBOX = os.environ.get('SRC32_BUSYBOX_SOURCE')
NEWLIB = os.environ.get('SRC32_NEWLIB_SOURCE')


@unittest.skipUnless(BUSYBOX and NEWLIB, 'SRC32_BUSYBOX_SOURCE と SRC32_NEWLIB_SOURCE が必要')
class BusyboxTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # 1 MiB の busybox.wax は読み込みだけでも時間がかかるので、既定の 8 秒では足りない。
        test_shell.STOP_TICKS = 2000
        test_shell.ShellTests.setUpClass.__func__(cls)
        out = cls.work / 'busybox'
        command = [sys.executable, str(ROOT / 'tools/pkcl/busybox.py'), '--busybox-source', BUSYBOX,
                   '--newlib-source', NEWLIB, '--fnd-wal', str(cls.work / 'root/lib/fnd.1.wal'), '--out', str(out)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        result = subprocess.run(command, capture_output=True, text=True, timeout=3000)
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        wax = (out / 'busybox.wax').read_bytes()
        # 起動上限は 3 MiB だが、既存構成の 1 MiB 未満をサイズ回帰の基準にする。
        assert len(wax) < 0x100000, f'busybox.wax is {len(wax)} bytes'
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/busybox.wax', wax, mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def run_commands(self, commands, frames):
        run = self.work / 'run'
        run.mkdir(exist_ok=True)
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', str(frames), '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached')]
        for index, text in enumerate(commands):
            command += ['--type', f'{30 + index * 70}={text}\\n']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=1800)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_applets(self):
        screen = self.run_commands([
            'busybox echo hello',
            'busybox printf %d-%s 42 abc',
            'busybox printf %5.2f 3.14159',
            'busybox mkdir /tmp/d',
            'busybox basename /a/b.txt .txt',
            'busybox dirname /a/b/c',
            'busybox test -d /tmp/d',
            'busybox cat /nonexistent',
            'busybox ls /tmp',
        ], 2000)
        for expected in ('hello', '42-abc', ' 3.14', '/a/b', "cat: can't open '/nonexistent': No such file or directory"):
            self.assertIn(expected, screen)

    def test_help_lists_applets(self):
        screen = self.run_commands(['busybox --help'], 1800)
        self.assertIn('BusyBox v', screen)
        self.assertIn('basename, cat, cp', screen)


if __name__ == '__main__':
    unittest.main()
