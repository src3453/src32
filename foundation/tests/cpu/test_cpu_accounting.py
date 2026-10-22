"""Real sched.c accounting against a deterministic MMIO clock (host CC)."""
import ctypes
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class CpuAccountingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='foundation-cpu-')
        lib = Path(cls.temp.name) / 'cpu.so'
        subprocess.run([os.environ.get('CC', 'cc'), '-shared', '-fPIC', '-O2',
                        '-I', str(ROOT / 'foundation/kernel'),
                        '-I', str(ROOT / 'foundation/include'),
                        '-I', str(ROOT / 'sdk/cpt32/include'),
                        str(Path(__file__).with_name('host.c')), '-o', str(lib)], check=True)
        cls.cpu = ctypes.CDLL(str(lib))
        for name in ('test_reset', 'test_clock', 'test_context', 'test_cycles'):
            getattr(cls.cpu, name).argtypes = [ctypes.c_uint32]
        cls.cpu.test_cycles.restype = ctypes.c_uint32
        cls.cpu.sched_cpu_snapshot.argtypes = [ctypes.POINTER(ctypes.c_uint32)]

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def snapshot(self):
        words = (ctypes.c_uint32 * 4)()
        self.cpu.sched_cpu_snapshot(words)
        return list(words)

    def test_idle_user_kernel_and_process_switch(self):
        self.cpu.test_reset(100)
        self.cpu.test_clock(1000)
        self.cpu.sched_cpu_enter()  # 900 idle, including HALT time
        self.cpu.test_clock(1100)
        self.cpu.test_context(0)
        self.cpu.sched_cpu_leave()  # 100 kernel
        self.cpu.test_clock(3100)
        self.cpu.sched_cpu_enter()  # 2000 user process 0
        self.cpu.test_clock(3200)
        self.assertEqual(self.snapshot(), [3100, 200, 900, 48000000])
        self.cpu.test_clock(3300)
        self.cpu.test_context(1)
        self.cpu.sched_cpu_leave()  # snapshot must not double-count kernel
        self.cpu.test_clock(3800)
        self.cpu.sched_cpu_enter()
        self.assertEqual(self.cpu.test_cycles(0), 2000)
        self.assertEqual(self.cpu.test_cycles(1), 500)
        self.assertEqual(self.snapshot(), [3700, 300, 900, 48000000])

    def test_clock_wrap_and_zero_elapsed(self):
        self.cpu.test_reset(0xfffffff0)
        self.cpu.test_context(0)
        self.cpu.test_clock(0x10)
        self.cpu.sched_cpu_enter()
        self.cpu.test_clock(0x20)
        self.assertEqual(self.snapshot(), [48, 16, 0, 48000000])
        self.assertEqual(self.cpu.test_cycles(0), 32)
        self.assertEqual(self.snapshot(), [48, 16, 0, 48000000])
        self.cpu.test_clock(0x30)
        self.cpu.sched_cpu_leave()
        self.assertEqual(self.snapshot(), [64, 32, 0, 48000000])

    def test_process_counter_wrap(self):
        self.cpu.test_reset(0)
        self.cpu.test_context(0)
        self.cpu.test_clock(0xfffffff0)
        self.cpu.sched_cpu_enter()
        self.assertEqual(self.cpu.test_cycles(0), 0xfffffff0)
        self.cpu.sched_cpu_leave()
        self.cpu.test_clock(0x10)
        self.cpu.sched_cpu_enter()
        self.assertEqual(self.cpu.test_cycles(0), 16)
        self.assertEqual(self.snapshot(), [16, 0, 0, 48000000])


if __name__ == '__main__':
    unittest.main()
