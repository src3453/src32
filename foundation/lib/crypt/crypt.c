/* RFC 8439 ChaCha20, Poly1305 and their AEAD, SHA-256, HMAC/HKDF-SHA256 and
 * X25519 (RFC 7748). Self-contained: no heap, libfnd, or work
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

/* Incremental Poly1305 (26-bit limbs). poly_update buffers partial blocks so
 * the AEAD can feed AD, padding, ciphertext and lengths piece by piece. */
typedef struct {
    uint32_t r0, r1, r2, r3, r4;
    uint32_t h0, h1, h2, h3, h4;
    uint8_t pad[16];
    uint8_t buf[16];
    uint32_t used;
} poly_state;

static void poly_init(poly_state *st, const uint8_t key[32]) {
    uint32_t i;
    st->r0 = load32(key + 0) & 0x3ffffffu;
    st->r1 = (load32(key + 3) >> 2) & 0x3ffff03u;
    st->r2 = (load32(key + 6) >> 4) & 0x3ffc0ffu;
    st->r3 = (load32(key + 9) >> 6) & 0x3f03fffu;
    st->r4 = (load32(key + 12) >> 8) & 0x00fffffu;
    st->h0 = st->h1 = st->h2 = st->h3 = st->h4 = 0;
    for (i = 0; i < 16; ++i)
        st->pad[i] = key[16 + i];
    st->used = 0;
}

static void poly_block(poly_state *st, const uint8_t block[16], uint32_t hibit) {
    const uint32_t r0 = st->r0, r1 = st->r1, r2 = st->r2, r3 = st->r3, r4 = st->r4;
    const uint32_t s1 = r1 * 5, s2 = r2 * 5, s3 = r3 * 5, s4 = r4 * 5;
    uint32_t h0 = st->h0, h1 = st->h1, h2 = st->h2, h3 = st->h3, h4 = st->h4;
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
    st->h0 = h0; st->h1 = h1; st->h2 = h2; st->h3 = h3; st->h4 = h4;
}

static void poly_update(poly_state *st, const uint8_t *m, uint32_t bytes) {
    while (bytes) {
        if (st->used == 0 && bytes >= 16) {
            poly_block(st, m, 1u << 24);
            m += 16;
            bytes -= 16;
            continue;
        }
        st->buf[st->used++] = *m++;
        --bytes;
        if (st->used == 16) {
            poly_block(st, st->buf, 1u << 24);
            st->used = 0;
        }
    }
}

/* Feeds zero bytes up to the next 16-byte boundary (RFC 8439 §2.8). */
static void poly_pad16(poly_state *st) {
    static const uint8_t zero[16] = {0};
    if (st->used)
        poly_update(st, zero, 16 - st->used);
}

static void poly_finish(poly_state *st, uint8_t tag[16]) {
    if (st->used) {
        uint32_t i;
        st->buf[st->used] = 1;
        for (i = st->used + 1; i < 16; ++i)
            st->buf[i] = 0;
        poly_block(st, st->buf, 0);
        st->used = 0;
    }
    uint32_t h0 = st->h0, h1 = st->h1, h2 = st->h2, h3 = st->h3, h4 = st->h4;
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

    uint64_t f0 = (((uint64_t)h0 | ((uint64_t)h1 << 26)) & 0xffffffffu) + load32(st->pad + 0);
    uint64_t f1 = (((uint64_t)(h1 >> 6) | ((uint64_t)h2 << 20)) & 0xffffffffu) + load32(st->pad + 4) + (f0 >> 32);
    uint64_t f2 = (((uint64_t)(h2 >> 12) | ((uint64_t)h3 << 14)) & 0xffffffffu) + load32(st->pad + 8) + (f1 >> 32);
    uint64_t f3 = (((uint64_t)(h3 >> 18) | ((uint64_t)h4 << 8)) & 0xffffffffu) + load32(st->pad + 12) + (f2 >> 32);
    store32(tag + 0, (uint32_t)f0);
    store32(tag + 4, (uint32_t)f1);
    store32(tag + 8, (uint32_t)f2);
    store32(tag + 12, (uint32_t)f3);
}

static void poly1305_auth(uint8_t tag[16], const uint8_t *m, uint32_t bytes,
                          const uint8_t key[32]) {
    poly_state st;
    poly_init(&st, key);
    poly_update(&st, m, bytes);
    poly_finish(&st, tag);
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

static uint32_t verify16(const uint8_t *a, const uint8_t *b) {
    uint32_t diff = 0, i;
    for (i = 0; i < 16; ++i)
        diff |= (uint32_t)(a[i] ^ b[i]);
    return diff;
}

static void wipe(void *p, uint32_t n) {
    volatile uint8_t *v = (volatile uint8_t *)p;
    while (n--)
        *v++ = 0;
}

/* ---- ChaCha20-Poly1305 AEAD (RFC 8439 §2.8) ---- */

static void aead_tag(uint8_t tag[16], const uint8_t *ad, uint32_t ad_size,
                     const uint8_t *ct, uint32_t ct_size,
                     const uint8_t *key, const uint8_t *nonce) {
    uint8_t block[64], lengths[16];
    poly_state st;
    chacha20_block(block, key, nonce, 0);
    poly_init(&st, block);
    poly_update(&st, ad, ad_size);
    poly_pad16(&st);
    poly_update(&st, ct, ct_size);
    poly_pad16(&st);
    store32(lengths + 0, ad_size);
    store32(lengths + 4, 0);
    store32(lengths + 8, ct_size);
    store32(lengths + 12, 0);
    poly_update(&st, lengths, 16);
    poly_finish(&st, tag);
    wipe(block, sizeof(block));
    wipe(&st, sizeof(st));
}

int32_t cpt_chacha20poly1305_seal(uint8_t *dst, uint32_t *dst_size,
                                  const uint8_t *src, uint32_t src_size,
                                  const uint8_t *ad, uint32_t ad_size,
                                  const uint8_t *key, const uint8_t *nonce) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if ((src_size && !src) || (ad_size && !ad) || !key || !nonce || (capacity && !dst))
        return CONDE_PARAMETER;
    if (src_size > 0xffffffffu - CRYPT_AEAD_TAG_SIZE || src_size > 0xffffffc0u)
        return CONDE_PARAMETER;
    if (capacity < src_size + CRYPT_AEAD_TAG_SIZE)
        return CONDE_OUTPUT_EOF;
    uint32_t n = src_size;
    int32_t status = chacha20_xor(dst, &n, src, src_size, key, nonce, 1);
    if (status)
        return status;
    aead_tag(dst + src_size, ad, ad_size, dst, src_size, key, nonce);
    *dst_size = src_size + CRYPT_AEAD_TAG_SIZE;
    return CONDE_OK;
}

int32_t cpt_chacha20poly1305_open(uint8_t *dst, uint32_t *dst_size,
                                  const uint8_t *src, uint32_t src_size,
                                  const uint8_t *ad, uint32_t ad_size,
                                  const uint8_t *key, const uint8_t *nonce) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if (!src || (ad_size && !ad) || !key || !nonce || (capacity && !dst))
        return CONDE_PARAMETER;
    if (src_size < CRYPT_AEAD_TAG_SIZE)
        return CONDE_DATA;
    uint32_t ct_size = src_size - CRYPT_AEAD_TAG_SIZE;
    if (capacity < ct_size)
        return CONDE_OUTPUT_EOF;
    uint8_t tag[16];
    aead_tag(tag, ad, ad_size, src, ct_size, key, nonce);
    if (verify16(tag, src + ct_size))
        return CONDE_DATA;
    uint32_t n = ct_size;
    int32_t status = chacha20_xor(dst, &n, src, ct_size, key, nonce, 1);
    if (status)
        return status;
    *dst_size = ct_size;
    return CONDE_OK;
}

/* ---- SHA-256 (FIPS 180-4) ---- */

static uint32_t load32be(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

static void store32be(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >> 8);
    p[3] = (uint8_t)v;
}

static uint32_t rotr32(uint32_t v, uint32_t n) {
    return (v >> n) | (v << (32 - n));
}

static const uint32_t sha256_k[64] = {
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u, 0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u, 0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u, 0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u
};

static void sha256_block(uint32_t state[8], const uint8_t block[64]) {
    uint32_t w[64], a, b, c, d, e, f, g, h, i;
    for (i = 0; i < 16; ++i)
        w[i] = load32be(block + 4 * i);
    for (i = 16; i < 64; ++i) {
        uint32_t s0 = rotr32(w[i - 15], 7) ^ rotr32(w[i - 15], 18) ^ (w[i - 15] >> 3);
        uint32_t s1 = rotr32(w[i - 2], 17) ^ rotr32(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    a = state[0]; b = state[1]; c = state[2]; d = state[3];
    e = state[4]; f = state[5]; g = state[6]; h = state[7];
    for (i = 0; i < 64; ++i) {
        uint32_t t1 = h + (rotr32(e, 6) ^ rotr32(e, 11) ^ rotr32(e, 25)) + ((e & f) ^ (~e & g)) + sha256_k[i] + w[i];
        uint32_t t2 = (rotr32(a, 2) ^ rotr32(a, 13) ^ rotr32(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }
    state[0] += a; state[1] += b; state[2] += c; state[3] += d;
    state[4] += e; state[5] += f; state[6] += g; state[7] += h;
}

int32_t cpt_sha256_init(cpt_sha256_ctx *ctx) {
    static const uint32_t iv[8] = {
        0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au, 0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u
    };
    uint32_t i;
    if (!ctx)
        return CONDE_PARAMETER;
    for (i = 0; i < 8; ++i)
        ctx->state[i] = iv[i];
    ctx->bytes_lo = ctx->bytes_hi = 0;
    return CONDE_OK;
}

int32_t cpt_sha256_update(cpt_sha256_ctx *ctx, const uint8_t *src, uint32_t src_size) {
    if (!ctx || (src_size && !src))
        return CONDE_PARAMETER;
    uint32_t used = ctx->bytes_lo & 63u;
    uint32_t old = ctx->bytes_lo;
    ctx->bytes_lo += src_size;
    if (ctx->bytes_lo < old)
        ++ctx->bytes_hi;
    while (src_size) {
        if (used == 0 && src_size >= 64) {
            sha256_block(ctx->state, src);
            src += 64;
            src_size -= 64;
            continue;
        }
        ctx->buf[used++] = *src++;
        --src_size;
        if (used == 64) {
            sha256_block(ctx->state, ctx->buf);
            used = 0;
        }
    }
    return CONDE_OK;
}

int32_t cpt_sha256_final(cpt_sha256_ctx *ctx, uint8_t *dst, uint32_t *dst_size) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if (!ctx || (capacity && !dst))
        return CONDE_PARAMETER;
    if (capacity < CRYPT_SHA256_SIZE)
        return CONDE_OUTPUT_EOF;
    uint32_t used = ctx->bytes_lo & 63u, i;
    uint32_t bits_hi = (ctx->bytes_hi << 3) | (ctx->bytes_lo >> 29), bits_lo = ctx->bytes_lo << 3;
    ctx->buf[used++] = 0x80;
    if (used > 56) {
        while (used < 64)
            ctx->buf[used++] = 0;
        sha256_block(ctx->state, ctx->buf);
        used = 0;
    }
    while (used < 56)
        ctx->buf[used++] = 0;
    store32be(ctx->buf + 56, bits_hi);
    store32be(ctx->buf + 60, bits_lo);
    sha256_block(ctx->state, ctx->buf);
    for (i = 0; i < 8; ++i)
        store32be(dst + 4 * i, ctx->state[i]);
    wipe(ctx, sizeof(*ctx));
    *dst_size = CRYPT_SHA256_SIZE;
    return CONDE_OK;
}

int32_t cpt_sha256(uint8_t *dst, uint32_t *dst_size, const uint8_t *src, uint32_t src_size) {
    cpt_sha256_ctx ctx;
    if (!dst_size)
        return CONDE_PARAMETER;
    if (src_size && !src) {
        *dst_size = 0;
        return CONDE_PARAMETER;
    }
    cpt_sha256_init(&ctx);
    cpt_sha256_update(&ctx, src, src_size);
    return cpt_sha256_final(&ctx, dst, dst_size);
}

/* ---- HMAC-SHA256 (RFC 2104) and HKDF-SHA256 (RFC 5869) ---- */

static void hmac_sha256(uint8_t out[32], const uint8_t *key, uint32_t key_size,
                        const uint8_t *m1, uint32_t m1_size, const uint8_t *m2, uint32_t m2_size) {
    uint8_t k[64], pad[64], inner[32];
    uint32_t i, n = 32;
    cpt_sha256_ctx ctx;
    for (i = 0; i < 64; ++i)
        k[i] = 0;
    if (key_size > 64) {
        cpt_sha256(k, &n, key, key_size);
    } else {
        for (i = 0; i < key_size; ++i)
            k[i] = key[i];
    }
    for (i = 0; i < 64; ++i)
        pad[i] = k[i] ^ 0x36u;
    cpt_sha256_init(&ctx);
    cpt_sha256_update(&ctx, pad, 64);
    cpt_sha256_update(&ctx, m1, m1_size);
    cpt_sha256_update(&ctx, m2, m2_size);
    n = 32;
    cpt_sha256_final(&ctx, inner, &n);
    for (i = 0; i < 64; ++i)
        pad[i] = k[i] ^ 0x5cu;
    cpt_sha256_init(&ctx);
    cpt_sha256_update(&ctx, pad, 64);
    cpt_sha256_update(&ctx, inner, 32);
    n = 32;
    cpt_sha256_final(&ctx, out, &n);
    wipe(k, sizeof(k));
    wipe(pad, sizeof(pad));
    wipe(inner, sizeof(inner));
}

int32_t cpt_hmac_sha256(uint8_t *dst, uint32_t *dst_size, const uint8_t *key, uint32_t key_size,
                        const uint8_t *src, uint32_t src_size) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if ((key_size && !key) || (src_size && !src) || (capacity && !dst))
        return CONDE_PARAMETER;
    if (capacity < CRYPT_SHA256_SIZE)
        return CONDE_OUTPUT_EOF;
    hmac_sha256(dst, key, key_size, src, src_size, 0, 0);
    *dst_size = CRYPT_SHA256_SIZE;
    return CONDE_OK;
}

int32_t cpt_hkdf_sha256(uint8_t *dst, uint32_t *dst_size, const uint8_t *salt, uint32_t salt_size,
                        const uint8_t *ikm, uint32_t ikm_size, const uint8_t *info, uint32_t info_size) {
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t want = *dst_size;
    *dst_size = 0;
    if ((salt_size && !salt) || (ikm_size && !ikm) || (info_size && !info) || (want && !dst))
        return CONDE_PARAMETER;
    if (want > 255u * CRYPT_SHA256_SIZE)
        return CONDE_PARAMETER;
    uint8_t prk[32], t[32 + 1];
    uint32_t done = 0, t_size = 0, i;
    hmac_sha256(prk, salt, salt_size, ikm, ikm_size, 0, 0);
    for (uint32_t counter = 1; done < want; ++counter) {
        /* T(n) = HMAC(PRK, T(n-1) | info | n): T(n-1) and n go through the
         * second buffer, so join T(n-1) and info with one inner update each. */
        cpt_sha256_ctx ctx;
        uint8_t k[64], pad[64], inner[32], c = (uint8_t)counter;
        uint32_t n;
        for (i = 0; i < 64; ++i)
            k[i] = i < 32 ? prk[i] : 0;
        for (i = 0; i < 64; ++i)
            pad[i] = k[i] ^ 0x36u;
        cpt_sha256_init(&ctx);
        cpt_sha256_update(&ctx, pad, 64);
        cpt_sha256_update(&ctx, t, t_size);
        cpt_sha256_update(&ctx, info, info_size);
        cpt_sha256_update(&ctx, &c, 1);
        n = 32;
        cpt_sha256_final(&ctx, inner, &n);
        for (i = 0; i < 64; ++i)
            pad[i] = k[i] ^ 0x5cu;
        cpt_sha256_init(&ctx);
        cpt_sha256_update(&ctx, pad, 64);
        cpt_sha256_update(&ctx, inner, 32);
        n = 32;
        cpt_sha256_final(&ctx, t, &n);
        t_size = 32;
        for (i = 0; i < 32 && done < want; ++i)
            dst[done++] = t[i];
        wipe(k, sizeof(k));
        wipe(pad, sizeof(pad));
    }
    wipe(prk, sizeof(prk));
    wipe(t, sizeof(t));
    *dst_size = want;
    return CONDE_OK;
}

/* ---- X25519 (RFC 7748), 16 limbs of 16 bits as in TweetNaCl ---- */

typedef int64_t gf[16];

static void car25519(gf o) {
    int i;
    for (i = 0; i < 16; ++i) {
        o[i] += (int64_t)1 << 16;
        int64_t c = o[i] >> 16;
        if (i < 15)
            o[i + 1] += c - 1;
        else
            o[0] += 38 * (c - 1);
        o[i] -= c * 65536;
    }
}

static void sel25519(gf p, gf q, int64_t b) {
    int64_t c = ~(b - 1);
    int i;
    for (i = 0; i < 16; ++i) {
        int64_t t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}

static void pack25519(uint8_t o[32], const gf n) {
    gf m, t;
    int i, j;
    for (i = 0; i < 16; ++i)
        t[i] = n[i];
    car25519(t);
    car25519(t);
    car25519(t);
    for (j = 0; j < 2; ++j) {
        m[0] = t[0] - 0xffed;
        for (i = 1; i < 15; ++i) {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        int64_t b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        sel25519(t, m, 1 - b);
    }
    for (i = 0; i < 16; ++i) {
        o[2 * i] = (uint8_t)(t[i] & 0xff);
        o[2 * i + 1] = (uint8_t)(t[i] >> 8);
    }
}

static void unpack25519(gf o, const uint8_t n[32]) {
    int i;
    for (i = 0; i < 16; ++i)
        o[i] = n[2 * i] + ((int64_t)n[2 * i + 1] << 8);
    o[15] &= 0x7fff;
}

static void fadd(gf o, const gf a, const gf b) {
    int i;
    for (i = 0; i < 16; ++i)
        o[i] = a[i] + b[i];
}

static void fsub(gf o, const gf a, const gf b) {
    int i;
    for (i = 0; i < 16; ++i)
        o[i] = a[i] - b[i];
}

/* Limbs stay within ±2^20 between carries, so each product fits a 32x32
 * signed multiply; only the sums need 64 bits. */
static void fmul(gf o, const gf a, const gf b) {
    int64_t t[31];
    int32_t a32[16], b32[16];
    int i, j;
    for (i = 0; i < 16; ++i) {
        a32[i] = (int32_t)a[i];
        b32[i] = (int32_t)b[i];
    }
    for (i = 0; i < 31; ++i)
        t[i] = 0;
    for (i = 0; i < 16; ++i)
        for (j = 0; j < 16; ++j)
            t[i + j] += (int64_t)a32[i] * b32[j];
    for (i = 0; i < 15; ++i)
        t[i] += 38 * t[i + 16];
    for (i = 0; i < 16; ++i)
        o[i] = t[i];
    car25519(o);
    car25519(o);
}

/* a * a with each cross product computed once (about half of fmul). */
static void fsqr(gf o, const gf a) {
    int64_t t[31];
    int32_t a32[16], d32[16];
    int i, j;
    for (i = 0; i < 16; ++i) {
        a32[i] = (int32_t)a[i];
        d32[i] = 2 * (int32_t)a[i];
    }
    for (i = 0; i < 31; ++i)
        t[i] = 0;
    for (i = 0; i < 16; ++i) {
        t[2 * i] += (int64_t)a32[i] * a32[i];
        for (j = i + 1; j < 16; ++j)
            t[i + j] += (int64_t)d32[i] * a32[j];
    }
    for (i = 0; i < 15; ++i)
        t[i] += 38 * t[i + 16];
    for (i = 0; i < 16; ++i)
        o[i] = t[i];
    car25519(o);
    car25519(o);
}

/* a * 121665 ((A - 2) / 4 of Curve25519). */
static void fmul121665(gf o, const gf a) {
    int i;
    for (i = 0; i < 16; ++i)
        o[i] = (int64_t)(int32_t)a[i] * 121665;
    car25519(o);
    car25519(o);
}

static void finv(gf o, const gf in) {
    gf c;
    int a;
    for (a = 0; a < 16; ++a)
        c[a] = in[a];
    for (a = 253; a >= 0; --a) {
        fsqr(c, c);
        if (a != 2 && a != 4)
            fmul(c, c, in);
    }
    for (a = 0; a < 16; ++a)
        o[a] = c[a];
}

static void x25519(uint8_t q[32], const uint8_t n[32], const uint8_t p[32]) {
    uint8_t z[32];
    gf x, a, b, c, d, e, f;
    int i;
    for (i = 0; i < 31; ++i)
        z[i] = n[i];
    z[31] = (uint8_t)((n[31] & 127) | 64);
    z[0] &= 248;
    unpack25519(x, p);
    for (i = 0; i < 16; ++i) {
        b[i] = x[i];
        d[i] = a[i] = c[i] = 0;
    }
    a[0] = d[0] = 1;
    for (i = 254; i >= 0; --i) {
        int64_t r = (z[i >> 3] >> (i & 7)) & 1;
        sel25519(a, b, r);
        sel25519(c, d, r);
        fadd(e, a, c);
        fsub(a, a, c);
        fadd(c, b, d);
        fsub(b, b, d);
        fsqr(d, e);
        fsqr(f, a);
        fmul(a, c, a);
        fmul(c, b, e);
        fadd(e, a, c);
        fsub(a, a, c);
        fsqr(b, a);
        fsub(c, d, f);
        fmul121665(a, c);
        fadd(a, a, d);
        fmul(c, c, a);
        fmul(a, d, f);
        fmul(d, b, x);
        fsqr(b, e);
        sel25519(a, b, r);
        sel25519(c, d, r);
    }
    finv(c, c);
    fmul(a, a, c);
    pack25519(q, a);
    wipe(z, sizeof(z));
}

int32_t cpt_x25519(uint8_t *out, const uint8_t *scalar, const uint8_t *point) {
    if (!out || !scalar || !point)
        return CONDE_PARAMETER;
    uint8_t result[32];
    uint32_t i, any = 0;
    x25519(result, scalar, point);
    for (i = 0; i < 32; ++i) {
        out[i] = result[i];
        any |= result[i];
    }
    /* An all-zero result means a low-order peer point (RFC 7748 §6.1). */
    return any ? CONDE_OK : CONDE_DATA;
}

int32_t cpt_x25519_base(uint8_t *out, const uint8_t *scalar) {
    static const uint8_t base[32] = {9};
    if (!out || !scalar)
        return CONDE_PARAMETER;
    x25519(out, scalar, base);
    return CONDE_OK;
}
