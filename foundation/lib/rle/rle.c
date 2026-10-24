/* PackBits RLE. llvm-src32 は 1 byte あたりの命令数が多いため、ラン検出・コピー・埋め込みは
 * 4 byte 単位（非整列アクセス可、doc/spec_CPU.md）で行う。heap・libfnd・作業領域は使わない。 */
#include <foundation/rle.h>

typedef struct { uint32_t v; } __attribute__((packed)) Word;

static inline uint32_t load(const uint8_t *p) { return ((const Word *)p)->v; }
static inline void store(uint8_t *p, uint32_t v) { ((Word *)p)->v = v; }

/* w ^ (w << 8) の隣接 byte 差のうち、先頭 3 byte が等しい（位置 0）／2..4 byte 目が等しい（位置 1）印。 */
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
#define RUN_AT_0 0xFFFF0000u
#define RUN_AT_1 0x00FFFF00u
#else
#define RUN_AT_0 0x00FFFF00u
#define RUN_AT_1 0xFFFF0000u
#endif

static void copy(uint8_t *dst, const uint8_t *src, uint32_t n) {
    uint32_t i = 0;
    for (; i + 4 <= n; i += 4)
        store(dst + i, load(src + i));
    for (; i < n; ++i)
        dst[i] = src[i];
}

static void fill(uint8_t *dst, uint8_t b, uint32_t n) {
    uint32_t word = b * 0x01010101u, i = 0;
    for (; i + 4 <= n; i += 4)
        store(dst + i, word);
    for (; i < n; ++i)
        dst[i] = b;
}

/* src[i] から続く同じ byte の長さ（最大 128）。 */
static uint32_t run_length(const uint8_t *src, uint32_t i, uint32_t n) {
    uint8_t b = src[i];
    uint32_t limit = n - i < 128 ? n : i + 128, word = b * 0x01010101u, j = i + 1;
    for (; j + 4 <= limit && load(src + j) == word; j += 4)
        ;
    for (; j < limit && src[j] == b; ++j)
        ;
    return j - i;
}

/* literal の終わり: s より後で最初に 3 連続が始まる位置（なければ n）。上限は呼出側で切る。 */
static uint32_t literal_end(const uint8_t *src, uint32_t s, uint32_t limit, uint32_t n) {
    uint32_t k = s + 1;
    while (k < limit) {
        if (k + 4 <= n) {
            uint32_t w = load(src + k), d = w ^ (w << 8);
            if (!(d & RUN_AT_0))
                return k;
            if (!(d & RUN_AT_1))
                return k + 1;
            k += 2;
        } else {
            if (k + 2 < n && src[k] == src[k + 1] && src[k] == src[k + 2])
                return k;
            ++k;
        }
    }
    return limit;
}

int32_t cpt_rle_compress(uint8_t *dst, uint32_t *dst_size,
                         const uint8_t *src, uint32_t src_size,
                         void *work, uint32_t work_size,
                         uint32_t level, uint32_t param) {
    (void)work;
    (void)work_size;
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size, o = 0, i = 0;
    *dst_size = 0;
    if ((src_size && !src) || (capacity && !dst) || level > 9 || param)
        return CONDE_PARAMETER;
    while (i < src_size) {
        uint8_t b = src[i];
        /* 3 連続、または直前が同じ byte で 128 打切りの続き（2 連続）なら繰返し。 */
        if (i + 1 < src_size && src[i + 1] == b &&
            ((i + 2 < src_size && src[i + 2] == b) || (i && src[i - 1] == b))) {
            uint32_t run = run_length(src, i, src_size);
            if (capacity - o < 2)
                return CONDE_OUTPUT_EOF;
            dst[o++] = (uint8_t)(257 - run);
            dst[o++] = b;
            i += run;
        } else {
            uint32_t limit = src_size - i < 128 ? src_size : i + 128;
            uint32_t n = literal_end(src, i, limit, src_size) - i;
            if (capacity - o < n + 1)
                return CONDE_OUTPUT_EOF;
            dst[o++] = (uint8_t)(n - 1);
            copy(dst + o, src + i, n);
            o += n;
            i += n;
        }
    }
    *dst_size = o;
    return CONDE_OK;
}

int32_t cpt_rle_decompress(uint8_t *dst, uint32_t *dst_size,
                           const uint8_t *src, uint32_t src_size,
                           void *work, uint32_t work_size) {
    (void)work;
    (void)work_size;
    if (!dst_size)
        return CONDE_PARAMETER;
    uint32_t capacity = *dst_size, o = 0, i = 0;
    *dst_size = 0;
    if ((src_size && !src) || (capacity && !dst))
        return CONDE_PARAMETER;
    while (i < src_size) {
        uint32_t c = src[i++];
        if (c < 128) {
            uint32_t n = c + 1;
            if (src_size - i < n)
                return CONDE_INPUT_EOF;
            if (capacity - o < n)
                return CONDE_OUTPUT_EOF;
            copy(dst + o, src + i, n);
            i += n;
            o += n;
        } else if (c == 128) {
            return CONDE_DATA;
        } else {
            uint32_t n = 257 - c;
            if (i == src_size)
                return CONDE_INPUT_EOF;
            if (capacity - o < n)
                return CONDE_OUTPUT_EOF;
            fill(dst + o, src[i++], n);
            o += n;
        }
    }
    *dst_size = o;
    return CONDE_OK;
}

uint32_t cpt_rle_compress_bound(uint32_t src_size) {
    /* 定数比較にする（飽和加算にまとめられると llvm-src32 が受け付けない）。 */
    return src_size > 0xFE000000u ? 0xFFFFFFFFu : src_size + (src_size + 127) / 128;
}
