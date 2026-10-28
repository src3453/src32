"""RFC 8439 ChaCha20 and Poly1305 vectors on host and in Foundation U mode."""
import ctypes
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
LIB = ROOT / "foundation/lib/crypt"
CLANG = os.environ.get("CLANG") or shutil.which("clang")
OK, DATA, PARAMETER, OUTPUT_EOF = 0, 1, 5, 7


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
        cls.temp = tempfile.TemporaryDirectory(prefix="crypt-host-")
        cls.addClassCleanup(cls.temp.cleanup)
        path = Path(cls.temp.name) / ("crypt.dll" if os.name == "nt" else "crypt.so")
        command = [CLANG, "-shared", "-O2", "-I", ROOT / "foundation/include", LIB / "crypt.c", "-o", path]
        if os.name == "nt":
            for name in ("cpt_chacha20_encrypt", "cpt_chacha20_decrypt", "cpt_chacha20_encrypt_bound",
                         "cpt_poly1305_auth", "cpt_poly1305_verify"):
                command += ["-Xlinker", "/export:" + name]
        else:
            command += ["-fPIC"]
        run(command)
        cls.lib = ctypes.CDLL(str(path))
        if os.name == "nt":
            import _ctypes
            cls.addClassCleanup(_ctypes.FreeLibrary, cls.lib._handle)
        ptr, u32 = ctypes.c_void_p, ctypes.c_uint32
        for name in ("cpt_chacha20_encrypt", "cpt_chacha20_decrypt"):
            fn = getattr(cls.lib, name)
            fn.argtypes, fn.restype = [ptr, ptr, ptr, u32, ptr, ptr, u32], ctypes.c_int32
        cls.lib.cpt_chacha20_encrypt_bound.argtypes = [u32]
        cls.lib.cpt_chacha20_encrypt_bound.restype = u32
        cls.lib.cpt_poly1305_auth.argtypes = [ptr, ptr, ptr, u32, ptr]
        cls.lib.cpt_poly1305_auth.restype = ctypes.c_int32
        cls.lib.cpt_poly1305_verify.argtypes = [ptr, ptr, u32, ptr]
        cls.lib.cpt_poly1305_verify.restype = ctypes.c_int32

    def test_rfc8439_vectors(self):
        key = bytes(range(32))
        nonce = bytes.fromhex("000000090000004a00000000")
        expected = bytes.fromhex(
            "10f1e7e4d13b5915500fdd1fa32071c4"
            "c7d1f4c733c068030422aa9ac3d46c4e"
            "d2826446079faa0914c2d705d98b02a2"
            "b5129cd1de164eb9cbd083e8a2503c4e")
        source = ctypes.create_string_buffer(bytes(64))
        output = ctypes.create_string_buffer(64)
        size = ctypes.c_uint32(64)
        self.assertEqual(self.lib.cpt_chacha20_encrypt(output, ctypes.byref(size), source, 64,
                                                       key, nonce, 1), OK)
        self.assertEqual((size.value, output.raw), (64, expected))
        back = ctypes.create_string_buffer(64)
        size.value = 64
        self.assertEqual(self.lib.cpt_chacha20_decrypt(back, ctypes.byref(size), output, 64,
                                                       key, nonce, 1), OK)
        self.assertEqual(back.raw, bytes(64))

        poly_key = bytes.fromhex("85d6be7857556d337f4452fe42d506a8"
                                 "0103808afb0db2fd4abff6af4149f51b")
        message = b"Cryptographic Forum Research Group"
        expected_tag = bytes.fromhex("a8061dc1305136c6c22b8baf0c0127a9")
        msg = ctypes.create_string_buffer(message)
        tag = ctypes.create_string_buffer(16)
        size.value = 16
        self.assertEqual(self.lib.cpt_poly1305_auth(tag, ctypes.byref(size), msg, len(message), poly_key), OK)
        self.assertEqual((size.value, tag.raw), (16, expected_tag))
        self.assertEqual(self.lib.cpt_poly1305_verify(expected_tag, msg, len(message), poly_key), OK)
        bad_tag = bytes([expected_tag[0] ^ 1]) + expected_tag[1:]
        self.assertEqual(self.lib.cpt_poly1305_verify(bad_tag, msg, len(message), poly_key), DATA)

    def test_buffer_contract_and_counter_wrap(self):
        key, nonce = bytes(32), bytes(12)
        src, out = ctypes.create_string_buffer(b"abc"), ctypes.create_string_buffer(3)
        size = ctypes.c_uint32(2)
        self.assertEqual(self.lib.cpt_chacha20_encrypt(out, ctypes.byref(size), src, 3, key, nonce, 0), OUTPUT_EOF)
        self.assertEqual(size.value, 0)
        size.value = 3
        self.assertEqual(self.lib.cpt_chacha20_encrypt(out, ctypes.byref(size), src, 3, key, nonce,
                                                       0xffffffff), OK)
        size.value = 65
        large_src, large_out = ctypes.create_string_buffer(65), ctypes.create_string_buffer(65)
        self.assertEqual(self.lib.cpt_chacha20_encrypt(large_out, ctypes.byref(size), large_src, 65,
                                                       key, nonce, 0xffffffff), PARAMETER)
        self.assertEqual(size.value, 0)
        self.assertEqual(self.lib.cpt_chacha20_encrypt_bound(0xffffffff), 0xffffffff)

        msg, tag = ctypes.create_string_buffer(b"x"), ctypes.create_string_buffer(16)
        size.value = 15
        self.assertEqual(self.lib.cpt_poly1305_auth(tag, ctypes.byref(size), msg, 1, key), OUTPUT_EOF)
        self.assertEqual(size.value, 0)


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
        with tempfile.TemporaryDirectory(prefix="crypt-guest-") as directory:
            out = Path(directory)
            (out / "config.yaml").write_text("", encoding="utf-8")
            run([sys.executable, LIB / "build.py", out, "--clang", CLANG, "--optimization=-O2"])
            driver = [sys.executable, ROOT / "tools/llvm-src32/driver.py", "--clang", CLANG]
            run([*driver, HERE / "app.c", "-I", ROOT / "foundation/include", "-O1",
                 "--wax3", "--image-id", "crypt_test", "--import-wal", out / "libcrypt.wal",
                 "--origin", "0xC00000", "--stack-top", "0x1000000", "--user-stack-size", "0x240000",
                 "-o", out / "test.wax"])
            lines = ["#include <stdint.h>"]
            for name, path in {"crypt": out / "libcrypt.wal", "test": out / "test.wax"}.items():
                data = path.read_bytes()
                words = data + bytes(-len(data) % 4)
                values = ",".join(hex(int.from_bytes(words[i:i + 4], "big")) + "u"
                                   for i in range(0, len(words), 4))
                lines += [f"#define img_{name}_size {len(data)}u",
                          f"static const uint32_t img_{name}[] = {{{values}}};"]
            (out / "images.h").write_text("\n".join(lines), encoding="utf-8")
            sources = [ROOT / "foundation/kernel" / name for name in
                       ("sched.c", "proc.c", "mm.c", "trap.c", "ipc.c", "fb.c", "sgu.c", "sgc.c", "pec.c", "blk.c", "wax.c")]
            run([*driver, HERE / "kernel.c", *sources, "-I", ROOT / "foundation/include",
                 "-I", ROOT / "sdk/cpt32/include", "-I", out, "-O1", "--origin", "0x10000",
                 "--stack-top", "0x100000", "-o", out / "kernel.wax"])
            for cpu in ("scalar", "pipeline"):
                self.assertIn("PASS:", run([bench.resolve(), out / "kernel.wax", "42", "--cpu", cpu,
                                             "--frames", "1200"], cwd=out, timeout=300))


if __name__ == "__main__":
    unittest.main()
