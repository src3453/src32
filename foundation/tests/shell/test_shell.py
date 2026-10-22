"""Foundation shell on src32_testbench (doc/spec_Foundation_shell.md).

Builds the boot image with foundation/boot/build.py, loads foundation.wax
with a fresh copy of foundation.img in IDC slot 0 (the boot drive, where fs
finds /bin), types keys through the testbench's PeC input (--type) and
compares the PCG text screen (--dump-pcg). The kernel stops the machine at
STOP_TICKS with R1 = 42.

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
FRAMES = 330
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
    @classmethod
    def setUpClass(cls):
        cls.harness = find_testbench()
        cls.temp = tempfile.TemporaryDirectory(prefix='foundation-shell-')
        cls.work = Path(cls.temp.name)
        command = [sys.executable, str(ROOT / 'foundation/boot/build.py'), str(cls.work),
                   '--stop-ticks', str(STOP_TICKS)]
        clang = os.environ.get('CLANG') or shutil.which('clang')
        if clang:
            command += ['--clang', clang]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=600)
        if result.returncode != 0:
            raise RuntimeError(result.stdout + result.stderr)
        cls.image = cls.work / 'foundation.wax'
        cls.disk = (cls.work / 'foundation.img').read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def screen(self, *batches, step=STEP_FRAMES):
        """Types each batch STEP frames after the previous one; returns the
        screen rows without the trailing blank rows."""
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n'
            'disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n',
            encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--cpu', 'pipeline',
                   '--frames', str(FRAMES), '--dump-pcg']
        for i, text in enumerate(batches):
            command += ['--type', f'{FIRST_FRAME + step * i}={text}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        while rows and not rows[-1]:
            rows.pop()
        return rows

    def test_banner_and_external_commands(self):
        # echo, uname and free are /bin programs (§10.3); names are
        # case-insensitive (§1, §7.1).
        self.assertEqual(self.screen('echo hello   world\\n', 'UName\\n', 'echo\\n', 'free\\n'), BANNER + [
            '/> echo hello   world',
            'hello world',
            '/> UName',
            BANNER[0],
            '/> echo',
            '',
            '/> free',
            '   16384 KiB total',
            '    4608 KiB free',
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

    def test_help_lists_only_builtins(self):
        # §10: cd, set, help and exit are built in; help names the shell.
        self.assertEqual(self.screen('help\\n', 'help cd\\n', 'help ls\\n'), BANNER + [
            '/> help',
            BANNER[1],
            'cd      change the current directory',
            'set     show or set variables',
            'help    show help',
            'exit    leave the shell',
            'Other commands are programs found in $PATH.',
            '/> help cd',
            'usage: cd [path]',
            '/> help ls',
            'help: ls: not found (-10)',
            '/>',
        ])

    def test_paths_and_provisional_file_system(self):
        # §7.1 against the provisional read-only fs (services/bootfs): /bin
        # only, no writes, no globs yet.
        self.assertEqual(self.screen(
            'echo a > out.txt\\n', 'echo *.txt\\n', 'ls -l\\n', 'cd /bin\\n', 'pwd\\n',
            './echo.wax a\\n', './echo b\\n', 'cd ..\\n', 'bin/x.wax\\n', 'cd nope\\n'), BANNER + [
            '/> echo a > out.txt',
            'shell: out.txt: read-only (-18)',
            '/> echo *.txt',
            'shell: *.txt: not supported (-13)',
            '/> ls -l',
            'ls: command not found (-10)',
            '/> cd /bin',
            '/bin> pwd',
            '/bin',
            '/bin> ./echo.wax a',
            'a',
            '/bin> ./echo b',
            './echo: bad format (-12)',
            '/bin> cd ..',
            '/> bin/x.wax',
            'bin/x.wax: not found (-10)',
            '/> cd nope',
            'cd: nope: not found (-10)',
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
        # EXIT ends the shell; init starts it again. ps shows the services,
        # init, the new shell and ps itself; exited processes are gone.
        rows = self.screen('exit 3\\n', 'ps\\n', step=20)
        self.assertEqual(rows[:7], BANNER + ['/> exit 3'] + BANNER + ['/> ps', '  PID PARENT STATE PRIO     MEM NAME'])
        # MEM depends on the build.
        patterns = [r'    1      0 BLOCK    1 +[1-9][0-9]*K console',
                    r'    2      0 BLOCK    1 +[1-9][0-9]*K fs',
                    r'    3      0 BLOCK    1 +[1-9][0-9]*K init',
                    r'    5      3 BLOCK    2 +[1-9][0-9]*K shell',
                    r'    6      5 RUN      2 +[1-9][0-9]*K ps']
        self.assertEqual(len(rows), 13, rows)
        for row, pattern in zip(rows[7:12], patterns):
            self.assertRegex(row, '^' + pattern + '$')
        self.assertEqual(rows[12], '/>')

if __name__ == '__main__':
    unittest.main()
