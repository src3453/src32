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
sys.path.insert(0, str(ROOT / 'tools/fs2'))
import fs2  # noqa: E402
STOP_TICKS = 6000
FRAMES = 600
# Keys typed before the shell is up (about frame 120: fs checks the boot
# volume, then init reads /bin/shell.wax) wait in the console's type-ahead.
FIRST_FRAME = 30
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

    def screen(self, *batches, step=STEP_FRAMES, at_frames=None):
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
            frame = at_frames[i] if at_frames is not None else FIRST_FRAME + step * i
            command += ['--type', f'{frame}={text}']
        self.last_run = run
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
            '    4096 KiB free',
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

    def test_paths_and_file_system(self):
        # §7.1 against the fs service on the boot disk: writes work, globs
        # are not there yet.
        self.assertEqual(self.screen(
            'echo a > out.txt\\n', 'echo *.txt\\n', 'ls -l\\n', 'cd /bin\\n', 'pwd\\n',
            './echo.wax a\\n', './echo b\\n', 'cd ..\\n', 'bin/x.wax\\n', 'cd nope\\n'), BANNER + [
            '/> echo a > out.txt',
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

    def test_redirects_reach_the_disk(self):
        # /tmp is RAM, /dev/null takes anything, and a file written to "/"
        # lands on the FS2 volume in dd0 (the same volume as /dd/0).
        rows = self.screen('echo hi > /tmp/a.txt\\n', 'cd /tmp\\n', 'pwd\\n', 'echo y > /dev/null\\n',
                           'echo hello disk > /note.txt\\n', 'cd /dd/0\\n', 'pwd\\n', 'cd /dd/1\\n')
        self.assertEqual(rows, BANNER + [
            '/> echo hi > /tmp/a.txt',
            '/> cd /tmp',
            '/tmp> pwd',
            '/tmp',
            '/tmp> echo y > /dev/null',
            '/tmp> echo hello disk > /note.txt',
            '/tmp> cd /dd/0',
            '/dd/0> pwd',
            '/dd/0',
            '/dd/0> cd /dd/1',
            'cd: /dd/1: not found (-10)',
            '/dd/0>',
        ])
        # The machine stops without unmounting, so the volume stays DIRTY
        # (FS2 §9.1); its files are still readable.
        volume = fs2.Volume(bytearray((self.last_run / 'disk.img').read_bytes()), allow_dirty=True)
        self.assertEqual(volume.read(volume.resolve('/note.txt')), b'hello disk\n')

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
                    r'    2      0 BLOCK    1 +[1-9][0-9]*K disk',
                    r'    3      0 BLOCK    1 +[1-9][0-9]*K fs',
                    r'    4      0 BLOCK    1 +[1-9][0-9]*K init',
                    r'    6      4 (RUN  |BLOCK)    2 +[1-9][0-9]*K shell',
                    r'    7      6 RUN      2 +[1-9][0-9]*K ps']
        self.assertEqual(len(rows), 14, rows)
        for row, pattern in zip(rows[7:13], patterns):
            self.assertRegex(row, '^' + pattern + '$')
        self.assertEqual(rows[13], '/>')

    def test_top_snapshot_and_return_to_shell(self):
        rows = self.screen('top -n 1\\n', 'echo done\\n', at_frames=[30, 300])
        self.assertEqual(rows[:4], BANNER + ['/> top -n 1', 'top - CPU (q/Ctrl+C to quit)'])
        self.assertRegex(rows[4], r'^user +\d+\.\d%  kernel~ +\d+\.\d%  idle +\d+\.\d%$')
        self.assertEqual(rows[5:7], ['Mem: 16384K total, 4096K free',
                                    '  CPU%   PID PARENT STATE PRIO     MEM NAME'])
        # /bin/top runs beside the services, init and the shell, busiest first.
        names = sorted(row.split()[-1] for row in rows[7:13])
        self.assertEqual(names, ['console', 'disk', 'fs', 'init', 'shell', 'top'])
        for row in rows[7:13]:
            self.assertRegex(row, r'^ +\d+\.\d% +\d+ +\d+ (RUN  |BLOCK) +[12] +[1-9][0-9]*K \w+$')
        self.assertEqual(rows[13:], ['/> echo done', 'done', '/>'])
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
                rows = self.screen('top\\n', quit_key, 'echo done\\n', at_frames=[30, 290, 320])
                self.assertEqual(rows[0], 'top - CPU (q/Ctrl+C to quit)')
                self.assertIn('Mem: 16384K total, 4096K free', rows)
                self.assertEqual(rows[-3:], ['/> echo done', 'done', '/>'])

    def test_top_invalid_arguments_and_help(self):
        # top is /bin/top.wax, not a built-in (§10.3), so help does not know it.
        rows = self.screen('top -n 0\\n', 'top -n -1\\n', 'top -n x\\n',
                           'top -n\\n', 'top extra\\n', 'help top\\n')
        self.assertEqual(rows.count('usage: top [-n count]'), 5)
        self.assertEqual(rows[-2:], ['help: top: not found (-10)', '/>'])


    def test_programs_import_libfnd(self):
        # The shell and the commands are WAX v3 that import the WAL fnd from
        # /lib (build.py, Foundation §9.5).
        root = self.work / 'root'
        fnd = (root / 'lib/fnd.1.wal').read_bytes()
        self.assertEqual(fnd[4:8], b'\x00\x03\x00\x02')  # v3 WAL
        self.assertIn(b'needs fsck', fnd)
        for name in ('shell', 'echo', 'ps'):
            image = (root / 'bin' / f'{name}.wax').read_bytes()
            self.assertEqual(image[4:8], b'\x00\x03\x00\x01', name)  # v3 WAX
            self.assertIn(b'fnd_write\x00', image)  # an import
            self.assertNotIn(b'needs fsck', image)  # strerror lives in fnd


if __name__ == '__main__':
    unittest.main()
