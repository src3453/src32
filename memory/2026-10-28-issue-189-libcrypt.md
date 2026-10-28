# Foundation libcrypt: ChaCha20 and Poly1305

- `foundation/lib/crypt` builds a self-contained `crypt` WAL. Keep `crypt.h` exports aligned with `crypt.exports`; boot images install it as `/lib/crypt.1.wal`.
- ChaCha20 follows RFC 8439's IETF 96-bit nonce and 32-bit block counter. Encrypt/decrypt are the same XOR operation. The caller must never repeat a key/nonce pair and must prevent counter wrap within one call.
- Poly1305 uses a 32-byte one-time key. Reusing that key for another message breaks the security contract. Tag verification compares all 16 bytes before returning.
- Key and nonce entropy are caller responsibilities; Foundation recommends reading `/dev/trng` for key material, fresh nonces, and Poly1305 one-time keys. `/dev/prng` is not suitable for cryptography.
- The API follows conde's `CONDE_*` status and destination-capacity/result-size convention, but has algorithm-specific key/nonce parameters and no work area. `cpt_chacha20_encrypt_bound(n) == n`.
- Verification: `python -m unittest foundation.tests.crypt.test_crypt -v` checks RFC vectors on the host and in Foundation scalar/pipeline U mode when `src32_testbench` is present. `python -m unittest foundation.tests.disk.test_disk_boot.DiskBootTests.test_image_layout -v` checks the default disk contains the WAL.
