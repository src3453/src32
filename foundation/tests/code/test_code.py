"""libcode (conde-style character code conversion) on the host with ctypes, then actual WAL calls in Foundation U mode."""
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
LIB = ROOT / "foundation/lib/code"
CLANG = os.environ.get("CLANG") or shutil.which("clang")
OK, DATA, PARAMETER, INPUT_EOF, OUTPUT_EOF = 0, 1, 5, 6, 7
UTF8, UTF8_BOM, UTF16LE, UTF16BE, UTF32LE, UTF32BE = 0, 1, 2, 3, 4, 5
AUTO = "auto"
REPLACE, STRIP_BOM, ADD_BOM = 1, 2, 4
CODECS = {UTF8: "utf-8", UTF8_BOM: "utf-8", UTF16LE: "utf-16-le", UTF16BE: "utf-16-be",
          UTF32LE: "utf-32-le", UTF32BE: "utf-32-be"}
BOMS = {UTF8: b"", UTF8_BOM: b"\xef\xbb\xbf", UTF16LE: b"\xff\xfe", UTF16BE: b"\xfe\xff",
        UTF32LE: b"\xff\xfe\0\0", UTF32BE: b"\0\0\xfe\xff"}


NAMES = {UTF8: "utf8", UTF8_BOM: "utf8bom", UTF16LE: "utf16le", UTF16BE: "utf16be", UTF32LE: "utf32le", UTF32BE: "utf32be"}
EXPORTS = [f"cpt_code_{n}_to_utf32be" for n in NAMES.values() if n != "utf32be"] + \
          [f"cpt_code_utf32be_to_{n}" for n in NAMES.values() if n != "utf32be"] + ["cpt_code_bound", "cpt_code_detect"]


def run(command, cwd=None, timeout=300):
    result = subprocess.run(list(map(str, command)), cwd=cwd, capture_output=True,
                            text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if result.returncode:
        raise AssertionError(result.stdout + result.stderr)
    return result.stdout


def encode(text, enc, bom=False):
    return (BOMS[enc] if bom or enc == UTF8_BOM else b"") + text.encode(CODECS[enc])


def random_text(rng, n):
    pools = [(0x20, 0x7E), (0x80, 0x7FF), (0x800, 0xD7FF), (0xE000, 0xFFFD), (0x10000, 0x10FFFF)]
    return "".join(chr(rng.randint(*rng.choice(pools))) for _ in range(n))


@unittest.skipUnless(CLANG, "Clang required")
class HostTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="code-host-")
        cls.addClassCleanup(cls.temp.cleanup)
        path = Path(cls.temp.name) / ("code.dll" if os.name == "nt" else "code.so")
        command = [CLANG, "-shared", "-O2", "-I", ROOT / "foundation/include", LIB / "code.c", "-o", path]
        if os.name == "nt":
            for symbol in EXPORTS:
                command += ["-Xlinker", "/export:" + symbol]
        else:
            command += ["-fPIC"]
        run(command)
        cls.lib = ctypes.CDLL(str(path))
        ptr, u32 = ctypes.c_void_p, ctypes.c_uint32
        for name in EXPORTS:
            fn = getattr(cls.lib, name)
            if name.endswith("bound"):
                fn.argtypes, fn.restype = [u32, u32, u32], u32
            elif name.endswith("detect"):
                fn.argtypes, fn.restype = [ptr, u32, ptr], ctypes.c_int32
            else:
                fn.argtypes, fn.restype = [ptr, ptr, ptr, u32, u32], ctypes.c_int32

    def call(self, name, data, capacity, flags):
        # An odd offset checks that the byte-wise paths tolerate unaligned buffers.
        src = ctypes.create_string_buffer(b"\0" + data)
        out = ctypes.create_string_buffer(capacity + 1)
        n = ctypes.c_uint32(capacity)
        status = getattr(self.lib, name)(ctypes.addressof(out) + 1, ctypes.byref(n),
                                         ctypes.addressof(src) + 1, len(data), flags)
        self.assertLessEqual(n.value, capacity)
        if status:
            self.assertEqual(n.value, 0)
        return status, out.raw[1:1 + n.value]

    def convert(self, data, src_enc, dst_enc, flags=0, capacity=1 << 16):
        """Direct for UTF-32BE on either side, otherwise through UTF-32BE (BOM flags on the matching leg)."""
        if src_enc == AUTO:
            ptr = ctypes.create_string_buffer(data)
            bom = ctypes.c_uint32(0)
            src_enc = self.lib.cpt_code_detect(ptr, len(data), ctypes.byref(bom))
            data = data[bom.value:]
        if src_enc == dst_enc == UTF32BE:
            return OK, data
        if dst_enc == UTF32BE:
            return self.call(f"cpt_code_{NAMES[src_enc]}_to_utf32be", data, capacity, flags & ~ADD_BOM) \
                if flags & ADD_BOM == 0 else self._add32(data, src_enc, flags, capacity)
        if src_enc == UTF32BE:
            return self.call(f"cpt_code_utf32be_to_{NAMES[dst_enc]}", data, capacity, flags)
        status, mid = self.call(f"cpt_code_{NAMES[src_enc]}_to_utf32be", data, 1 << 16, flags & ~ADD_BOM)
        if status:
            return status, b""
        return self.call(f"cpt_code_utf32be_to_{NAMES[dst_enc]}", mid, capacity, flags & ~STRIP_BOM)

    def _add32(self, data, src_enc, flags, capacity):
        status, out = self.call(f"cpt_code_{NAMES[src_enc]}_to_utf32be", data, capacity, flags & ~ADD_BOM)
        return (status, out) if status else (OK, BOMS[UTF32BE] + out)

    def test_all_pairs_round_trip(self):
        rng = random.Random(3)
        texts = ["", "A", "\x00", "é", "あ", "\U0001F600", "﻿x", "퟿\U0010ffff"]
        texts += [random_text(rng, rng.randrange(1, 60)) for _ in range(60)]
        encodings = (UTF8, UTF8_BOM, UTF16LE, UTF16BE, UTF32LE, UTF32BE)
        for text in texts:
            for a in encodings:
                for b in encodings:
                    with self.subTest(text=text, a=a, b=b):
                        want = encode(text, b)
                        self.assertEqual(self.convert(encode(text, a), a, b), (OK, want))
                        n = self.lib.cpt_code_bound(len(encode(text, a)), a, b)
                        self.assertGreaterEqual(n, len(want))

    def test_bom(self):
        text = "aあ\U0001F600"
        for enc in CODECS:
            enc = UTF8_BOM if enc == UTF8 else enc  # ADD_BOM on UTF8 writes the same bytes as UTF8_BOM
            data = encode(text, enc, bom=True)
            self.assertEqual(self.convert(data, AUTO, UTF8), (OK, text.encode()))
            self.assertEqual(self.convert(text.encode(), UTF8, enc, ADD_BOM), (OK, data))
            ptr = ctypes.create_string_buffer(data)
            bom = ctypes.c_uint32(99)
            self.assertEqual(self.lib.cpt_code_detect(ptr, len(data), ctypes.byref(bom)), enc)
            self.assertEqual(bom.value, len(BOMS[enc]))
        self.assertEqual(self.convert(b"ab", AUTO, UTF16LE), (OK, "ab".encode("utf-16-le")))
        self.assertEqual(self.convert(b"\xef\xbb\xbfab", UTF8_BOM, UTF16BE), (OK, "ab".encode("utf-16-be")))
        self.assertEqual(self.convert(b"ab", UTF8_BOM, UTF8), (OK, b"ab"))
        # without a strip flag an explicit encoding keeps U+FEFF as a character
        self.assertEqual(self.convert(b"\xef\xbb\xbfab", UTF8, UTF8), (OK, b"\xef\xbb\xbfab"))
        self.assertEqual(self.convert(b"\xef\xbb\xbfab", UTF8, UTF8, STRIP_BOM), (OK, b"ab"))
        self.assertEqual(self.convert(b"\xff\xfea\0", UTF16LE, UTF8), (OK, b"\xef\xbb\xbfa"))
        self.assertEqual(self.convert(b"\xff\xfea\0", UTF16LE, UTF8, STRIP_BOM), (OK, b"a"))
        # FF FE 00 00 is UTF-32LE, as documented
        self.assertEqual(self.convert(b"\xff\xfe\0\0a\0\0\0", AUTO, UTF8), (OK, b"a"))
        self.assertEqual(self.convert(b"", AUTO, UTF8_BOM), (OK, b"\xef\xbb\xbf"))
        self.assertEqual(self.convert(b"\xef\xbb\xbfab", UTF8_BOM, UTF32BE), (OK, "ab".encode("utf-32-be")))
        self.assertEqual(self.convert(b"", UTF8, UTF32BE, ADD_BOM), (OK, BOMS[UTF32BE]))
        self.assertEqual(self.lib.cpt_code_detect(None, 0, None), UTF8)
        self.assertLess(self.lib.cpt_code_detect(None, 1, None), 0)

    def test_utf8_validation(self):
        bad = [b"\x80", b"\xc0\xaf", b"\xc1\xbf", b"\xe0\x80\x80", b"\xe0\x9f\xbf", b"\xed\xa0\x80",
               b"\xed\xbf\xbf", b"\xf0\x80\x80\x80", b"\xf4\x90\x80\x80", b"\xf5\x80\x80\x80", b"\xff",
               b"\xc2a", b"\xe2\x82a", b"\xf0\x9f\x98a"]
        good = ["\u0080", "߿", "ࠀ", "퟿", "", "￿", "\U00010000", "\U0010ffff"]
        for data in bad:
            with self.subTest(data=data):
                self.assertEqual(self.convert(data, UTF8, UTF32LE)[0], DATA)
        for text in good:
            self.assertEqual(self.convert(text.encode(), UTF8, UTF32LE), (OK, text.encode("utf-32-le")))
        for data in (b"\xc2", b"\xe2\x82", b"\xf0\x9f\x98", b"a\xf0"):
            self.assertEqual(self.convert(data, UTF8, UTF16LE), (INPUT_EOF, b""))

    def test_replace_matches_python(self):
        rng = random.Random(11)
        for enc in (UTF8, UTF16LE, UTF16BE, UTF32LE, UTF32BE):
            for _ in range(600):
                width = {UTF8: 1, UTF16LE: 2, UTF16BE: 2, UTF32LE: 4, UTF32BE: 4}[enc]
                text = random_text(rng, rng.randrange(0, 6))
                data = bytearray(encode(text, enc))
                for _ in range(rng.randrange(0, 4)):
                    action = rng.randrange(3)
                    if action == 0 and data:
                        data[rng.randrange(len(data))] = rng.choice((0, 0x80, 0xBF, 0xC0, 0xD8, 0xDC, 0xDF, 0xE0, 0xED, 0xF4, 0xFF))
                    elif action == 1 and data:
                        del data[rng.randrange(len(data)):]
                    else:
                        data[rng.randrange(len(data) + 1):0] = bytes(rng.randrange(256) for _ in range(rng.randrange(1, width + 2)))
                data = bytes(data)
                want = data.decode(CODECS[enc], errors="replace")
                with self.subTest(enc=enc, data=data):
                    status, got = self.convert(data, enc, UTF32LE, REPLACE)
                    self.assertEqual((status, got.decode("utf-32-le")), (OK, want))
                    strict = self.convert(data, enc, UTF32LE)
                    try:
                        data.decode(CODECS[enc])
                        self.assertEqual(strict[0], OK)
                    except UnicodeDecodeError:
                        self.assertIn(strict[0], (DATA, INPUT_EOF))

    def test_utf16_and_utf32_errors(self):
        self.assertEqual(self.convert(b"\x3d\xd8", UTF16LE, UTF8), (INPUT_EOF, b""))  # high surrogate alone
        self.assertEqual(self.convert(b"\x3d\xd8a\0", UTF16LE, UTF8), (DATA, b""))
        self.assertEqual(self.convert(b"\x00\xde", UTF16LE, UTF8), (DATA, b""))  # low surrogate alone
        self.assertEqual(self.convert(b"a\0b", UTF16LE, UTF8), (INPUT_EOF, b""))  # odd length
        self.assertEqual(self.convert(b"a\0b", UTF16LE, UTF8, REPLACE), (OK, b"a\xef\xbf\xbd"))
        self.assertEqual(self.convert(b"\0\xd8\0\0", UTF32LE, UTF8)[0], DATA)  # surrogate
        self.assertEqual(self.convert(b"\0\0\x11\0", UTF32LE, UTF8)[0], DATA)  # > U+10FFFF
        self.assertEqual(self.convert(b"a\0\0", UTF32LE, UTF8), (INPUT_EOF, b""))

    def test_output_limits(self):
        data = "aéあ\U0001F600".encode()
        for to in (UTF8_BOM, UTF16LE, UTF32BE):
            full = self.convert(data, UTF8, to)[1]
            for capacity in range(len(full)):
                self.assertEqual(self.convert(data, UTF8, to, capacity=capacity), (OUTPUT_EOF, b""))
            self.assertEqual(self.convert(data, UTF8, to, capacity=len(full)), (OK, full))

    def test_bound_covers_worst_cases(self):
        rng = random.Random(5)
        encodings = (UTF8, UTF16LE, UTF16BE, UTF32LE, UTF32BE)
        for _ in range(3000):
            a, b = rng.choice(encodings), rng.choice(encodings)
            data = bytes(rng.choice((0, 0x41, 0x80, 0xD8, 0xDC, 0xE3, 0xFF)) for _ in range(rng.randrange(0, 20)))
            bound = self.lib.cpt_code_bound(len(data), a, b)
            status, out = self.convert(data, a, b, REPLACE | ADD_BOM, capacity=bound)
            self.assertEqual(status, OK, (data, a, b))
            self.assertLessEqual(len(out), bound)
        self.assertEqual(self.lib.cpt_code_bound(0, UTF8, UTF8_BOM), 3)
        self.assertEqual(self.lib.cpt_code_bound(0xFFFFFFFF, UTF32LE, UTF8), 0xFFFFFFFF)
        self.assertEqual(self.lib.cpt_code_bound(1, 9, UTF8), 0xFFFFFFFF)

    def test_parameters(self):
        self.assertEqual(self.convert(b"a", UTF8, UTF32BE, 8)[0], PARAMETER)
        self.assertEqual(self.lib.cpt_code_bound(1, 9, UTF8), 0xFFFFFFFF)
        self.assertEqual(self.lib.cpt_code_bound(1, UTF8, 9), 0xFFFFFFFF)
        self.assertEqual(self.lib.cpt_code_utf8_to_utf32be(None, None, None, 0, 0), PARAMETER)
        n = ctypes.c_uint32(0)
        self.assertEqual(self.lib.cpt_code_utf8_to_utf32be(None, ctypes.byref(n), None, 1, 0), PARAMETER)
        self.assertEqual(self.lib.cpt_code_utf8_to_utf32be(None, ctypes.byref(n), None, 0, 0), OK)


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
        with tempfile.TemporaryDirectory(prefix="code-guest-") as directory:
            out = Path(directory)
            (out / "config.yaml").write_text("", encoding="utf-8")
            driver = [sys.executable, ROOT / "tools/llvm-src32/driver.py", "--clang", CLANG]
            for opt in ("-O1", "-O2"):
                with self.subTest(optimization=opt):
                    run([sys.executable, LIB / "build.py", out, "--clang", CLANG, "--optimization=" + opt])
                    run([*driver, HERE / "app.c", "-I", ROOT / "foundation/include", opt,
                         "--wax3", "--image-id", "code_test", "--import-wal", out / "libcode.wal",
                         "--origin", "0xC00000", "--stack-top", "0x1000000", "--user-stack-size", "0x240000",
                         "-o", out / "test.wax"])
                    lines = ["#include <stdint.h>"]
                    for name, path in {"code": out / "libcode.wal", "test": out / "test.wax"}.items():
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
