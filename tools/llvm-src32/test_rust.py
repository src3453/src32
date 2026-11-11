"""Rust guests through cargo_src32.py (bare metal). Needs Cargo with rust-src
whose LLVM major version matches llvmlite's, Clang and src32_testbench; the
tests skip when Rust is not usable.

Run: python -m unittest discover -s tools/llvm-src32 -p 'test_rust.py' -v
CARGO, CLANG and SRC32_TESTBENCH override executable discovery. The Foundation
side is foundation/tests/rust.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import cargo_src32  # noqa: E402

CONFORMANCE = HERE / 'rust/conformance/Cargo.toml'
BAREMETAL = ROOT / 'examples/rust-baremetal/Cargo.toml'
BENCH = ROOT / 'examples/bench-c-rust/run.py'


def find_testbench():
    if os.environ.get('SRC32_TESTBENCH'):
        return Path(os.environ['SRC32_TESTBENCH'])
    suffix = '.exe' if os.name == 'nt' else ''
    for profile in ('release', 'debug'):
        candidate = ROOT / 'target' / profile / ('src32_testbench' + suffix)
        if candidate.is_file():
            return candidate
    return None


class RustTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        problem = cargo_src32.toolchain_problem()
        if problem:
            raise unittest.SkipTest(problem)
        cls.harness = find_testbench()
        if cls.harness is None:
            raise unittest.SkipTest('build cargo build --release --bin src32_testbench or set SRC32_TESTBENCH')
        cls.clang = os.environ.get('CLANG') or shutil.which('clang')
        # One Cargo target directory for the class: core is built once per profile.
        cls.temp = tempfile.TemporaryDirectory(prefix='llvm-src32-rust-')
        cls.work = Path(cls.temp.name)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def build(self, manifest, output, *options):
        command = [sys.executable, str(HERE / 'cargo_src32.py'), '--manifest-path', str(manifest),
                   '--target-dir', str(self.work / 'cargo'), *options, '-o', str(output)]
        if self.clang:
            command += ['--clang', self.clang]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=900)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return output

    def run_image(self, image, *options):
        run = Path(tempfile.mkdtemp(dir=self.work))  # no BIOS / disk configuration
        result = subprocess.run([str(self.harness), str(image), *options], cwd=run, capture_output=True,
                                text=True, encoding='utf-8', errors='replace', timeout=300)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout, run

    def test_conformance(self):
        # opt-level 2 (release) and 0 (dev, with overflow checks and every core function kept by LTO).
        for profile in ('release', 'dev'):
            image = self.build(CONFORMANCE, self.work / f'conformance-{profile}.wax', '--profile', profile)
            for cpu in ('pipeline', 'cached'):
                with self.subTest(profile=profile, cpu=cpu):
                    stdout, _ = self.run_image(image, '42', '--cpu', cpu)
                    self.assertIn('PASS:', stdout)

    def test_panic_handler_reports_location(self):
        image = self.build(CONFORMANCE, self.work / 'panic.wax', '--features', 'panic')
        _, run = self.run_image(image, '--allow-running', '--skip-render-checks', '--frames', '5',
                                '--dump-uart', 'uart.txt')
        text = (run / 'uart.txt').read_bytes().decode('ascii')
        self.assertRegex(text, r'^panic at src[\\/]lib\.rs:\d+\r\n$')

    def test_baremetal_example(self):
        image = self.build(BAREMETAL, self.work / 'baremetal.wax')
        stdout, _ = self.run_image(image, '42', '--cpu', 'pipeline', '--dump-pcg')
        lines = stdout.splitlines()
        screen = [row for row in lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')] if row]
        self.assertEqual(screen, ['Hello from Rust on CPT32!', 'checks: ok'])

    def test_bench_c_rust(self):
        # The C and Rust versions of the benchmark kernels agree with run.py's Python reference.
        report = self.work / 'bench.json'
        command = [sys.executable, str(BENCH), '--work', str(self.work / 'bench'), '--testbench', str(self.harness),
                   '--json', str(report)]
        if self.clang:
            command += ['--clang', self.clang]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', errors='replace',
                                timeout=1800)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        measured = json.loads(report.read_text(encoding='utf-8'))
        self.assertEqual(measured['problems'], [])
        for language in ('c', 'rust'):
            for kernel, expected in measured['expected'].items():
                with self.subTest(language=language, kernel=kernel):
                    self.assertEqual(measured['results'][language][kernel]['value'], expected)
                    self.assertGreater(measured['results'][language][kernel]['cycles'], 0)
                    self.assertGreater(measured['results'][language][kernel]['stack'], 0)

    def test_missing_foundation_wal_is_reported(self):
        result = subprocess.run([sys.executable, str(HERE / 'cargo_src32.py'), '--manifest-path', str(CONFORMANCE),
                                 '--foundation-wal', str(self.work / 'missing.wal'), '-o', str(self.work / 'x.wax')],
                                capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=300)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('--foundation-wal', result.stderr)
        self.assertNotIn('Traceback', result.stderr)


if __name__ == '__main__':
    unittest.main()
