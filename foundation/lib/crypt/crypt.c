/* RFC 8439 ChaCha20 and Poly1305. Self-contained: no heap, libfnd, or work
 * buffer. Word loads/stores are byte-wise so caller buffers need no alignment. */
#include <foundation/crypt.h>

static uint32_t load32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static void store32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

static uint32_t rotl32(uint32_t v, uint32_t n) {
    return (v << n) | (v >> (32 - n));
}

#define QR(a, b, c, d) do { \
    a += b; d ^= a; d = rotl32(d, 16); \
    c += d; b ^= c; b = rotl32(b, 12); \
    a += b; d ^= a; d = rotl32(d, 8);  \
    c += d; b ^= c; b = rotl32(b, 7);  \
} while (0)

static void chacha20_block(uint8_t out[64], const uint8_t key[32],
                           const uint8_t nonce[12], uint32_t counter) {
    static const uint32_t sigma[4] = {
        0x61707865u, 0x3320646eu, 0x79622d32u, 0x6b206574u
    };
    uint32_t x[16], initial[16];
    uint32_t i;
    for (i = 0; i < 4; ++i)
        initial[i] = sigma[i];
    for (i = 0; i < 8; ++i)
        initial[4 + i] = load32(key + 4 * i);
    initial[12] = counter;
    initial[13] = load32(nonce);
    initial[14] = load32(nonce + 4);
    initial[15] = load32(nonce + 8);
    for (i = 0; i < 16; ++i)
        x[i] = initial[i];
    for (i = 0; i < 10; ++i) {
        QR(x[0], x[4], x[8], x[12]);
        QR(x[1], x[5], x[9], x[13]);
        QR(x[2], x[6], x[10], x[14]);
        QR(x[3], x[7], x[11], x[15]);
        QR(x[0], x[5], x[10], x[15]);
        QR(x[1], x[6], x[11], x[12]);
        QR(x[2], x[7], x[8], x[13]);
        QR(x[3], x[4], x[9], x[14]);
    }
    for (i = 0; i < 16; ++i)
        store32(out + 4 * i, x[i] + initial[i]);
}

static int32_t chacha20_xor(uint8_t *dst, uint32_t *dst_size,
                            const uint8_t *src, uint32_t src_size,
                            const uint8_t *key, const uint8_t *nonce,
                            uint32_t counter) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if ((src_size && !src) || !key || !nonce || (capacity && !dst))
        return CONDE_PARAMETER;
    if (capacity < src_size)
        return CONDE_OUTPUT_EOF;
    if (src_size && (src_size - 1) / 64 > 0xffffffffu - counter)
        return CONDE_PARAMETER;

    uint8_t block[64];
    uint32_t offset = 0, block_counter = counter;
    while (offset < src_size) {
        uint32_t n = src_size - offset;
        uint32_t i;
        if (n > 64)
            n = 64;
        chacha20_block(block, key, nonce, block_counter++);
        for (i = 0; i < n; ++i)
            dst[offset + i] = src[offset + i] ^ block[i];
        offset += n;
    }
    *dst_size = src_size;
    return CONDE_OK;
}

int32_t cpt_chacha20_encrypt(uint8_t *dst, uint32_t *dst_size,
                             const uint8_t *src, uint32_t src_size,
                             const uint8_t *key, const uint8_t *nonce,
                             uint32_t counter) {
    return chacha20_xor(dst, dst_size, src, src_size, key, nonce, counter);
}

int32_t cpt_chacha20_decrypt(uint8_t *dst, uint32_t *dst_size,
                             const uint8_t *src, uint32_t src_size,
                             const uint8_t *key, const uint8_t *nonce,
                             uint32_t counter) {
    return chacha20_xor(dst, dst_size, src, src_size, key, nonce, counter);
}

uint32_t cpt_chacha20_encrypt_bound(uint32_t src_size) {
    return src_size;
}

#undef QR

static void poly1305_auth(uint8_t tag[16], const uint8_t *m, uint32_t bytes,
                          const uint8_t key[32]) {
    const uint32_t r0 = load32(key + 0) & 0x3ffffffu;
    const uint32_t r1 = (load32(key + 3) >> 2) & 0x3ffff03u;
    const uint32_t r2 = (load32(key + 6) >> 4) & 0x3ffc0ffu;
    const uint32_t r3 = (load32(key + 9) >> 6) & 0x3f03fffu;
    const uint32_t r4 = (load32(key + 12) >> 8) & 0x00fffffu;
    const uint32_t s1 = r1 * 5, s2 = r2 * 5, s3 = r3 * 5, s4 = r4 * 5;
    uint32_t h0 = 0, h1 = 0, h2 = 0, h3 = 0, h4 = 0;

    while (bytes) {
        uint8_t block[16] = {0};
        uint32_t n = bytes < 16 ? bytes : 16;
        uint32_t hibit = 1u << 24;
        uint32_t i;
        for (i = 0; i < n; ++i)
            block[i] = m[i];
        if (n < 16) {
            block[n] = 1;
            hibit = 0;
        }
        h0 += load32(block + 0) & 0x3ffffffu;
        h1 += (load32(block + 3) >> 2) & 0x3ffffffu;
        h2 += (load32(block + 6) >> 4) & 0x3ffffffu;
        h3 += (load32(block + 9) >> 6) & 0x3ffffffu;
        h4 += (load32(block + 12) >> 8) | hibit;

        uint64_t d0 = (uint64_t)h0 * r0 + (uint64_t)h1 * s4 +
                      (uint64_t)h2 * s3 + (uint64_t)h3 * s2 + (uint64_t)h4 * s1;
        uint64_t d1 = (uint64_t)h0 * r1 + (uint64_t)h1 * r0 +
                      (uint64_t)h2 * s4 + (uint64_t)h3 * s3 + (uint64_t)h4 * s2;
        uint64_t d2 = (uint64_t)h0 * r2 + (uint64_t)h1 * r1 +
                      (uint64_t)h2 * r0 + (uint64_t)h3 * s4 + (uint64_t)h4 * s3;
        uint64_t d3 = (uint64_t)h0 * r3 + (uint64_t)h1 * r2 +
                      (uint64_t)h2 * r1 + (uint64_t)h3 * r0 + (uint64_t)h4 * s4;
        uint64_t d4 = (uint64_t)h0 * r4 + (uint64_t)h1 * r3 +
                      (uint64_t)h2 * r2 + (uint64_t)h3 * r1 + (uint64_t)h4 * r0;

        uint32_t c = (uint32_t)(d0 >> 26);
        h0 = (uint32_t)d0 & 0x3ffffffu;
        d1 += c;
        c = (uint32_t)(d1 >> 26);
        h1 = (uint32_t)d1 & 0x3ffffffu;
        d2 += c;
        c = (uint32_t)(d2 >> 26);
        h2 = (uint32_t)d2 & 0x3ffffffu;
        d3 += c;
        c = (uint32_t)(d3 >> 26);
        h3 = (uint32_t)d3 & 0x3ffffffu;
        d4 += c;
        c = (uint32_t)(d4 >> 26);
        h4 = (uint32_t)d4 & 0x3ffffffu;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= 0x3ffffffu;
        h1 += c;
        m += n;
        bytes -= n;
    }

    uint32_t c = h1 >> 26;
    h1 &= 0x3ffffffu;
    h2 += c;
    c = h2 >> 26;
    h2 &= 0x3ffffffu;
    h3 += c;
    c = h3 >> 26;
    h3 &= 0x3ffffffu;
    h4 += c;
    c = h4 >> 26;
    h4 &= 0x3ffffffu;
    h0 += c * 5;
    c = h0 >> 26;
    h0 &= 0x3ffffffu;
    h1 += c;

    uint32_t g0 = h0 + 5;
    c = g0 >> 26;
    g0 &= 0x3ffffffu;
    uint32_t g1 = h1 + c;
    c = g1 >> 26;
    g1 &= 0x3ffffffu;
    uint32_t g2 = h2 + c;
    c = g2 >> 26;
    g2 &= 0x3ffffffu;
    uint32_t g3 = h3 + c;
    c = g3 >> 26;
    g3 &= 0x3ffffffu;
    uint32_t g4 = h4 + c - (1u << 26);
    uint32_t mask = (g4 >> 31) - 1;
    uint32_t inverse = ~mask;
    h0 = (h0 & inverse) | (g0 & mask);
    h1 = (h1 & inverse) | (g1 & mask);
    h2 = (h2 & inverse) | (g2 & mask);
    h3 = (h3 & inverse) | (g3 & mask);
    h4 = (h4 & inverse) | (g4 & mask);

    uint64_t f0 = (((uint64_t)h0 | ((uint64_t)h1 << 26)) & 0xffffffffu) + load32(key + 16);
    uint64_t f1 = (((uint64_t)(h1 >> 6) | ((uint64_t)h2 << 20)) & 0xffffffffu) + load32(key + 20) + (f0 >> 32);
    uint64_t f2 = (((uint64_t)(h2 >> 12) | ((uint64_t)h3 << 14)) & 0xffffffffu) + load32(key + 24) + (f1 >> 32);
    uint64_t f3 = (((uint64_t)(h3 >> 18) | ((uint64_t)h4 << 8)) & 0xffffffffu) + load32(key + 28) + (f2 >> 32);
    store32(tag + 0, (uint32_t)f0);
    store32(tag + 4, (uint32_t)f1);
    store32(tag + 8, (uint32_t)f2);
    store32(tag + 12, (uint32_t)f3);
}

int32_t cpt_poly1305_auth(uint8_t *dst, uint32_t *dst_size,
                          const uint8_t *src, uint32_t src_size,
                          const uint8_t *key) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if (!key || (src_size && !src) || (capacity && !dst))
        return CONDE_PARAMETER;
    if (capacity < CRYPT_POLY1305_TAG_SIZE)
        return CONDE_OUTPUT_EOF;
    poly1305_auth(dst, src, src_size, key);
    *dst_size = CRYPT_POLY1305_TAG_SIZE;
    return CONDE_OK;
}

int32_t cpt_poly1305_verify(const uint8_t *tag, const uint8_t *src,
                            uint32_t src_size, const uint8_t *key) {
    if (!tag || !key || (src_size && !src))
        return CONDE_PARAMETER;
    uint8_t expected[CRYPT_POLY1305_TAG_SIZE];
    uint32_t diff = 0, i;
    poly1305_auth(expected, src, src_size, key);
    for (i = 0; i < CRYPT_POLY1305_TAG_SIZE; ++i)
        diff |= (uint32_t)(expected[i] ^ tag[i]);
    return diff == 0 ? CONDE_OK : CONDE_DATA;
}
