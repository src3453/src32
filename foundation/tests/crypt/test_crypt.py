"""libcrypt vectors (ChaCha20, Poly1305, AEAD, SHA-256, HMAC, HKDF, X25519) on host and in Foundation U mode."""
import ctypes
import hashlib
import hmac
import os
import random
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
EXPORTS = [line.split()[0] for line in (LIB / "crypt.exports").read_text().splitlines()
           if line and not line.startswith("#")]


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
            for name in EXPORTS:
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
        for name in ("cpt_chacha20poly1305_seal", "cpt_chacha20poly1305_open"):
            fn = getattr(cls.lib, name)
            fn.argtypes, fn.restype = [ptr, ptr, ptr, u32, ptr, u32, ptr, ptr], ctypes.c_int32
        for name, args in (("cpt_sha256", [ptr, ptr, ptr, u32]), ("cpt_sha256_init", [ptr]),
                           ("cpt_sha256_update", [ptr, ptr, u32]), ("cpt_sha256_final", [ptr, ptr, ptr]),
                           ("cpt_hmac_sha256", [ptr, ptr, ptr, u32, ptr, u32]),
                           ("cpt_hkdf_sha256", [ptr, ptr, ptr, u32, ptr, u32, ptr, u32]),
                           ("cpt_x25519", [ptr, ptr, ptr]), ("cpt_x25519_base", [ptr, ptr])):
            fn = getattr(cls.lib, name)
            fn.argtypes, fn.restype = args, ctypes.c_int32

    def call_out(self, fn, capacity, *args):
        out = ctypes.create_string_buffer(max(capacity, 1))
        size = ctypes.c_uint32(capacity)
        status = fn(out, ctypes.byref(size), *args)
        return status, out.raw[:size.value]

    def test_sha256_hmac_hkdf(self):
        self.assertEqual(self.call_out(self.lib.cpt_sha256, 32, b"abc", 3)[1].hex(),
                         "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        rng = random.Random(1)
        for size in (0, 1, 55, 56, 63, 64, 65, 119, 120, 1000):
            data = rng.randbytes(size)
            self.assertEqual(self.call_out(self.lib.cpt_sha256, 32, data, size), (OK, hashlib.sha256(data).digest()))
            ctx = ctypes.create_string_buffer(104)
            self.lib.cpt_sha256_init(ctx)
            for i in range(0, size, 7):
                chunk = data[i:i + 7]
                self.lib.cpt_sha256_update(ctx, chunk, len(chunk))
            self.assertEqual(self.call_out(lambda o, s: self.lib.cpt_sha256_final(ctx, o, s), 32),
                             (OK, hashlib.sha256(data).digest()))
            for key in (b"", b"Jefe", rng.randbytes(64), rng.randbytes(131)):
                self.assertEqual(self.call_out(self.lib.cpt_hmac_sha256, 32, key, len(key), data, size),
                                 (OK, hmac.new(key, data, hashlib.sha256).digest()))
        # RFC 4231 test case 2 and RFC 5869 test case 1
        self.assertEqual(self.call_out(self.lib.cpt_hmac_sha256, 32, b"Jefe", 4,
                                       b"what do ya want for nothing?", 28)[1].hex(),
                         "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        ikm, salt, info = bytes([0x0b]) * 22, bytes(range(13)), bytes(range(0xf0, 0xfa))
        self.assertEqual(self.call_out(self.lib.cpt_hkdf_sha256, 42, salt, 13, ikm, 22, info, 10)[1].hex(),
                         "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
        self.assertEqual(self.call_out(self.lib.cpt_sha256, 31, b"", 0), (OUTPUT_EOF, b""))

    def test_aead(self):
        # RFC 8439 §2.8.2
        key = bytes(range(0x80, 0xa0))
        nonce = bytes.fromhex("070000004041424344454647")
        ad = bytes.fromhex("50515253c0c1c2c3c4c5c6c7")
        text = (b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip "
                b"for the future, sunscreen would be it.")
        status, sealed = self.call_out(self.lib.cpt_chacha20poly1305_seal, len(text) + 16,
                                       text, len(text), ad, len(ad), key, nonce)
        self.assertEqual(status, OK)
        self.assertEqual(sealed[-16:].hex(), "1ae10b594f09e26a7e902ecbd0600691")
        self.assertEqual(sealed[:16].hex(), "d31a8d34648e60db7b86afbc53ef7ec2")
        self.assertEqual(self.call_out(self.lib.cpt_chacha20poly1305_open, len(text), sealed, len(sealed),
                                       ad, len(ad), key, nonce), (OK, text))
        bad = bytes([sealed[0] ^ 1]) + sealed[1:]
        self.assertEqual(self.call_out(self.lib.cpt_chacha20poly1305_open, len(text), bad, len(bad),
                                       ad, len(ad), key, nonce)[0], DATA)
        self.assertEqual(self.call_out(self.lib.cpt_chacha20poly1305_open, len(text), sealed, len(sealed),
                                       ad[:-1], len(ad) - 1, key, nonce)[0], DATA)
        try:
            from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
        except ImportError:
            return
        rng = random.Random(2)
        for size in (0, 1, 15, 16, 17, 63, 64, 65, 984):
            key, nonce, ad, data = rng.randbytes(32), rng.randbytes(12), rng.randbytes(size % 23), rng.randbytes(size)
            self.assertEqual(self.call_out(self.lib.cpt_chacha20poly1305_seal, size + 16, data, size,
                                           ad, len(ad), key, nonce),
                             (OK, ChaCha20Poly1305(key).encrypt(nonce, data, ad)))

    def test_x25519(self):
        # RFC 7748 §5.2 and §6.1
        out = ctypes.create_string_buffer(32)
        scalar = bytes.fromhex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4")
        point = bytes.fromhex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")
        self.assertEqual(self.lib.cpt_x25519(out, scalar, point), OK)
        self.assertEqual(out.raw.hex(), "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
        alice = bytes.fromhex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
        bob_pub = bytes.fromhex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        self.assertEqual(self.lib.cpt_x25519_base(out, alice), OK)
        self.assertEqual(out.raw.hex(), "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        self.assertEqual(self.lib.cpt_x25519(out, alice, bob_pub), OK)
        self.assertEqual(out.raw.hex(), "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
        self.assertEqual(self.lib.cpt_x25519(out, alice, bytes(32)), DATA)  # low-order point
        try:
            from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
        except ImportError:
            return
        rng = random.Random(3)
        for _ in range(20):
            a, b = rng.randbytes(32), rng.randbytes(32)
            pub = X25519PrivateKey.from_private_bytes(b).public_key().public_bytes_raw()
            expected = X25519PrivateKey.from_private_bytes(a).exchange(
                X25519PrivateKey.from_private_bytes(b).public_key())
            self.assertEqual(self.lib.cpt_x25519(out, a, pub), OK)
            self.assertEqual(out.raw, expected)

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
                       ("sched.c", "proc.c", "mm.c", "trap.c", "ipc.c", "fb.c", "sgu.c", "sgc.c", "pec.c", "cnet.c", "blk.c", "wax.c")]
            run([*driver, HERE / "kernel.c", *sources, "-I", ROOT / "foundation/include",
                 "-I", ROOT / "sdk/cpt32/include", "-I", out, "-O1", "--origin", "0x10000",
                 "--stack-top", "0x100000", "-o", out / "kernel.wax"])
            for cpu in ("scalar", "pipeline"):
                self.assertIn("PASS:", run([bench.resolve(), out / "kernel.wax", "42", "--cpu", cpu,
                                             "--frames", "1200"], cwd=out, timeout=300))


if __name__ == "__main__":
    unittest.main()
