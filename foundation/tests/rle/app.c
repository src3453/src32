/* conde の確認: rle と lzma を同時に import し、同じ呼び方で往復する。 */
#include <foundation/rle.h>
#include <foundation/lzma.h>
#include <string.h>

static uint8_t source[1500], packed[2048], restored[1500];

/* 手計算した PackBits: 連続 5 個の 'a'、literal "bc"、200 個の 'z'（128 + 72）。 */
static const uint8_t vector_plain_head[] = "aaaaabc";

int main(void) {
    uint64_t work[262144];
    for (uint32_t i = 0; i < sizeof(source); ++i)
        source[i] = (i / 50) % 3 == 0 ? (uint8_t)(i * 37 + (i >> 3)) : (uint8_t)(i / 50);
    uint32_t n = sizeof(packed), m = sizeof(restored);
    if (cpt_rle_compress(packed, &n, vector_plain_head, 7, 0, 0, 0, 0) != CONDE_OK ||
        n != 5 || packed[0] != 252 || packed[1] != 'a' || packed[2] != 1 ||
        packed[3] != 'b' || packed[4] != 'c')
        return 101;
    if (cpt_rle_compress_bound(1500) != 1500 + 12)
        return 102;
    n = sizeof(packed);
    if (cpt_rle_compress(packed, &n, source, sizeof(source), 0, 0, 5, 0) != CONDE_OK ||
        n > cpt_rle_compress_bound(sizeof(source)))
        return 103;
    if (cpt_rle_decompress(restored, &m, packed, n, 0, 0) != CONDE_OK ||
        m != sizeof(source) || memcmp(source, restored, m))
        return 104;
    m = 10;
    if (cpt_rle_decompress(restored, &m, packed, n, 0, 0) != CONDE_OUTPUT_EOF || m)
        return 105;
    n = sizeof(packed);
    if (cpt_rle_compress(packed, &n, source, 8, 0, 0, 0, 1) != CONDE_PARAMETER || n)
        return 106;
    /* 同じ呼び方で lzma。 */
    uint32_t bound = cpt_lzma_compress_bound(sizeof(source));
    if (bound < sizeof(source) || bound > sizeof(packed) + 1500)
        return 107;
    n = sizeof(packed);
    if (cpt_lzma_compress(packed, &n, source, sizeof(source), work, sizeof(work), 1, 4096) != CONDE_OK)
        return 108;
    m = sizeof(restored);
    if (cpt_lzma_decompress(restored, &m, packed, n, work, sizeof(work)) != CONDE_OK ||
        m != sizeof(source) || memcmp(source, restored, m))
        return 109;
    return 42;
}
