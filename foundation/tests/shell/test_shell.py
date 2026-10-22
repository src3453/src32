"""Foundation shell on src32_testbench (doc/spec_Foundation_shell.md).

Builds the boot image with foundation/boot/build.py, types keys through the
testbench's PeC input (--type) and compares the PCG text screen (--dump-pcg).
The kernel stops the machine at STOP_TICKS with R1 = 42. FndWalShellTests
runs the same tests with libfnd as the WAL fnd (build.py --fnd-wal).

    python -m unittest discover -s foundation/tests/shell -v

CLANG and SRC32_TESTBENCH override executable discovery.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
STOP_TICKS = 3000
FRAMES = 260
FIRST_FRAME = 30   # the shell is loaded and waiting for keys by then
STEP_FRAMES = 6    # between two --type batches
BANNER = ['Foundation OS (kernel ABI 1.0)', 'Foundation shell 1.0']  # §2.1


def find_testbench():
    suffix = '.exe' if os.name == 'nt' else ''
    if os.environ.get('SRC32_TESTBENCH'):
        return Path(os.environ['SRC32_TESTBENCH'])
    for profile in ('release', 'debug'):
        candidate = ROOT / 'target' / profile / ('src32_testbench' + suffix)
        if candidate.is_file():
            return candidate
    raise RuntimeError('Build cargo build --bin src32_testbench or set SRC32_TESTBENCH')


class ShellTests(unittest.TestCase):
    BUILD_OPTIONS = []

    @classmethod
    def setUpClass(cls):
        cls.harness = find_testbench()
        cls.temp = tempfile.TemporaryDirectory(prefix='foundation-shell-')
        cls.work = Path(cls.temp.name)
        command = [sys.executable, str(ROOT / 'foundation/boot/build.py'), str(cls.work),
                   '--stop-ticks', str(STOP_TICKS), *cls.BUILD_OPTIONS]
        clang = os.environ.get('CLANG') or shutil.which('clang')
        if clang:
            command += ['--clang', clang]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=600)
        if result.returncode != 0:
            raise RuntimeError(result.stdout + result.stderr)
        cls.image = cls.work / 'foundation.wax'

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def screen(self, *batches, step=STEP_FRAMES, at_frames=None, config=None):
        """Types each batch STEP frames after the previous one; returns the
        screen rows without the trailing blank rows. config is the text of a
        config.yaml for this run only."""
        command = [str(self.harness), str(self.image), '42', '--cpu', 'pipeline',
                   '--frames', str(FRAMES), '--dump-pcg']
        for i, text in enumerate(batches):
            frame = at_frames[i] if at_frames is not None else FIRST_FRAME + step * i
            command += ['--type', f'{frame}={text}']
        with tempfile.TemporaryDirectory(prefix='foundation-shell-run-') as run_dir:
            cwd = self.work
            if config is not None:
                cwd = Path(run_dir)
                (cwd / 'config.yaml').write_text(config, encoding='utf-8')
            result = subprocess.run(command, cwd=cwd, capture_output=True, text=True,
                                    encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        while rows and not rows[-1]:
            rows.pop()
        return rows

    def test_banner_builtins_and_versions(self):
        # Command names are case-insensitive (§1).
        self.assertEqual(self.screen('echo hello   world\\n', 'UName\\n', 'echo\\n', 'free\\n'), BANNER + [
            '/> echo hello   world',
            'hello world',
            '/> UName',
            *BANNER,
            '/> echo',
            '',
            '/> free',
            '   16384 KiB total',
            '   10240 KiB free',
            '    4096 KiB VRAM',
            '    1024 KiB PCMRAM',
            '/>',
        ])

    def test_free_reports_installed_memory(self):
        # The kernel takes the sizes from the BMC (spec_BMC_DMAC.md §1.4).
        # With 8 MiB of RAM only slots 0-4 exist, and the shell holds slot 4.
        self.assertEqual(self.screen('free\\n', config='memory:\n  ram_mib: 8\n  vram_mib: 8\n'),
                         BANNER + [
            '/> free',
            '    8192 KiB total',
            '    2048 KiB free',
            '    8192 KiB VRAM',
            '    1024 KiB PCMRAM',
            '/>',
        ])

    def test_quotes_and_variables(self):
        # §3.2, §5.1: values are not split again; '...' expands nothing.
        self.assertEqual(self.screen(
            'SET "MSG=hello  world"\\n',
            'echo $msg ${MSG}x \'$MSG\' "[$MSG]" \\\\$MSG a"b c"d $ $?\\n',
            'set msg\\n', 'set\\n', 'set MSG=\\n', 'set msg\\n', 'echo [$NOPE] $0 $# [$1]\\n'), BANNER + [
            '/> SET "MSG=hello  world"',
            '/> echo $msg ${MSG}x \'$MSG\' "[$MSG]" \\$MSG a"b c"d $ $?',
            'hello  world hello  worldx $MSG [hello  world] $MSG ab cd $ 0',
            '/> set msg',
            'hello  world',
            '/> set',
            'MSG=hello  world',
            'PATH=/bin',
            '/> set MSG=',
            '/> set msg',
            'set: msg: not found (-10)',
            '/> echo [$NOPE] $0 $# [$1]',
            '[] shell 0 []',
            '/>',
        ])

    def test_conditionals_and_status(self):
        # §4: && and || from the left; a command that does not run keeps $?.
        self.assertEqual(self.screen(
            'foo && echo a || echo b\\n', 'nope; echo $?\\n', 'echo x || nope; echo $?\\n',
            'exit x; echo $?\\n'), BANNER + [
            '/> foo && echo a || echo b',
            'foo: command not found (-10)',
            'b',
            '/> nope; echo $?',
            'nope: command not found (-10)',
            '-10',
            '/> echo x || nope; echo $?',
            'x',
            '0',
            '/> exit x; echo $?',
            'usage: exit [code]',
            '-2',
            '/>',
        ])

    def test_syntax_errors_run_nothing(self):
        self.assertEqual(self.screen(
            'echo "abc\\n', 'echo a; ; echo b\\n', 'echo a | b\\n', 'echo a &\\n', 'echo a &&\\n',
            '> x\\n', 'echo a\\\\\\n', '  # comment\\n', 'echo a#b 2>&1; echo $?\\n'), BANNER + [
            '/> echo "abc',
            'shell: unterminated quote: invalid argument (-2)',
            '/> echo a; ; echo b',
            'shell: syntax error: invalid argument (-2)',
            '/> echo a | b',
            "shell: '|': not supported (-13)",
            '/> echo a &',
            "shell: '&': not supported (-13)",
            '/> echo a &&',
            'shell: syntax error: invalid argument (-2)',
            '/> > x',
            'shell: syntax error: invalid argument (-2)',
            '/> echo a\\',
            'shell: trailing \\: invalid argument (-2)',
            '/>   # comment',
            '/> echo a#b 2>&1; echo $?',
            'a#b',
            '0',
            '/>',
        ])

    def test_needs_file_system(self):
        # Provisional: no fs service yet (README.md).
        self.assertEqual(self.screen(
            'echo a > out.txt\\n', 'echo *.txt\\n', 'ls -l\\n', 'cd\\n', 'pwd\\n', 'cd /bin\\n', '/bin/x.wax\\n'), BANNER + [
            '/> echo a > out.txt',
            'echo: out.txt: not supported (-13)',
            '/> echo *.txt',
            'shell: *.txt: not supported (-13)',
            '/> ls -l',
            'ls: not supported (-13)',
            '/> cd',
            '/> pwd',
            '/',
            '/> cd /bin',
            'cd: /bin: not supported (-13)',
            '/> /bin/x.wax',
            '/bin/x.wax: command not found (-10)',
            '/>',
        ])

    def test_line_editing_and_history(self):
        # §9.2: arrows, Home, End, Backspace, Delete, history and Ctrl+C.
        self.assertEqual(self.screen(
            'echo wrld\\L\\L\\Lo\\n', 'xecho hi\\H\\X\\E!\\n', 'echo abcd\\b\\bef\\n', '\\n',
            '\\U\\U\\n', '\\U\\U\\U\\D\\n', 'echo nope\\L\\L\\C', 'echo $?\\n'), BANNER + [
            '/> echo world',
            'world',
            '/> echo hi!',
            'hi!',
            '/> echo abef',
            'abef',
            '/>',
            '/> echo hi!',
            'hi!',
            '/> echo abef',
            'abef',
            '/> echo nope^C',
            '/> echo $?',
            '0',
            '/>',
        ])

    def test_clear_and_scrolling(self):
        batches = ['echo first\\n', 'clear\\n'] + [f'echo line{i}\\n' for i in range(20)]
        rows = self.screen(*batches, step=4)
        # 20 commands of 2 rows and the prompt scroll the cleared screen by 11.
        self.assertEqual(len(rows), 30)
        self.assertEqual(rows[0], 'line5')
        self.assertEqual(rows[-3:], ['/> echo line19', 'line19', '/>'])
        self.assertNotIn('first', '\n'.join(rows))

    def test_exit_restarts_shell(self):
        # EXIT ends the shell; the boot image starts it again (as init would).
        rows = self.screen('exit 3\\n', 'ps\\n', step=20)
        self.assertEqual(rows[:5] + rows[6:], BANNER + [
            '/> ps',
            '  PID PARENT STATE PRIO     MEM NAME',
            '    1      0 EXIT     2      0K shell',
            '/>',
        ])
        # MEM: the image (its size depends on the build) and the 16 KiB stack.
        self.assertRegex(rows[5], r'^    2      0 RUN      2 +[1-9][0-9]*K shell$')

    def test_top_snapshot_and_return_to_shell(self):
        rows = self.screen('top -n 1\\n', 'echo done\\n', at_frames=[30, 135])
        self.assertEqual(rows[:4], BANNER + ['/> top -n 1', 'top - CPU (q/Ctrl+C to quit)'])
        self.assertRegex(rows[4], r'^user +\d+\.\d%  kernel~ +\d+\.\d%  idle +\d+\.\d%$')
        self.assertEqual(rows[5:7], ['Mem: 16384K total, 10240K free',
                                    '  CPU%   PID PARENT STATE PRIO     MEM NAME'])
        self.assertRegex(rows[7], r'^ +\d+\.\d% +1 +0 RUN +2 +[1-9][0-9]*K shell$')
        self.assertEqual(rows[8:], ['/> echo done', 'done', '/>'])
        # Check measured proportions, rather than pinning costs to one build.
        values = [float(v.rstrip('%')) for v in rows[4].split() if v.endswith('%')]
        self.assertTrue(all(0 <= v <= 100 for v in values))
        self.assertAlmostEqual(sum(values), 100, delta=0.3)
        self.assertGreater(values[1], 0)
        self.assertGreater(values[2], 50)

    def test_top_multiple_snapshots(self):
        rows = self.screen('top -n 2\\n')
        self.assertEqual(rows.count('top - CPU (q/Ctrl+C to quit)'), 2)
        self.assertEqual(rows[-1], '/>')

    def test_top_refresh_and_quit(self):
        for quit_key in ('q', '\\C'):
            with self.subTest(quit_key=quit_key):
                rows = self.screen('top\\n', quit_key, 'echo done\\n', at_frames=[30, 125, 145])
                self.assertEqual(rows[0], 'top - CPU (q/Ctrl+C to quit)')
                self.assertIn('Mem: 16384K total, 10240K free', rows)
                self.assertEqual(rows[-3:], ['/> echo done', 'done', '/>'])

    def test_top_invalid_arguments_and_help(self):
        rows = self.screen('top -n 0\\n', 'top -n -1\\n', 'top -n x\\n',
                           'top -n\\n', 'top extra\\n', 'help top\\n')
        self.assertEqual(rows.count('usage: top [-n count]'), 6)
        self.assertEqual(rows[-1], '/>')


class FndWalShellTests(ShellTests):
    """The shell as a WAX v3 importing libfnd from the WAL fnd 1.0."""
    BUILD_OPTIONS = ['--fnd-wal']

    def test_shell_imports_libfnd(self):
        shell, fnd = (self.work / 'shell.wax').read_bytes(), (self.work / 'fnd.wal').read_bytes()
        self.assertEqual(shell[4:8], b'\x00\x03\x00\x01')  # v3 WAX
        self.assertEqual(fnd[4:8], b'\x00\x03\x00\x02')  # v3 WAL
        self.assertIn(b'fnd_write\x00', shell)  # an import
        self.assertNotIn(b'needs fsck', shell)  # strerror lives in fnd
        self.assertIn(b'needs fsck', fnd)


if __name__ == '__main__':
    unittest.main()
