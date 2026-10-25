#include "../../bin/wav_core.h"
#include <string.h>
static const uint8_t *source;
static uint32_t source_size;
static fnd_status read_at(void *ctx, uint32_t pos, uint8_t *out, uint32_t bytes) {
    (void)ctx;
    if (pos > source_size || bytes > source_size - pos) return FND_E_IO;
    memcpy(out, source + pos, bytes); return FND_OK;
}
int32_t wav_test_parse(const uint8_t *data, uint32_t bytes) {
    source = data; source_size = bytes; wav_info info;
    return wav_parse(0, read_at, bytes, &info);
}
int32_t wav_test_decode(const uint8_t *data, uint32_t bytes, uint8_t *out, uint32_t cap, uint32_t block) {
    source = data; source_size = bytes; wav_info info;
    fnd_status st = wav_parse(0, read_at, bytes, &info);
    if (st < 0) return st;
    wav_decoder decoder; wav_begin(&decoder, 0, read_at, &info);
    uint32_t total = 0;
    while (!decoder.ended) {
        uint32_t size = cap - total < block ? cap - total : block, got;
        if (size < info.channels * 2) return FND_E_LIMIT;
        st = wav_render(&decoder, out + total, size, &got);
        if (st < 0) return st;
        total += got;
    }
    return (int32_t)total;
}
