"""A Rust command on Foundation (examples/rust-foundation, sdk/rust/fnd).

Builds Foundation (shell and echo only), builds rsdemo with
tools/llvm-src32/cargo_src32.py against the built /lib/fnd.1.wal, rebuilds the
disk with it in /bin (build.py --bin-dir) and runs it in the shell on
src32_testbench, comparing the PCG screen. Skips without a usable Rust
toolchain (Cargo, rust-src, LLVM matching llvmlite).

    python -m unittest discover -s foundation/tests/rust -v

CARGO, CLANG and SRC32_TESTBENCH override executable discovery.
"""
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / 'tools/llvm-src32'))
import cargo_src32  # noqa: E402

STOP_TICKS = 800
FRAMES = 660
BANNER = ['Foundation OS (kernel ABI 1.0)', 'Foundation shell 1.0']


def find_testbench():
    if os.environ.get('SRC32_TESTBENCH'):
        return Path(os.environ['SRC32_TESTBENCH'])
    suffix = '.exe' if os.name == 'nt' else ''
    for profile in ('release', 'debug'):
        candidate = ROOT / 'target' / profile / ('src32_testbench' + suffix)
        if candidate.is_file():
            return candidate
    return None


class RustCommandTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        problem = cargo_src32.toolchain_problem()
        if problem:
            raise unittest.SkipTest(problem)
        cls.harness = find_testbench()
        if cls.harness is None:
            raise unittest.SkipTest('build cargo build --release --bin src32_testbench or set SRC32_TESTBENCH')
        cls.temp = tempfile.TemporaryDirectory(prefix='foundation-rust-')
        cls.work = Path(cls.temp.name)
        clang = os.environ.get('CLANG') or shutil.which('clang')
        clang_option = ['--clang', clang] if clang else []
        out, bins = cls.work / 'out', cls.work / 'bin'
        bins.mkdir()
        build = [sys.executable, str(ROOT / 'foundation/boot/build.py'), str(out), '--stop-ticks', str(STOP_TICKS),
                 '--commands', 'echo', '--libs', '', *clang_option]
        cls.run_tool(build)
        cls.run_tool([sys.executable, str(ROOT / 'tools/llvm-src32/cargo_src32.py'),
                      '--manifest-path', str(ROOT / 'examples/rust-foundation/Cargo.toml'),
                      '--target-dir', str(cls.work / 'cargo'), '--build-std', 'core,alloc',
                      '--foundation-wal', str(out / 'root/lib/fnd.1.wal'), *clang_option,
                      '-o', str(bins / 'rsdemo.wax')])
        # The disk again, now with /bin/rsdemo.wax.
        cls.run_tool([*build, '--bin-dir', str(bins)])
        cls.image = out / 'foundation.wax'
        cls.disk = (out / 'foundation.img').read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    @staticmethod
    def run_tool(command):
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=900)
        if result.returncode != 0:
            raise RuntimeError(' '.join(command) + '\n' + result.stdout + result.stderr)

    def screen(self, *batches):
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n',
            encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--cpu', 'cached', '--frames', str(FRAMES), '--dump-pcg']
        for i, text in enumerate(batches):
            command += ['--type', f'{30 + 6 * i}={text}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        while rows and not rows[-1]:
            rows.pop()
        return rows

    def test_rsdemo_args_alloc_and_files(self):
        self.assertEqual(self.screen('rsdemo pear apple fig\\n', 'rsdemo; echo $?\\n'), BANNER + [
            '/> rsdemo pear apple fig',
            'Hello from Rust on Foundation!',
            '3 argument(s)',
            'sorted: apple,fig,pear',
            'file: 14 bytes, apple,fig,pear',
            '/> rsdemo; echo $?',
            'Hello from Rust on Foundation!',
            '0 argument(s)',
            'sorted:',  # the screen dump drops trailing blanks
            'file: 0 bytes,',
            '0',
            '/>',
        ])

    def test_panic_writes_message_and_exits_101(self):
        rows = self.screen('rsdemo --panic; echo $?\\n')
        self.assertEqual(rows[:3], BANNER + ['/> rsdemo --panic; echo $?'])
        self.assertRegex(rows[3], r'^panic at src[\\/]lib\.rs:\d+: requested by --panic$')
        self.assertEqual(rows[4:], ['101', '/>'])


if __name__ == '__main__':
    unittest.main()
