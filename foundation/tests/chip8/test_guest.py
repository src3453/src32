"""Shell -> CHIP-8 WAX -> fb/SGU, plus real held/released PeC input.

Build cargo build --release --bin src32_testbench before running this suite.
Run again with FND_MM=slot. Temporary files must be writable on Windows.
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

class GuestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        test_shell.ShellTests.setUpClass.__func__(cls)
        cls.addClassCleanup(cls.temp.cleanup)
        out = cls.work / 'devices.wax'
        command = [sys.executable, ROOT / 'tools/llvm-src32/driver.py', HERE / 'devices.c',
                   '-I', ROOT / 'foundation/include', '-I', ROOT / 'sdk/cpt32/include',
                   '-O1', '--wax3', '--page-align', '--image-id', 'test.chip8.devices',
                   '--import-wal', cls.work / 'root/lib/fnd.1.wal',
                   '--origin', '0x800000', '--stack-top', '0xC00000', '-o', out]
        if os.environ.get('CLANG'):
            command += ['--clang', os.environ['CLANG']]
        subprocess.run(list(map(str, command)), check=True, capture_output=True, text=True, timeout=120)
        vol = test_shell.fs2.Volume(bytearray(cls.disk))
        vol.put('/bin/chipdev.wax', out.read_bytes(), mode=0x3D)
        # Draw the built-in zero, beep, wait for key+release, then unsupported SYS.
        vol.put('/demo.ch8', bytes.fromhex('00e0 6000 f029 6100 6200 d125 60ff f018 f00a 0123'), mode=0x24)
        vol.put('/bad.ch8', bytes.fromhex('0123'), mode=0x24)
        vol.put('/empty.ch8', b'', mode=0x24)
        vol.put('/big.ch8', bytes(3585), mode=0x24)
        sample = cls.work / 'sample.ch8'
        subprocess.run([sys.executable, str(ROOT / 'examples/foundation-chip8/demo.py'), str(sample)],
                       check=True, capture_output=True, text=True)
        vol.put('/sample.ch8', sample.read_bytes(), mode=0x24)
        cls.disk = bytes(vol.img)

    def run_guest(self, batches, final=(), keys=(), frames=660, cpu='cached', running=False):
        run = self.work / self.id().split('.')[-1]
        run.mkdir()
        (run / 'disk.img').write_bytes(self.disk)
        (run / 'config.yaml').write_text('memory:\n  ram_mib: 16\n  vram_mib: 4\n'
            'disk:\n  - slot: 0\n    type: hdd\n    image: disk.img\n    read_only: false\n', encoding='utf-8')
        command = [str(self.harness), str(self.image), '42', '--frames', str(frames),
                   '--cpu', os.environ.get('FOUNDATION_TEST_CPU', cpu), '--dump-pcg']
        if running: command += ['--allow-running', '--profile-cpu']
        for frame, text in batches: command += ['--type', f'{frame}={text}']
        for key in keys: command += ['--key', key]
        for address, value in final: command += ['--expect-final-mem-u8', f'{address:#x}={value}']
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_render_and_sgu_while_waiting(self):
        # Stop before the kernel test halt: observe active graphics and SGU state.
        self.run_guest([(30, 'chip8 /demo.ch8\\n')], frames=260, running=True,
                       final=[(0x10000000 + 40*320, 1), (0x10000000 + 40*320 + 20, 0),
                              (0x10000000, 0), (0x80020800, 1), (0x80020801, 0xB8),
                              (0x80020803, 128), (0x80020000, 0xE0), (0x80020080, 0x20)])

    def test_sample_rom_maps_v_to_hex_f(self):
        self.run_guest([(30, 'chip8 /sample.ch8\\n'), (240, 'V')], frames=300, running=True,
                       final=[(0x10000000 + 110*320 + 150, 1),
                              (0x10000000 + 110*320 + 170, 0),
                              (0x10000000 + 115*320 + 150, 1),
                              (0x10000000 + 115*320 + 155, 0), (0x80020803, 0)])

    def test_key_release_exits_wait_and_cleans_up(self):
        text = self.run_guest([(30, 'chip8 /demo.ch8\\n'), (320, 'x'),
                               (360, 'echo result=$?\\n')], final=[(0x80020803, 0)])
        self.assertIn('result=-12', text)

    def test_escape(self):
        text = self.run_guest([(30, 'chip8 /demo.ch8\\n'), (280, '\\e'),
                              (320, 'echo escaped=$?\\n')], final=[(0x80020803, 0)])
        self.assertIn('escaped=0', text)

    def test_rom_errors(self):
        text = self.run_guest([(30, 'chip8 /empty.ch8\\necho empty=$?\\n'),
                              (120, 'chip8 /big.ch8\\necho big=$?\\n'),
                              (210, 'chip8 /bad.ch8\\n'),
                              (270, 'echo bad=$?\\n'),
                              (330, 'chip8 /absent.ch8\\necho missing=$?\\n')])
        for expected in ('empty=-12', 'big=-6', 'bad=-12', 'missing=-10'):
            self.assertIn(expected, text)

    def test_devices_and_held_keys(self):
        text = self.run_guest([(30, 'chipdev keys\\n'), (360, 'echo devices=$?\\n')],
                              keys=['240:30:1','240:31:1','270:30:0','290:31:0'])
        self.assertIn('chip8 devices passed', text)
        self.assertIn('devices=0', text)

    def test_exit_and_fault_release_sgu(self):
        text = self.run_guest([(30, 'chipdev leak\\necho leak=$?\\nchipdev fault\\necho fault=$?\\n'
                                   'chipdev\\necho devices=$?\\n')], cpu='pipeline',
                              final=[(0x80020803, 0), (0x800218E3, 0)])
        self.assertIn('leak=0', text)
        self.assertRegex(text, r'fault=-\d+')
        self.assertIn('chip8 devices passed', text)
        self.assertIn('devices=0', text)
