"""IRQ syscall/IPC regressions using real host-frame VSync. No inherited config.

python -m unittest discover -s foundation/tests/irq -v
Runs both VM selection and forced slots on cached and pipeline CPUs.
"""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/llvm-src32'))
import test_toolchain


class IrqTests(unittest.TestCase):
    def test_irq_delivery_ack_and_lifetime(self):
        test_toolchain.ToolchainTests.setUpClass()
        foundation = ROOT / 'foundation'
        sources = [foundation / 'tests/irq/user.c', foundation / 'tests/irq/main.c',
                   *sorted((foundation / 'kernel').glob('*.c'))]
        with tempfile.TemporaryDirectory(prefix='foundation-irq-') as tmp:
            work = Path(tmp)
            (work / 'config.yaml').write_text('memory:\n  ram_mib: 16\n', encoding='utf-8')
            for slot in (False, True):
                with self.subTest(slot=slot):
                    image = work / f'irq-{slot}.wax'
                    command = [sys.executable, str(ROOT / 'tools/llvm-src32/driver.py'),
                               '--clang', test_toolchain.ToolchainTests.clang, *map(str, sources),
                               '-I', str(ROOT / 'sdk/cpt32/include'),
                               '-I', str(foundation / 'include'), '-I', str(foundation / 'kernel'),
                               '--origin', '0x10000', '--stack-top', '0x100000', '-O1', '-o', str(image)]
                    if slot:
                        command += ['-D', 'FND_MM_SLOT_ONLY']
                    result = subprocess.run(command, capture_output=True, text=True, timeout=180)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    for cpu in ('cached', 'pipeline'):
                        with self.subTest(cpu=cpu):
                            result = subprocess.run(
                                [str(test_toolchain.ToolchainTests.harness), str(image), '42', '--cpu', cpu, '--frames', '90'],
                                cwd=work, capture_output=True, text=True, timeout=120)
                            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                            self.assertIn('PASS:', result.stdout)


if __name__ == '__main__':
    unittest.main()
