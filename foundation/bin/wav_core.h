#ifndef FOUNDATION_WAV_CORE_H
#define FOUNDATION_WAV_CORE_H
#include <foundation/fnd.h>

typedef fnd_status (*wav_read_at)(void *, uint32_t, uint8_t *, uint32_t);
typedef struct { uint32_t channels, bits, rate, align, offset, bytes; } wav_info;
static uint32_t wav_u16(const uint8_t *p) { return p[0] | ((uint32_t)p[1] << 8); }
static uint32_t wav_u32(const uint8_t *p) { return wav_u16(p) | (wav_u16(p + 2) << 16); }
static int wav_tag(const uint8_t *p, const char *s) {
    return p[0] == s[0] && p[1] == s[1] && p[2] == s[2] && p[3] == s[3];
}
/* Walk every chunk before acquiring the SGU. Bounds use subtraction so even
 * hostile UINT32_MAX chunk sizes cannot wrap into the accepted range. */
static fnd_status wav_parse(void *ctx, wav_read_at read, uint32_t size, wav_info *out) {
    uint8_t h[16]; wav_info w = {0}; uint32_t fmt = 0, data = 0;
    if (size < 12) return FND_E_FORMAT;
    fnd_status st = read(ctx, 0, h, 12);
    if (st < 0) return st;
    if (!wav_tag(h, "RIFF") || !wav_tag(h + 8, "WAVE") || wav_u32(h + 4) != size - 8)
        return FND_E_FORMAT;
    uint32_t pos = 12;
    while (pos < size) {
        if (size - pos < 8) return FND_E_FORMAT;
        st = read(ctx, pos, h, 8); if (st < 0) return st;
        uint32_t n = wav_u32(h + 4); pos += 8;
        if (n > size - pos || (n & 1u) > size - pos - n) return FND_E_FORMAT;
        if (wav_tag(h, "fmt ")) {
            if (fmt++ || n < 16) return FND_E_FORMAT;
            st = read(ctx, pos, h, 16); if (st < 0) return st;
            if (wav_u16(h) != 1) return FND_E_UNSUPPORTED;
            w.channels = wav_u16(h + 2); w.rate = wav_u32(h + 4);
            w.align = wav_u16(h + 12); w.bits = wav_u16(h + 14);
            if ((w.channels != 1 && w.channels != 2) || (w.bits != 8 && w.bits != 16) ||
                w.rate < 8000 || w.rate > 192000) return FND_E_UNSUPPORTED;
            if (w.align != w.channels * (w.bits / 8) || wav_u32(h + 8) != w.rate * w.align)
                return FND_E_FORMAT;
        } else if (wav_tag(h, "data")) {
            if (data++) return FND_E_FORMAT;
            w.offset = pos; w.bytes = n;
        }
        pos += n + (n & 1u);
    }
    if (!fmt || !data || !w.bytes || w.bytes % w.align) return FND_E_FORMAT;
    *out = w; return FND_OK;
}

typedef struct {
    void *ctx; wav_read_at read; wav_info info;
    uint32_t position, remaining, used, available, phase, ended;
    uint8_t cache[4096], frame[4];
} wav_decoder;
static void wav_begin(wav_decoder *d, void *ctx, wav_read_at read, const wav_info *info) {
    d->ctx = ctx; d->read = read; d->info = *info;
    d->position = info->offset; d->remaining = info->bytes;
    d->used = d->available = d->ended = 0; d->phase = 48000;
}
/* Integer phase accumulator, zero-order hold resampling to 48 kHz. Source
 * reads are bounded, output is BE PCM16, independent of host endianness.
 * Append exactly one zero frame to clear the DAC's underrun hold value. */
static fnd_status wav_render(wav_decoder *d, uint8_t *out, uint32_t capacity, uint32_t *bytes) {
    uint32_t count = 0, stride = d->info.channels * 2;
    /* The native-rate PCM16 path must fit the real-time guest budget. Avoid
     * a per-frame phase machine when only an endian swap is needed. */
    if (d->info.rate == 48000 && d->info.bits == 16 && !d->ended) {
        uint32_t n = capacity - capacity % stride;
        if (n > d->remaining) n = d->remaining;
        if (n) {
            fnd_status st = d->read(d->ctx, d->position, out, n);
            if (st < 0) return st;
            for (uint32_t i = 0; i < n; i += 2) {
                uint8_t low = out[i]; out[i] = out[i + 1]; out[i + 1] = low;
            }
            d->position += n; d->remaining -= n; count = n;
        }
        if (!d->remaining && capacity - count >= stride) {
            for (uint32_t i = 0; i < stride; ++i) out[count++] = 0;
            d->ended = 1;
        }
        *bytes = count; return FND_OK;
    }
    while (!d->ended && capacity - count >= stride) {
        while (d->phase >= 48000 && !d->ended) {
            if (!d->remaining && d->used == d->available) { d->ended = 1; break; }
            if (d->used == d->available) {
                uint32_t n = d->remaining < sizeof d->cache ? d->remaining : sizeof d->cache;
                fnd_status st = d->read(d->ctx, d->position, d->cache, n);
                if (st < 0) return st;
                d->position += n; d->remaining -= n; d->used = 0; d->available = n;
            }
            const uint8_t *p = d->cache + d->used;
            if (d->info.bits == 16) {
                d->frame[0] = p[1]; d->frame[1] = p[0];
                if (stride == 4) { d->frame[2] = p[3]; d->frame[3] = p[2]; }
            } else {
                d->frame[0] = p[0] ^ 128u; d->frame[1] = 0;
                if (stride == 4) { d->frame[2] = p[1] ^ 128u; d->frame[3] = 0; }
            }
            d->used += d->info.align;
            d->phase -= 48000;
        }
        if (d->ended) d->frame[0] = d->frame[1] = d->frame[2] = d->frame[3] = 0;
        out[count] = d->frame[0]; out[count + 1] = d->frame[1];
        if (stride == 4) { out[count + 2] = d->frame[2]; out[count + 3] = d->frame[3]; }
        count += stride;
        d->phase += d->info.rate;
    }
    *bytes = count; return FND_OK;
}
#endif
