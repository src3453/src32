"""Foundation shell on src32_testbench (doc/spec_Foundation_shell.md).

Builds the boot image with foundation/boot/build.py, loads foundation.wax
with a fresh copy of foundation.img in IDC slot 0 (the boot drive, where fs
finds /bin), types keys through the testbench's PeC input (--type) and
compares the PCG text screen (--dump-pcg). The kernel stops the machine at
STOP_TICKS with R1 = 42.

    python -m unittest discover -s foundation/tests/shell -v

CLANG and SRC32_TESTBENCH override executable discovery.
Functional checks use cached; restart scheduling and one CPU-accounting
snapshot use pipeline. FOUNDATION_TEST_CPU overrides the engine for all checks.
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
sys.path.insert(0, str(ROOT / 'tools/fs2'))
import fs2  # noqa: E402
STOP_TICKS = 800  # 8 seconds at 100 Hz, including long command batches
# FND_MM=nommu forces physical memory; otherwise CPUID decides the actual backend.
FRAMES = 660
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

    def free_kib(self, rows):
        """Free memory depends on the image sizes and the backend: require a
        positive value and normalize it to N."""
        out = []
        for row in rows:
            match = re.match(r'^( +)(\d+)( KiB free)$', row)
            if match:
                self.assertGreater(int(match.group(2)), 0, row)
                row = ' ' * (len(match.group(1)) + len(match.group(2)) - 1) + 'N' + match.group(3)
            match = re.match(r'^(Mem: \d+K total, )(\d+)(K free)$', row)
            if match:
                self.assertGreater(int(match.group(2)), 0, row)
                row = match.group(1) + 'N' + match.group(3)
            out.append(row)
        return out

    def free_line(self, kib):
        return '%8s KiB free' % 'N'

    def mem_line(self, kib):
        return 'Mem: 16384K total, NK free'

    def screen(self, *batches, step=STEP_FRAMES, at_frames=None, config=None, cpu='cached', final=(), keys=(), audio=False):
        """Types each batch STEP frames after the previous one; returns the
        screen rows without the trailing blank rows. config is the text of a
        config.yaml for this run only."""
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            (config or 'memory:\n  ram_mib: 16\n  vram_mib: 4\n') +
            'disk:\n  - slot: 0\n    type: "hdd"\n    image: "disk.img"\n    read_only: false\n',
            encoding='utf-8')
        cpu = os.environ.get('FOUNDATION_TEST_CPU', cpu)
        command = [str(self.harness), str(self.image), '42', '--cpu', cpu,
                   '--frames', str(FRAMES), '--dump-pcg']
        if audio:
            command += ['--dump-audio', str(run / 'audio.pcm')]
        for i, text in enumerate(batches):
            frame = at_frames[i] if at_frames is not None else FIRST_FRAME + step * i
            command += ['--type', f'{frame}={text}']
        for address, value in final:
            command += ['--expect-final-mem-u8', f'{address:#x}={value}']
        for key in keys:
            command += ['--key', key]
        self.last_run = run
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        while rows and not rows[-1]:
            rows.pop()
        return self.free_kib(rows)

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
            self.free_line(4096),
            '    4096 KiB VRAM',
            '    1024 KiB PCMRAM',
            '/>',
        ])

    def test_exec_after_image_replacement(self):
        # disk's read cache must discard old executable sectors on overwrite.
        self.assertEqual(self.screen(
            'echo before\\n', 'cp -f /bin/uname.wax /bin/echo.wax\\n', 'echo\\n'), BANNER + [
            '/> echo before',
            'before',
            '/> cp -f /bin/uname.wax /bin/echo.wax',
            '/> echo',
            BANNER[0],
            '/>',
        ])

    def test_free_reports_installed_memory(self):
        # The kernel takes the sizes from the BMC (spec_BMC_DMAC.md §1.5);
        # the free memory is whatever pages the processes have left.
        self.assertEqual(self.screen('free\\n', config='memory:\n  ram_mib: 12\n  vram_mib: 8\n'),
                         BANNER + [
            '/> free',
            '   12288 KiB total',
            self.free_line(0),
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

    def test_ls(self):
        # §10.3, §10.4: names sorted ignoring case, -l with the totals, a
        # file path lists just that file, errors use the §11.1 format.
        rows = self.screen('echo a > B.txt\\n', 'echo a > a.txt\\n', 'ls /tmp\\n', 'ls\\n', 'ls /bin/echo.wax\\n',
                           'ls /nope\\n', 'ls -x\\n')
        self.assertEqual(rows[:2], BANNER)
        self.assertIn('/> ls', rows)
        self.assertTrue(any(row.startswith('a.txt   B.txt   ') for row in rows), rows)
        self.assertIn('/bin/echo.wax', rows)
        self.assertIn('ls: /nope: not found (-10)', rows)
        self.assertIn('usage: ls [-l] [path...]', rows)

    def test_ls_tab_output(self):
        # Check actual HT/LF bytes as well as console expansion and boundaries.
        rows = self.screen('mkdir /names\\n', 'mkdir /empty\\n',
                           'echo a > /names/B.txt\\n', 'echo a > /names/a.txt\\n',
                           'ls /names > /list.txt\\n', 'ls /empty > /empty.txt\\n',
                           'ls /names /empty /names/a.txt\\n')
        self.assertEqual(rows[-9:], [
            '/> ls /names /empty /names/a.txt',
            '/names:', 'a.txt   B.txt', '', '/empty:', '',
            '/names/a.txt:', '/names/a.txt', '/>',
        ])
        vol = fs2.Volume(bytearray((self.last_run / 'disk.img').read_bytes()), allow_dirty=True)
        self.assertEqual(vol.read(vol.resolve('/list.txt')), b'a.txt\tB.txt\n')
        self.assertEqual(vol.read(vol.resolve('/empty.txt')), b'')

    def test_hex(self):
        # Cursor edit both nibbles, ASCII edit, exact goto, legacy command,
        # save and return. Check the disk bytes as well as shell recovery.
        rows = self.screen('echo 0123456789ABCD > h.txt\\n', 'hex h.txt\\n',
                           '58\\R\\tZ\\t', 'g000e\\n', '00', ':e 1 59\\n',
                           'w', 'q', 'hex nope\\n', 'hex\\n', step=32)
        vol = fs2.Volume(bytearray((self.last_run / 'disk.img').read_bytes()), allow_dirty=True)
        self.assertEqual(vol.read(vol.resolve('/h.txt')), b'XYZ3456789ABCD\x00')
        self.assertIn('hex: nope: not found (-10)', rows)
        self.assertIn('usage: hex path', rows)

    def hex_disk(self, content):
        vol = fs2.Volume(bytearray(self.disk))
        vol.put('/hex.bin', content, mode=0x24)
        self.disk = bytes(vol.img)

    def test_hex_all_glyphs_and_colors(self):
        original = self.disk
        try:
            self.hex_disk(bytes(range(256)))
            final = []
            # ASCII column is a literal PCG glyph, even NUL/LF/ESC/FF.
            for i in range(256):
                cell = (3 + i // 16) * 80 + 54 + i % 16
                final.append((0x10000000 + cell, i))
            # Cursor starts on 00: cyan highlight; nonzero bytes are white.
            final += [(0x10001000 + 3 * 80 + 5, 0),
                      (0x10002000 + 3 * 80 + 5, 31),
                      (0x10001000 + 3 * 80 + 8, 63)]
            self.screen('hex hex.bin\\n', step=40, final=final)
        finally:
            self.disk = original

    def test_hex_zero_grey_8x16_and_command_cancel(self):
        original = self.disk
        try:
            self.hex_disk(bytes(256))
            rows = self.screen('mode 8x16\\n', 'hex hex.bin > ignored.txt\\n',
                               'gffff\\e', ':g zz\\n', step=40,
                               final=[(0x8000F003, 3),
                                      (0x10001000 + 3 * 80 + 8, 21),
                                      (0x10001000 + 3 * 80 + 55, 21)])
            self.assertIn('ERROR: BAD ADDRESS', rows)
        finally:
            self.disk = original

    def test_hex_control_save_quit_and_ascii_q(self):
        original = self.disk
        try:
            self.hex_disk(bytes(4))
            keys = []
            for frame, usage in ((240, 22), (300, 20)):  # Ctrl+S, Ctrl+Q
                keys += [f'{frame}:224:1', f'{frame}:{usage}:1',
                         f'{frame}:{usage}:0', f'{frame}:224:0']
            rows = self.screen('hex hex.bin\\n', '\\tq', 'echo shortcuts\\n',
                               at_frames=[30, 190, 350], keys=keys,
                               final=[(0x8000F007, 1), (0x10001000 + 27 * 80, 63),
                                      (0x10002000 + 27 * 80, 0)])
            self.assertIn('shortcuts', rows)
            # STOP_TICKS halts with a mounted (DIRTY) volume: inspect only.
            vol = fs2.Volume(bytearray((self.last_run / 'disk.img').read_bytes()), allow_dirty=True)
            self.assertEqual(vol.read(vol.resolve('/hex.bin')), b'q\x00\x00\x00')
        finally:
            self.disk = original

    def test_hex_bounds_goto_and_discard(self):
        original = self.disk
        try:
            self.hex_disk(bytes(65536))
            rows = self.screen('hex hex.bin\\n', '\\L\\U', 'gffff\\n', 'aa\\R\\D',
                               'g10000\\n', 'q', 'n', 'q', 'y', 'echo returned\\n', step=35)
            self.assertIn('UNSAVED CHANGES DISCARDED', rows)
            self.assertIn('returned', rows)
            vol = fs2.Volume(bytearray((self.last_run / 'disk.img').read_bytes()), allow_dirty=True)
            self.assertEqual(vol.read(vol.resolve('/hex.bin')), bytes(65536))
        finally:
            self.disk = original

    def test_hex_40_columns_zero_and_page(self):
        original = self.disk
        try:
            self.hex_disk(bytes(256))
            # N moves 128 bytes in 40 columns; editing must not cross EOF.
            rows = self.screen('mode 40\\n', 'hex hex.bin\\n', 'n', '\\Eaa', 'w', 'q',
                               'echo ready\\n', step=40)
            self.assertIn('ready', rows)
            vol = fs2.Volume(bytearray((self.last_run / 'disk.img').read_bytes()), allow_dirty=True)
            self.assertEqual(vol.read(vol.resolve('/hex.bin')), bytes(135) + b'\xaa' + bytes(120))
        finally:
            self.disk = original

    def test_ls_long(self):
        rows = self.screen('ls -l /bin\\n')
        self.assertTrue(any(r.endswith('  ls.wax') and '  <DIR>' not in r for r in rows), rows)
        self.assertTrue(any(r.endswith(' file(s), 1 dir(s)') or ' file(s), 0 dir(s)' in r for r in rows), rows)

    def test_cat_and_cp(self):
        # §10.3, §10.5: cat joins its files, cp refuses an existing target
        # unless -f, copies into a directory and rejects the same file.
        rows = self.screen(
            'echo hello > a.txt\\n', 'cat a.txt\\n', 'cp a.txt b.txt\\n', 'cat b.txt a.txt\\n', 'cp a.txt b.txt\\n',
            'echo new > a.txt\\n', 'cp -f a.txt b.txt\\n', 'cat b.txt\\n', 'cp a.txt a.txt\\n', 'cat /bin\\n', 'cat\\n',
            'cp -x a b\\n')
        self.assertEqual(rows, BANNER + [
            '/> echo hello > a.txt',
            '/> cat a.txt',
            'hello',
            '/> cp a.txt b.txt',
            '/> cat b.txt a.txt',
            'hello',
            'hello',
            '/> cp a.txt b.txt',
            'cp: b.txt: already exists (-11)',
            '/> echo new > a.txt',
            '/> cp -f a.txt b.txt',
            '/> cat b.txt',
            'new',
            '/> cp a.txt a.txt',
            'cp: a.txt: invalid argument (-2)',
            '/> cat /bin',
            'cat: /bin: is a directory (-65)',
            '/> cat',
            'usage: cat path...',
            '/> cp -x a b',
            'usage: cp [-f] src... dst',
            '/>',
        ])

    def test_directory_commands(self):
        # §10.3, §10.5: mkdir, mv, rm and rmdir; a failed target does not stop
        # the rest. The screen scrolls, so only the tail is compared.
        rows = self.screen(
            'echo x > a.txt\\n', 'mkdir d\\n', 'cp a.txt a.txt d\\n', 'mv a.txt d/c.txt\\n',
            'mv d/c.txt d/a.txt\\n', 'rmdir d\\n', 'rm d/c.txt nope d/a.txt\\n', 'rmdir d\\n')
        self.assertEqual(rows[-9:], [
            '/> mv a.txt d/c.txt',
            '/> mv d/c.txt d/a.txt',
            'mv: d/c.txt: already exists (-11)',
            '/> rmdir d',
            'rmdir: d: directory not empty (-66)',
            '/> rm d/c.txt nope d/a.txt',
            'rm: nope: not found (-10)',
            '/> rmdir d',
            '/>',
        ])

    def test_paths_and_file_system(self):
        # §7.1 against the fs service on the boot disk: writes work, globs
        # are not there yet.
        self.assertEqual(self.screen(
            'echo a > out.txt\\n', 'echo *.txt\\n', 'ls out.txt\\n', 'cd /bin\\n', 'pwd\\n',
            './echo.wax a\\n', './echo b\\n', 'cd ..\\n', 'bin/x.wax\\n', 'cd nope\\n'), BANNER + [
            '/> echo a > out.txt',
            '/> echo *.txt',
            'shell: *.txt: not supported (-13)',
            '/> ls out.txt',
            'out.txt',
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
        rows = self.screen('exit 3\\n', 'ps\\n', step=20, cpu='pipeline')
        self.assertEqual(rows[:7], BANNER + ['/> exit 3'] + BANNER + ['/> ps', '  PID PARENT STATE PRIO     MEM NAME'])
        # MEM depends on the build. After spawn, fnd__exec frees its images
        # before the shell calls wait. A time-slice expiry in that cleanup
        # lets ps snapshot the ready shell (RUN) before it blocks in wait.
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
        rows = self.screen('top -n 1\\n', 'echo done\\n', at_frames=[30, 300], cpu='pipeline')
        self.assertEqual(rows[:4], BANNER + ['/> top -n 1', 'top - CPU (q/Ctrl+C to quit)'])
        self.assertRegex(rows[4], r'^user +\d+\.\d%  kernel~ +\d+\.\d%  idle +\d+\.\d%$')
        self.assertEqual(rows[5:7], [self.mem_line(4096),
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
                self.assertIn(self.mem_line(4096), rows)
                self.assertEqual(rows[-3:], ['/> echo done', 'done', '/>'])

    def test_ctrl_c_kills_running_command(self):
        # §9.3: the break thread kills the running program; $? is KILLED (-20)
        # and ^C goes to STDERR. top ignores nothing here: the shell never
        # passes Ctrl+C to the child.
        rows = self.screen('top\\n', '\\C', 'echo $?\\n', at_frames=[30, 290, 320])
        self.assertIn('^C', rows)
        self.assertEqual(rows[-3:], ['/> echo $?', '-20', '/>'])

    def test_ctrl_c_interrupts_a_blocked_read(self):
        # hex waits in CON_READ_KEY for a cursor/edit key; Ctrl+C fails the read with
        # INTERRUPTED (§8.4) and the shell kills hex. Repeating works.
        rows = self.screen('hex /bin/echo.wax\\n', '\\C', 'hex /bin/echo.wax\\n', '\\C', 'echo $?\\n', 'echo again\\n',
                           at_frames=[30, 300, 330, 360, 390, 420],
                           final=[(0x8000F007, 1)])
        self.assertTrue(any('^C' in row for row in rows), rows)
        # Shell interruption clears old TUI rows, including their tails.
        self.assertEqual(rows[-5:], ['/> echo $?', '-20', '/> echo again', 'again', '/>'])

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
