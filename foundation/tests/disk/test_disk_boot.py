"""Foundation disk boot on src32_testbench (doc/spec_Foundation.md §11).

Builds foundation.img with foundation/boot/build.py and boots it through the
BIOS (doc/spec_BIOS.md): IDC boot sector -> stage1 -> FNDBOOT -> kernel ->
the console, disk, fs and init modules -> /bin/shell.wax from the disk. The testbench program is a 4-byte zero word, so the BIOS
skips the RAM boot (bios/README.md). The kernel stops the machine at
STOP_TICKS with R1 = 42; stage1 HALTs with R1 = its failed step.

    python -m unittest discover -s foundation/tests/disk -v

CLANG and SRC32_TESTBENCH override executable discovery.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / 'foundation/boot'))
sys.path.insert(0, str(ROOT / 'tools/fs2'))
import fndboot  # noqa: E402
import fs2  # noqa: E402

STOP_TICKS = 600  # 6 seconds at 100 Hz
# The kernel starts after the 120-frame POST hold and the HDD read (~frame
# 125) and stops STOP_TICKS later (360 frames at 60 Hz). fs checks the volume
# before init can load /bin/shell.wax, so the banner appears ~115 frames later.
FRAMES = 560
TYPE_FRAME = 320   # the shell has printed its banner by then
FAIL_FRAMES = 200  # stage1 halts right after the POST hold
BANNER = ['Foundation OS (kernel ABI 1.0)', 'Foundation shell 1.0']


def find_testbench():
    suffix = '.exe' if os.name == 'nt' else ''
    if os.environ.get('SRC32_TESTBENCH'):
        return Path(os.environ['SRC32_TESTBENCH'])
    for profile in ('release', 'debug'):
        candidate = ROOT / 'target' / profile / ('src32_testbench' + suffix)
        if candidate.is_file():
            return candidate
    raise RuntimeError('Build cargo build --bin src32_testbench or set SRC32_TESTBENCH')


class DiskBootTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.harness = find_testbench()
        cls.temp = tempfile.TemporaryDirectory(prefix='foundation-disk-')
        cls.work = Path(cls.temp.name)
        cls.build = cls.work / 'build'
        command = [sys.executable, str(ROOT / 'foundation/boot/build.py'), str(cls.build),
                   '--stop-ticks', str(STOP_TICKS)]
        clang = os.environ.get('CLANG') or shutil.which('clang')
        if clang:
            command += ['--clang', clang]
        result = subprocess.run(command, capture_output=True, text=True, encoding='utf-8',
                                errors='replace', timeout=600)
        if result.returncode != 0:
            raise RuntimeError(result.stdout + result.stderr)
        cls.image = (cls.build / 'foundation.img').read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def boot(self, image, expected, frames, *extra, slot=0, kind='hdd'):
        """Boots image from IDC slot; returns the PCG screen without trailing blank rows."""
        run = Path(tempfile.mkdtemp(dir=self.work))
        (run / 'disk.img').write_bytes(image)
        (run / 'zero.bin').write_bytes(bytes(4))
        bios = json.dumps(str(ROOT / 'assets/bios.rom'))
        (run / 'config.yaml').write_text(
            f'memory:\n  ram_mib: 16\nbios:\n  image: {bios}\n'
            f'disk:\n  - slot: {slot}\n    type: "{kind}"\n    image: "disk.img"\n    read_only: false\n',
            encoding='utf-8')
        command = [str(self.harness), 'zero.bin', str(expected), '--cpu', 'pipeline',
                   '--frames', str(frames), '--dump-pcg', *extra]
        result = subprocess.run(command, cwd=run, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=600)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        rows = lines[lines.index('PCG SCREEN BEGIN') + 1:lines.index('PCG SCREEN END')]
        while rows and not rows[-1]:
            rows.pop()
        return rows

    def test_boots_the_shell_from_disk(self):
        # Slot 1 with dd0 empty: stage1 must use the drive the BIOS passed in R1.
        rows = self.boot(self.image, 42, FRAMES, '--type', f'{TYPE_FRAME}=uname\\n', slot=1)
        self.assertEqual(rows, BANNER + ['/> uname', BANNER[0], '/>'])

    def test_image_layout(self):
        volume = fs2.Volume(bytearray(self.image))
        self.assertEqual(fs2.check_image(bytearray(self.image)), [])
        sb = volume.sb
        boot = self.image[sb['boot_start'] * fs2.BLOCK:][:sb['boot_size']]
        self.assertEqual(boot, (self.build / 'fndboot.bin').read_bytes())
        self.assertEqual(self.image[:fs2.BLOCK], (self.build / 'stage1.bin').read_bytes())
        header, modules = fndboot.unpack(boot)
        kernel = (self.build / 'kernel.bin').read_bytes()
        self.assertEqual((header['magic'], header['version'], header['size']), (fndboot.MAGIC, 1, len(boot)))
        self.assertEqual((header['kernel_load'], header['kernel_entry']), (0x10000, 0x10000))
        self.assertEqual(boot[header['kernel_off']:][:header['kernel_size']], kernel)
        self.assertEqual([m['name'] for m in modules], ['console', 'disk', 'fs', 'init'])  # init last (§11)
        for module in modules:
            image = (self.build / f"{module['name']}.wax").read_bytes()
            self.assertEqual(boot[module['offset']:][:module['size']], image)
        self.assertEqual([m['caps'] for m in modules], [8, 12, 8, 0])  # SVC for the services, DISK for disk
        self.assertEqual(len(modules[0]['grants']), 3)  # the console's MMIO windows
        for path in ('/bin/shell.wax', '/bin/echo.wax', '/lib/fnd.1.wal'):
            inode = volume.resolve(path)
            self.assertEqual(volume.read(inode), (self.build / 'root' / path[1:]).read_bytes())
            self.assertEqual(inode['mode'], 0x003D)

    def corrupt(self, offset, mask):
        image = bytearray(self.image)
        image[offset] ^= mask
        return bytes(image)

    def test_stage1_rejects_a_dirty_volume(self):
        rows = self.boot(self.corrupt(fs2.BLOCK + 7, fs2.FLAG_DIRTY), 2, FAIL_FRAMES)
        self.assertNotIn(BANNER[0], rows)

    def test_stage1_rejects_bad_superblock_layout(self):
        # bitmap_start no longer equals 2 + boot_blocks.
        rows = self.boot(self.corrupt(fs2.BLOCK + 0x1B, 1), 2, FAIL_FRAMES)
        self.assertNotIn(BANNER[0], rows)

    def test_stage1_rejects_a_bad_fndboot_magic(self):
        rows = self.boot(self.corrupt(fs2.BOOT_START * fs2.BLOCK, 0xFF), 3, FAIL_FRAMES)
        self.assertNotIn(BANNER[0], rows)

    def test_stage1_rejects_fndboot_size_mismatch(self):
        rows = self.boot(self.corrupt(fs2.BOOT_START * fs2.BLOCK + 11, 4), 3, FAIL_FRAMES)
        self.assertNotIn(BANNER[0], rows)

    def test_kernel_rejects_a_bad_module_table(self):
        # A non-zero reserved header word: stage1 boots the kernel, whose
        # fnd_boot_check returns FND_E_FORMAT (-12) from main, so it HALTs.
        rows = self.boot(self.corrupt(fs2.BOOT_START * fs2.BLOCK + 40, 1), (-12) & 0xFFFFFFFF, FAIL_FRAMES)
        self.assertNotIn(BANNER[0], rows)


if __name__ == '__main__':
    unittest.main()
