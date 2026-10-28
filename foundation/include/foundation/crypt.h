#ifndef FOUNDATION_CRYPT_H
#define FOUNDATION_CRYPT_H
#include <foundation/conde.h>

/* ChaCha20 and Poly1305 WAL API. The buffer sizing and CONDE_* result
 * conventions match conde: dst_size is capacity in and bytes written out;
 * on failure it is set to zero. No heap, fnd imports, or caller work area is
 * used. Key/nonce/tag sizes follow RFC 8439. Obtain secret keys and fresh
 * nonces/one-time Poly1305 keys from the high-quality /dev/trng device. */
#define CRYPT_CHACHA20_KEY_SIZE 32u
#define CRYPT_CHACHA20_NONCE_SIZE 12u
#define CRYPT_POLY1305_KEY_SIZE 32u
#define CRYPT_POLY1305_TAG_SIZE 16u

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

#endif
