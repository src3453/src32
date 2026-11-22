#ifndef FOUNDATION_CRYPT_H
#define FOUNDATION_CRYPT_H
#include <foundation/conde.h>

/* libcrypt WAL API (ABI 1.1): ChaCha20, Poly1305, ChaCha20-Poly1305 AEAD,
 * SHA-256, HMAC-SHA256, HKDF-SHA256 and X25519. 1.0 had only ChaCha20 and
 * Poly1305; 1.1 adds the rest for L3P encryption (doc/spec_NET_L3P.md §7). The buffer sizing and CONDE_* result
 * conventions match conde: dst_size is capacity in and bytes written out;
 * on failure it is set to zero. No heap, fnd imports, or caller work area is
 * used. Key/nonce/tag sizes follow RFC 8439. Obtain secret keys and fresh
 * nonces/one-time Poly1305 keys from the high-quality /dev/trng device. */
#define CRYPT_CHACHA20_KEY_SIZE 32u
#define CRYPT_CHACHA20_NONCE_SIZE 12u
#define CRYPT_POLY1305_KEY_SIZE 32u
#define CRYPT_POLY1305_TAG_SIZE 16u
#define CRYPT_AEAD_TAG_SIZE 16u
#define CRYPT_SHA256_SIZE 32u
#define CRYPT_X25519_SIZE 32u

/* Encrypt or decrypt src with the RFC 8439 IETF ChaCha20 stream cipher.
 * The two directions are the same XOR operation. Exact in-place operation is
 * supported; other overlapping src/dst ranges are not. The counter is the
 * initial 32-bit block counter and must not wrap during this call.
 * key and nonce must point to 32 and 12 bytes respectively. src may be NULL
 * only when src_size is zero; dst may be NULL only when its capacity is zero.
 * cpt_chacha20_encrypt_bound returns src_size. */
int32_t cpt_chacha20_encrypt(uint8_t *dst, uint32_t *dst_size,
                             const uint8_t *src, uint32_t src_size,
                             const uint8_t *key, const uint8_t *nonce,
                             uint32_t counter);
int32_t cpt_chacha20_decrypt(uint8_t *dst, uint32_t *dst_size,
                             const uint8_t *src, uint32_t src_size,
                             const uint8_t *key, const uint8_t *nonce,
                             uint32_t counter);
uint32_t cpt_chacha20_encrypt_bound(uint32_t src_size);

/* Compute a Poly1305 one-time authenticator. dst_size must have capacity for
 * 16 bytes and is set to 16 on success. The key must be unique per message.
 * src may be NULL only when src_size is zero. */
int32_t cpt_poly1305_auth(uint8_t *dst, uint32_t *dst_size,
                          const uint8_t *src, uint32_t src_size,
                          const uint8_t *key);

/* Verify a 16-byte tag in constant time. Returns CONDE_OK for a match,
 * CONDE_DATA for a mismatch, or CONDE_PARAMETER for invalid pointers. */
int32_t cpt_poly1305_verify(const uint8_t *tag, const uint8_t *src,
                            uint32_t src_size, const uint8_t *key);

/* ChaCha20-Poly1305 AEAD (RFC 8439 §2.8). seal writes ciphertext followed by
 * the 16-byte tag (dst capacity src_size + 16); open takes that form and
 * writes the plaintext (src_size - 16 bytes) only when the tag matches, else
 * returns CONDE_DATA. Exact in-place operation (dst == src) is supported.
 * Never reuse a key/nonce pair. */
int32_t cpt_chacha20poly1305_seal(uint8_t *dst, uint32_t *dst_size,
                                  const uint8_t *src, uint32_t src_size,
                                  const uint8_t *ad, uint32_t ad_size,
                                  const uint8_t *key, const uint8_t *nonce);
int32_t cpt_chacha20poly1305_open(uint8_t *dst, uint32_t *dst_size,
                                  const uint8_t *src, uint32_t src_size,
                                  const uint8_t *ad, uint32_t ad_size,
                                  const uint8_t *key, const uint8_t *nonce);

/* SHA-256. The context is plain caller memory (no alignment needed beyond
 * uint32_t); final wipes it. dst capacity must be at least 32. */
typedef struct {
    uint32_t state[8];
    uint32_t bytes_lo, bytes_hi;
    uint8_t buf[64];
} cpt_sha256_ctx;
int32_t cpt_sha256_init(cpt_sha256_ctx *ctx);
int32_t cpt_sha256_update(cpt_sha256_ctx *ctx, const uint8_t *src, uint32_t src_size);
int32_t cpt_sha256_final(cpt_sha256_ctx *ctx, uint8_t *dst, uint32_t *dst_size);
int32_t cpt_sha256(uint8_t *dst, uint32_t *dst_size, const uint8_t *src, uint32_t src_size);

/* HMAC-SHA256 (RFC 2104) with a key of any length; writes 32 bytes. */
int32_t cpt_hmac_sha256(uint8_t *dst, uint32_t *dst_size, const uint8_t *key, uint32_t key_size,
                        const uint8_t *src, uint32_t src_size);

/* HKDF-SHA256 (RFC 5869) extract-and-expand. *dst_size is the wanted output
 * length (at most 255 * 32) and is kept on success. Noise's HKDF(ck, ikm)
 * with 2 or 3 outputs is cpt_hkdf_sha256 with salt = ck, info empty and a
 * length of 64 or 96. */
int32_t cpt_hkdf_sha256(uint8_t *dst, uint32_t *dst_size, const uint8_t *salt, uint32_t salt_size,
                        const uint8_t *ikm, uint32_t ikm_size, const uint8_t *info, uint32_t info_size);

/* X25519 (RFC 7748). All buffers are 32 bytes; the scalar is clamped
 * internally. cpt_x25519 returns CONDE_DATA (with an all-zero out) when the
 * peer's point has low order; callers must then abort the key exchange.
 * cpt_x25519_base computes the public key of a private scalar (read it
 * from /dev/trng). */
int32_t cpt_x25519(uint8_t *out, const uint8_t *scalar, const uint8_t *point);
int32_t cpt_x25519_base(uint8_t *out, const uint8_t *scalar);

#endif
