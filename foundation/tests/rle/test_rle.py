"""PackBits RLE (conde) on the host with ctypes, then actual WAL calls in Foundation U mode."""
import ctypes
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
LIB = ROOT / "foundation/lib/rle"
CLANG = os.environ.get("CLANG") or shutil.which("clang")
OK, DATA, PARAMETER, INPUT_EOF, OUTPUT_EOF = 0, 1, 5, 6, 7


def run(command, cwd=None, timeout=300):
    result = subprocess.run(list(map(str, command)), cwd=cwd, capture_output=True,
                            text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if result.returncode:
        raise AssertionError(result.stdout + result.stderr)
    return result.stdout


def reference_unpack(data):
    """Independent PackBits decoder (rejects 128)."""
    out, i = bytearray(), 0
    while i < len(data):
        c = data[i]
        i += 1
        if c < 128:
            out += data[i:i + c + 1]
            assert i + c + 1 <= len(data)
            i += c + 1
        else:
            assert c != 128
            out += bytes([data[i]]) * (257 - c)
            i += 1
    return bytes(out)


@unittest.skipUnless(CLANG, "Clang required")
class HostTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="rle-host-")
        cls.addClassCleanup(cls.temp.cleanup)
        path = Path(cls.temp.name) / ("rle.dll" if os.name == "nt" else "rle.so")
        command = [CLANG, "-shared", "-O2", "-I", ROOT / "foundation/include", LIB / "rle.c", "-o", path]
        if os.name == "nt":
            for symbol in ("cpt_rle_compress", "cpt_rle_decompress", "cpt_rle_compress_bound"):
                command += ["-Xlinker", "/export:" + symbol]
        else:
            command += ["-fPIC"]
        run(command)
        cls.lib = ctypes.CDLL(str(path))
        if os.name == "nt":
            import _ctypes
            cls.addClassCleanup(_ctypes.FreeLibrary, cls.lib._handle)
        ptr, u32 = ctypes.c_void_p, ctypes.c_uint32
        cls.lib.cpt_rle_compress.argtypes = [ptr, ptr, ptr, u32, ptr, u32, u32, u32]
        cls.lib.cpt_rle_decompress.argtypes = [ptr, ptr, ptr, u32, ptr, u32]
        cls.lib.cpt_rle_compress_bound.argtypes = [u32]
        cls.lib.cpt_rle_compress_bound.restype = u32

    def call(self, data, compress, capacity=1 << 16, level=0, param=0):
        # An odd offset checks that the word-wise paths tolerate unaligned buffers.
        src = ctypes.create_string_buffer(b"\0" + data)
        out = ctypes.create_string_buffer(capacity + 1)
        n = ctypes.c_uint32(capacity)
        src_p = ctypes.addressof(src) + 1
        out_p = ctypes.addressof(out) + 1
        if compress:
            status = self.lib.cpt_rle_compress(out_p, ctypes.byref(n), src_p, len(data), None, 0, level, param)
        else:
            status = self.lib.cpt_rle_decompress(out_p, ctypes.byref(n), src_p, len(data), None, 0)
        self.assertLessEqual(n.value, capacity)
        return status, out.raw[1:1 + n.value]

    def roundtrip(self, data):
        status, packed = self.call(data, True)
        self.assertEqual(status, OK)
        self.assertEqual(reference_unpack(packed), data)
        self.assertLessEqual(len(packed), self.lib.cpt_rle_compress_bound(len(data)))
        self.assertEqual(self.call(packed, False), (OK, data))
        return packed

    def test_vectors(self):
        self.assertEqual(self.call(b"", True), (OK, b""))
        self.assertEqual(self.call(b"x", True), (OK, b"\x00x"))
        self.assertEqual(self.call(b"aaaaabc", True), (OK, bytes([252]) + b"a\x01bc"))
        self.assertEqual(self.call(b"aabb", True), (OK, b"\x03aabb"))  # 2 byte runs stay literal
        self.assertEqual(self.call(b"a" * 128, True), (OK, b"\x81a"))
        self.assertEqual(self.call(b"a" * 130, True), (OK, b"\x81a\xffa"))  # continuation of a capped run
        self.assertEqual(self.call(bytes(range(130)), True)[1][:1], b"\x7f")  # literal capped at 128
        self.assertEqual(self.call(b"\x01\x02\x03" + b"\0" * 200 + b"\x04", True)[1][:5], b"\x02\x01\x02\x03\x81")

    def test_round_trips(self):
        rng = random.Random(7)
        samples = [bytes(range(256)) * 3, b"\0" * 1000, b"ab" * 300, os.urandom(2048)]
        for n in range(0, 40):
            samples += [bytes([5]) * n, bytes(rng.choice(b"ab") for _ in range(n))]
        for _ in range(400):
            parts = [bytes([rng.randrange(4)]) * rng.choice((1, 2, 3, 4, 5, 127, 128, 129, 300)) for _ in range(rng.randrange(8))]
            samples.append(b"".join(parts))
        for data in samples:
            with self.subTest(size=len(data)):
                self.roundtrip(data)

    def test_incompressible_bound(self):
        data = bytes(range(256)) * 4  # no runs: exactly one header per 128 bytes
        packed = self.roundtrip(data)
        self.assertEqual(len(packed), len(data) + 8)
        self.assertEqual(self.lib.cpt_rle_compress_bound(len(data)), len(data) + 8)
        self.assertEqual(self.lib.cpt_rle_compress_bound(0), 0)
        self.assertEqual(self.lib.cpt_rle_compress_bound(0xFFFFFFFF), 0xFFFFFFFF)
        self.assertEqual(self.lib.cpt_rle_compress_bound(0xFE000000), 0xFE000000 + 0xFE000000 // 128)

    def test_limits_and_bad_streams(self):
        data = b"abc" + b"z" * 300 + b"def"
        packed = self.roundtrip(data)
        for capacity in range(len(packed)):
            status, out = self.call(data, True, capacity=capacity)
            self.assertEqual((status, out), (OUTPUT_EOF, b""))
        self.assertEqual(self.call(data, True, capacity=len(packed))[0], OK)
        for capacity in (0, 5, len(data) - 1):
            self.assertEqual(self.call(packed, False, capacity=capacity), (OUTPUT_EOF, b""))
        self.assertEqual(self.call(packed, False, capacity=len(data))[0], OK)
        self.assertEqual(self.call(b"\x80", False), (DATA, b""))
        self.assertEqual(self.call(b"\x00a\x80", False), (DATA, b""))
        self.assertEqual(self.call(b"\xff", False), (INPUT_EOF, b""))
        self.assertEqual(self.call(b"\x03abc", False), (INPUT_EOF, b""))
        self.assertEqual(self.call(packed[:-1], False)[0], INPUT_EOF)
        self.assertEqual(self.call(b"x", True, level=10), (PARAMETER, b""))
        self.assertEqual(self.call(b"x", True, param=1), (PARAMETER, b""))
        self.assertEqual(self.lib.cpt_rle_compress(None, None, None, 0, None, 0, 0, 0), PARAMETER)
        self.assertEqual(self.lib.cpt_rle_decompress(None, None, None, 0, None, 0), PARAMETER)


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
        with tempfile.TemporaryDirectory(prefix="rle-guest-") as directory:
            out = Path(directory)
            (out / "config.yaml").write_text("", encoding="utf-8")
            driver = [sys.executable, ROOT / "tools/llvm-src32/driver.py", "--clang", CLANG]
            for opt in ("-O1", "-O2"):
                with self.subTest(optimization=opt):
                    run([sys.executable, LIB / "build.py", out, "--clang", CLANG, "--optimization=" + opt])
                    run([sys.executable, ROOT / "foundation/lib/lzma/build.py", out, "--clang", CLANG,
                         "--optimization=" + opt])
                    run([*driver, HERE / "app.c", "-I", ROOT / "foundation/include", opt,
                         "--wax3", "--image-id", "rle_test",
                         "--import-wal", out / "librle.wal", "--import-wal", out / "liblzma.wal",
                         "--origin", "0xC00000", "--stack-top", "0x1000000", "--user-stack-size", "0x240000",
                         "-o", out / "test.wax"])
                    lines = ["#include <stdint.h>"]
                    for name, path in {"rle": out / "librle.wal", "lzma": out / "liblzma.wal",
                                       "test": out / "test.wax"}.items():
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
