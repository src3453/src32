"""ST_WRITE over real libfnd/IPC with PCG text and attribute checks.

Run also with FND_MM=slot. Uses the shell boot builder and testbench.
"""
import os
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE.parent / 'shell'))
import test_shell
sys.path.insert(0, str(ROOT / "tools/wax"))
import wax


class ConsoleGuestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        # Testbench does not load GUI font configuration. Seed a custom font
        # in a test-only WAX block, exactly where pcg.font_8x16 loads it.
        cls.font = bytes((i * 37 + 0x5a) & 255 for i in range(4096))
        cls.font = cls.font[:32 * 16] + bytes(16) + cls.font[33 * 16:]
        image = wax.parse(cls.image.read_bytes())
        blocks = [(b['load_address'], b['data']) for b in image['blocks']]
        blocks.append((0x10021000, cls.font))
        cls.image = cls.work / 'configured-font.wax'
        cls.image.write_bytes(wax.pack(image['base'], image['entry_offset'], blocks))
        guest = cls.work / 'concheck.wax'
        command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'), str(HERE / 'guest.c'),
                   '-I', str(ROOT / 'foundation/include'), '-I', str(ROOT / 'sdk/cpt32/include'),
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.console',
                   '--import-wal', str(cls.work / 'root/lib/fnd.1.wal'),
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', str(guest)]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        result = subprocess.run(command, capture_output=True, text=True, timeout=120)
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        volume = test_shell.fs2.Volume(bytearray(cls.disk))
        volume.put('/bin/concheck.wax', guest.read_bytes(), mode=0x3D)
        volume.put('/display.txt', b'\x1b[2J\x1b[H\x1b[31;44mA \x1b[0m'
                   b'\x1b[30;1H\x1b[41m \x1b[0m\x1b[3;1H', mode=0x3D)
        volume.put('/wide.txt', '\x1b[2J\x1b[H\x1b[31;44m日本a\x1b[0m'.encode(), mode=0x3D)
        # Mixed-width prefixes leave two cells, one cell, or no cells.
        edge = '\x1b[2J\x1b[H'
        for row, prefix in [(3, 'a日' * 26), (6, 'a日' * 26 + 'b'),
                            (9, 'a日' * 26 + 'bc')]:
            edge += f'\x1b[{row};1H\x1b[31;44m{prefix}日本z'
        # Keep the shell prompt away from the rows inspected below.
        edge += '\x1b[0m\x1b[15;1H'
        volume.put('/edge.txt', edge.encode(), mode=0x3D)
        volume.put('/scroll.txt', b'row\n' * 46, mode=0x3D)
        cls.disk = bytes(volume.img)

    @classmethod
    def tearDownClass(cls):
        test_shell.ShellTests.tearDownClass.__func__(cls)

    def run_console(self, *batches, final=(), pixels=(), spacing=40):
        self.run_count = getattr(self, 'run_count', 0) + 1
        run = self.work / f'{self.id().split(".")[-1]}-{self.run_count}'
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text(
            'memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', '660', '--dump-pcg',
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', 'cached')]
        for i, batch in enumerate(batches):
            command += ['--type', f'{30 + i * spacing}={batch}']
        for address, value in final:
            command += ['--expect-final-mem-u8', f'{address:#x}={value}']
        for x, y, rgb in pixels:
            command += ['--expect-pixel', f'{x}:{y}={rgb:#x}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_ansi_via_ipc(self):
        text = self.run_console('concheck\\n', 'echo status=$?\\n', spacing=170,
                               final=[(0x10001000, 32), (0x10002000, 2),
                               (0x10001001, 2), (0x10002001, 53),
                               (0x10001002, 63), (0x10002002, 0),
                               (0x8000F007, 1)])
        self.assertIn('CRD', text)
        self.assertIn('console guest passed', text)
        self.assertIn('status=0', text)

    def test_mode_40_query_and_line_edit(self):
        text = self.run_console('mode 40\\n', 'mode\\n', 'echo wrld\\L\\L\\Lo\\n',
                                final=[(0x8000F003, 0)])
        self.assertIn('40x30, 8x8', text)
        self.assertIn('world', text)

    def test_mode_80_restores_default(self):
        text = self.run_console('mode 8x16\\n', 'mode 40\\n', 'mode 80\\n', 'mode\\n',
                                final=[(0x8000F003, 1), (0x8000F00E, 0), (0x8000F00F, 0)])
        self.assertIn('80x30, 8x8', text)

    def test_mode_errors_and_redirected_output(self):
        text = self.run_console('mode 40 > /switch.txt\\n', 'mode > /current.txt\\n',
                                'cat /current.txt\\n', 'mode 2; echo bad=$?\\n',
                                'mode 40 extra; echo extra=$?\\n', 'mode ""; echo empty=$?\\n',
                                final=[(0x8000F003, 0)])
        self.assertIn('40x30, 8x8', text)
        self.assertEqual(text.count('usage: mode [40|80|8x16]'), 3)
        for name in ('bad', 'extra', 'empty'):
            self.assertIn(f'{name}=-2', text)  # FND_E_BAD_ARG (api.h)

    def test_40_column_display_pixels(self):
        text = self.run_console('mode 40\\n', 'cat /display.txt\\n',
                                final=[(0x8000F003, 0), (0x10001000, 32), (0x10002000, 2)],
                                pixels=[(8, 0, 0x0000AA), (0, 232, 0xAA0000), (0, 239, 0xAA0000)])
        self.assertIn('A', text)

    def test_configured_8x16_font_survives_switches_and_display_pixels(self):
        glyph = self.font[ord('A') * 16:(ord('A') + 1) * 16]
        y = next(i for i, bits in enumerate(glyph) if bits)
        x = next(i for i in range(8) if glyph[y] & (0x80 >> i))
        offsets = sorted(set([0, 31 * 16, 127 * 16, 255 * 16, 4095] +
                             list(range(ord('A') * 16, (ord('A') + 1) * 16))))
        text = self.run_console('mode 8x16\\n', 'mode 8x16\\n', 'mode 40\\n',
                                'mode 80\\n', 'mode 8x16\\n', 'mode\\n', 'cat /display.txt\\n',
                                final=[(0x8000F003, 3)] +
                                      [(0x10021000 + i, self.font[i]) for i in offsets],
                                pixels=[(x, y, 0xAA0000), (8, 0, 0x0000AA),
                                        (0, 464, 0xAA0000), (0, 479, 0xAA0000)])
        self.assertIn('A', text)

    def test_wide_characters_in_8x16(self):
        rom = (ROOT / 'assets/unifont-bmp.chr').read_bytes()

        def lit(unit):
            glyph = rom[unit * 32:unit * 32 + 32]
            return [(x, y) for y in range(16) for x in range(16)
                    if glyph[y * 2 + x // 8] & (0x80 >> x % 8)]
        pixels = []
        for index, unit in enumerate('日本'):
            x, y = lit(ord(unit))[0]
            pixels.append((index * 16 + x, y, 0xAA0000))
        # Each character is a pair of cells: WIDE_KIND 01 (MSB) then 10 (LSB), CLUT 32 (red).
        cells = [(0x10000000, 0x65), (0x10000001, 0xE5), (0x10000002, 0x67), (0x10000003, 0x2C),
                 (0x10000004, ord('a')),
                 (0x10001000, 32 | 0x40), (0x10001001, 32 | 0x80), (0x10001002, 32 | 0x40),
                 (0x10001003, 32 | 0x80), (0x10001004, 32), (0x10002000, 2), (0x10002001, 2),
                 (0x8000F010, 0), (0x8000F011, 0), (0x8000F012, 0x20), (0x8000F013, 0)]
        text = self.run_console('mode 8x16\\n', 'cat /wide.txt\\n', final=cells, pixels=pixels)
        self.assertIn('a', text)
        # In the 8x8 modes there is no wide font, so a character is one '?'.
        self.run_console('mode 80\\n', 'cat /wide.txt\\n',
                         final=[(0x10000000, ord('?')), (0x10000001, ord('?')), (0x10000002, ord('a')),
                                (0x10001000, 32), (0x10001001, 32)])

    def test_mixed_width_line_ends_in_8x16(self):
        cells = [(0x8000F003, 3)]

        def expect(x, y, byte, kind=0):
            offset = y * 80 + x
            cells.extend([(0x10000000 + offset, byte),
                          (0x10001000 + offset, 32 | kind << 6),
                          (0x10002000 + offset, 2)])

        def pair(x, y, character):
            expect(x, y, ord(character) >> 8, 1)
            expect(x + 1, y, ord(character) & 255, 2)

        # Exactly two cells: 日 fits; 本 and z follow on the next row.
        pair(78, 2, '日')
        pair(0, 3, '本')
        expect(2, 3, ord('z'))
        # One cell: blank the last column, then place both whole characters.
        expect(78, 5, ord('b'))
        expect(79, 5, ord(' '))
        pair(0, 6, '日')
        pair(2, 6, '本')
        expect(4, 6, ord('z'))
        # Zero cells: the final ASCII cell already wrapped to the next row.
        expect(78, 8, ord('b'))
        expect(79, 8, ord('c'))
        pair(0, 9, '日')
        pair(2, 9, '本')
        expect(4, 9, ord('z'))
        self.run_console('mode 8x16\\n', 'cat /edge.txt\\n', final=cells)

    def test_mode_scroll_uses_pixel_cell_height(self):
        for mode, hardware, low, high in [('40', 0, 144, 0), ('8x16', 3, 32, 1)]:
            with self.subTest(mode=mode):
                text = self.run_console(f'mode {mode}\\n', 'cat /scroll.txt\\n',
                                        final=[(0x8000F003, hardware),
                                               (0x8000F00E, low), (0x8000F00F, high)])
                self.assertIn('row', text)


if __name__ == '__main__':
    unittest.main()
