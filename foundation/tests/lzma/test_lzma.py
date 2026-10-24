"""SDK interoperability on the host and actual WAL calls in Foundation U mode."""
import ctypes
import lzma
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
LIB = ROOT / "foundation/lib/lzma"
CLANG = os.environ.get("CLANG") or shutil.which("clang")
if not CLANG and Path("C:/Program Files/AMD/ROCm/6.4/bin/clang.exe").exists():
    CLANG = "C:/Program Files/AMD/ROCm/6.4/bin/clang.exe"


def run(command, cwd=None, timeout=300):
    result = subprocess.run(list(map(str, command)), cwd=cwd, capture_output=True,
                            text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if result.returncode:
        raise AssertionError(result.stdout + result.stderr)
    return result.stdout


@unittest.skipUnless(CLANG, "Clang required")
class HostTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="lzma-host-")
        cls.addClassCleanup(cls.temp.cleanup)
        path = Path(cls.temp.name) / ("lzma.dll" if os.name == "nt" else "lzma.so")
        command = [CLANG, "-shared", "-O2", "-DZ7_ST", "-I", ROOT / "foundation/include",
                   "-I", LIB / "vendor", LIB / "lzma.c",
                   *(LIB / "vendor" / name for name in ("LzmaEnc.c", "LzmaDec.c", "LzFind.c", "CpuArch.c")),
                   "-o", path]
        if os.name == "nt":
            for symbol in ("cpt_lzma_compress", "cpt_lzma_decompress"):
                command += ["-Xlinker", "/export:" + symbol]
        else:
            command += ["-fPIC"]
        run(command)
        cls.lib = ctypes.CDLL(str(path))
        if os.name == "nt":
            # Windows cannot unlink a loaded DLL; release before temp cleanup.
            import _ctypes
            cls.addClassCleanup(_ctypes.FreeLibrary, cls.lib._handle)
        ptr = ctypes.c_void_p
        cls.lib.cpt_lzma_compress.argtypes = [ptr, ptr, ptr, ctypes.c_uint32, ptr,
                                             ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
        cls.lib.cpt_lzma_decompress.argtypes = [ptr, ptr, ptr, ctypes.c_uint32, ptr, ctypes.c_uint32]

    def call(self, data, compress=False, capacity=16384, workspace=2 * 1024 * 1024, level=1, dictionary=4096):
        src = ctypes.create_string_buffer(data)
        out = ctypes.create_string_buffer(capacity)
        work = (ctypes.c_uint64 * ((workspace + 7) // 8))()
        n = ctypes.c_uint32(capacity)
        args = [out, ctypes.byref(n), src, len(data), work, workspace]
        if compress:
            args += [level, dictionary]
        function = self.lib.cpt_lzma_compress if compress else self.lib.cpt_lzma_decompress
        result = function(*args)
        self.assertLessEqual(n.value, capacity)
        return result, out.raw[:n.value]

    def test_interoperability(self):
        for data in (b"", b"x", b"Foundation " * 100, bytes(range(256)) * 4, os.urandom(1024)):
            for level in (0, 1, 5, 9):
                with self.subTest(size=len(data), level=level):
                    status, packed = self.call(data, True, level=level)
                    self.assertEqual(status, 0)
                    self.assertEqual(lzma.decompress(packed, format=lzma.FORMAT_ALONE), data)
                    self.assertEqual(int.from_bytes(packed[5:13], "little"), len(data))
                    self.assertEqual(self.call(packed), (0, data))
            packed = lzma.compress(data, format=lzma.FORMAT_ALONE)
            self.assertEqual(self.call(packed), (0, data))

    def test_limits_and_bad_streams(self):
        packed = lzma.compress(b"Foundation " * 100, format=lzma.FORMAT_ALONE)
        for data in (b"", packed[:12], packed[:-3], packed + b"garbage", bytes([255]) + packed[1:]):
            status, output = self.call(data)
            self.assertNotEqual(status, 0)
            self.assertEqual(output, b"")
        self.assertNotEqual(self.call(packed, capacity=8)[0], 0)
        self.assertEqual(self.call(packed, workspace=8), (2, b""))
        self.assertEqual(self.call(b"x", True, workspace=8), (2, b""))
        self.assertEqual(self.call(b"x", True, capacity=12), (7, b""))
        self.assertEqual(self.call(b"x", True, level=10), (5, b""))
        self.assertEqual(self.call(b"x", True, dictionary=1), (5, b""))


@unittest.skipUnless(CLANG, "Clang required")
class GuestTests(unittest.TestCase):
    def test_wal_in_foundation(self):
        suffix = ".exe" if os.name == "nt" else ""
        default_bench = ROOT / ("target/release/src32_testbench" + suffix)
        if not default_bench.exists():
            default_bench = ROOT / ("target/debug/src32_testbench" + suffix)
        bench = Path(os.environ.get("SRC32_TESTBENCH", default_bench))
        if not bench.exists():
            self.skipTest("src32_testbench required")
        with tempfile.TemporaryDirectory(prefix="lzma-guest-") as directory:
            out = Path(directory)
            (out / "config.yaml").write_text("", encoding="utf-8")
            plain = b"Foundation SDK interoperability. " * 4
            fixture = lzma.compress(plain, format=lzma.FORMAT_ALONE)
            def array(name, data):
                return f"static const unsigned char {name}[] = {{{','.join(map(str, data))}}};\n"
            (out / "fixtures.h").write_text(array("fixture", fixture) + array("plain", plain), encoding="utf-8")
            driver = [sys.executable, ROOT / "tools/llvm-src32/driver.py", "--clang", CLANG]
            for opt in ("-O1", "-O2"):
                with self.subTest(optimization=opt):
                    run([sys.executable, ROOT / "examples/lzma-wal/build.py", out, "--clang", CLANG,
                         "--optimization=" + opt])
                    run([*driver, HERE / "app.c", "-I", ROOT / "foundation/include", "-I", out, opt,
                         "--wax3", "--image-id", "lzma_test", "--import-wal", out / "liblzma.wal",
                         "--origin", "0xC00000", "--stack-top", "0x1000000", "--user-stack-size", "0x240000",
                         "-o", out / "test.wax"])
                    images = {"library": out / "liblzma.wal", "test": out / "test.wax", "demo": out / "lzma-demo.wax"}
                    lines = ["#include <stdint.h>"]
                    for name, path in images.items():
                        data = path.read_bytes()
                        words = data + bytes(-len(data) % 4)
                        text = ",".join(hex(int.from_bytes(words[i:i+4], "big")) + "u" for i in range(0, len(words), 4))
                        lines += [f"#define img_{name}_size {len(data)}u", f"static const uint32_t img_{name}[] = {{{text}}};"]
                    (out / "images.h").write_text("\n".join(lines), encoding="utf-8")
                    run([*driver, HERE / "kernel.c", *(ROOT / "foundation/kernel" / name for name in
                         ("sched.c", "proc.c", "mm.c", "trap.c", "ipc.c", "fb.c", "sgu.c", "sgc.c", "pec.c", "blk.c", "wax.c")),
                         "-I", ROOT / "foundation/include", "-I", ROOT / "sdk/cpt32/include", "-I", out,
                         "-O1", "--origin", "0x10000", "--stack-top", "0x100000", "-o", out / "kernel.wax"])
                    for cpu in ("scalar", "pipeline"):
                        self.assertIn("PASS:", run([bench.resolve(), out / "kernel.wax", "42", "--cpu", cpu,
                                                   "--frames", "1200"], cwd=out, timeout=300))


if __name__ == "__main__":
    unittest.main()
