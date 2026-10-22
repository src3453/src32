#include <foundation/lzma.h>
#include <string.h>
#include "fixtures.h"

static uint8_t packed[2048], restored[2048], source[1024];

int main(void) {
    uint64_t work[262144];
    uint32_t n = sizeof(restored);
    if (cpt_lzma_decompress(restored, &n, fixture, sizeof(fixture), work, sizeof(work)) ||
        n != sizeof(plain) || memcmp(restored, plain, n))
        return 101;
    for (uint32_t i = 0; i < sizeof(source); ++i)
        source[i] = (uint8_t)(i * 17 + (i >> 4));
    for (uint32_t count = 0; count < 2; ++count) {
        uint32_t size = count ? sizeof(source) : 0;
        n = sizeof(packed);
        if (cpt_lzma_compress(packed, &n, source, size, work, sizeof(work), 1, 4096))
            return 102;
        uint32_t actual = sizeof(restored);
        if (cpt_lzma_decompress(restored, &actual, packed, n, work, sizeof(work)) ||
            actual != size || memcmp(restored, source, size))
            return 103;
    }
    n = sizeof(packed);
    if (cpt_lzma_compress(packed, &n, source, 20, work, 8, 1, 4096) != CPT_LZMA_MEMORY || n)
        return 104;
    n = 1;
    if (cpt_lzma_decompress(restored, &n, fixture, sizeof(fixture), work, sizeof(work)) == 0 || n)
        return 105;
    n = sizeof(restored);
    if (!cpt_lzma_decompress(restored, &n, fixture, sizeof(fixture) - 2, work, sizeof(work)) || n)
        return 106;
    n = sizeof(restored);
    if (cpt_lzma_decompress(restored, &n, fixture, 12, work, sizeof(work)) != CPT_LZMA_INPUT_EOF || n)
        return 107;
    n = sizeof(packed);
    if (cpt_lzma_compress(packed, &n, source, 20, work, sizeof(work), 10, 4096) != CPT_LZMA_PARAMETER || n)
        return 108;
    n = sizeof(packed);
    if (cpt_lzma_compress(packed, &n, source, 20, (uint8_t *)work + 1,
                          sizeof(work) - 1, 1, 4096) != CPT_LZMA_PARAMETER || n)
        return 109;
    return 42;
}
