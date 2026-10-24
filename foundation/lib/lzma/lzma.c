/* CPT32 adapter. SDK callbacks stay inside this WAL and never cross its ABI. */
#include <foundation/lzma.h>
#include "LzmaEnc.h"
#include "LzmaDec.h"

typedef struct {
    ISzAlloc api;
    Byte *base;
    SizeT size, used;
} Arena;

static void *arena_alloc(ISzAllocPtr api, size_t size) {
    Arena *a = (Arena *)api;
    SizeT left = a->size - a->used;
    if (size > left || size > (SizeT)-1 - 7)
        return 0;
    SizeT aligned = (size + 7) & ~(SizeT)7;
    if (aligned > left)
        return 0;
    void *p = a->base + a->used;
    a->used += aligned;
    return p;
}

static void arena_free(ISzAllocPtr api, void *address) {
    (void)api;
    (void)address;
}

static int arena_init(Arena *a, void *work, uint32_t size) {
    if (!work || ((uintptr_t)work & 7))
        return 0;
    a->api.Alloc = arena_alloc;
    a->api.Free = arena_free;
    a->base = (Byte *)work;
    a->size = size;
    a->used = 0;
    return 1;
}

int32_t cpt_lzma_compress(uint8_t *dst, uint32_t *dst_size,
                          const uint8_t *src, uint32_t src_size,
                          void *work, uint32_t work_size,
                          uint32_t level, uint32_t dictionary_size) {
    Arena a;
    if (!dst_size)
        return SZ_ERROR_PARAM;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if (!dst || !src || level > 9 || dictionary_size < 4096 ||
        dictionary_size > (1u << 24) || !arena_init(&a, work, work_size))
        return SZ_ERROR_PARAM;
    if (capacity < 13)
        return SZ_ERROR_OUTPUT_EOF;
    CLzmaEncProps props;
    LzmaEncProps_Init(&props);
    props.level = (int)level;
    props.dictSize = dictionary_size;
    props.numThreads = 1;
    SizeT output = capacity - 13, property_size = 5;
    SRes result = LzmaEncode(dst + 13, &output, src, src_size, &props,
                            dst, &property_size, 1, 0, &a.api, &a.api);
    if (result != SZ_OK)
        return result;
    /* The .lzma header is little-endian even on the big-endian guest. */
    for (uint32_t i = 0; i < 8; ++i)
        dst[5 + i] = i < 4 ? (Byte)(src_size >> (8 * i)) : 0;
    *dst_size = (uint32_t)output + 13;
    return SZ_OK;
}

int32_t cpt_lzma_decompress(uint8_t *dst, uint32_t *dst_size,
                            const uint8_t *src, uint32_t src_size,
                            void *work, uint32_t work_size) {
    Arena a;
    if (!dst_size)
        return SZ_ERROR_PARAM;
    uint32_t capacity = *dst_size;
    *dst_size = 0;
    if (!dst || !src || !arena_init(&a, work, work_size))
        return SZ_ERROR_PARAM;
    if (src_size < 13)
        return SZ_ERROR_INPUT_EOF;
    uint32_t lo = 0, hi = 0;
    for (uint32_t i = 0; i < 4; ++i) {
        lo |= (uint32_t)src[5 + i] << (8 * i);
        hi |= (uint32_t)src[9 + i] << (8 * i);
    }
    int unknown = lo == 0xFFFFFFFFu && hi == 0xFFFFFFFFu;
    if (!unknown && (hi || lo > capacity))
        return SZ_ERROR_OUTPUT_EOF;
    SizeT output = unknown ? capacity : lo, input = src_size - 13;
    ELzmaStatus status;
    SRes result = LzmaDecode(dst, &output, src + 13, &input, src, 5,
                            LZMA_FINISH_END, &status, &a.api);
    if (result != SZ_OK)
        return result;
    if (status != LZMA_STATUS_FINISHED_WITH_MARK &&
        !(status == LZMA_STATUS_MAYBE_FINISHED_WITHOUT_MARK && !unknown))
        return unknown && output == capacity ? SZ_ERROR_OUTPUT_EOF : SZ_ERROR_DATA;
    if ((!unknown && output != lo) || input != src_size - 13)
        return SZ_ERROR_DATA;
    *dst_size = (uint32_t)output;
    return SZ_OK;
}

uint32_t cpt_lzma_compress_bound(uint32_t src_size) {
    return src_size > 0xBFFFFF00u ? 0xFFFFFFFFu : src_size + src_size / 3 + 128 + 13;
}
