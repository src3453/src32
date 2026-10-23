#ifndef FOUNDATION_FB_H
#define FOUNDATION_FB_H
#include <foundation/fnd.h>

/* /dev/fb0, spec_Foundation_API.md §9.1. Pixel bytes are in VDP order. */
#define FND_FB_GET_INFO 0x1100u
#define FND_FB_SET_FORMAT 0x1101u
#define FND_FB_CLUT_READ 0x1102u
#define FND_FB_CLUT_WRITE 0x1103u
#define FND_FB_INDEX8 0u
#define FND_FB_RGB555 1u
#define FND_FB_RGB888 2u

typedef struct {
    uint32_t width, height, stride, size, format, clut_entries;
} fnd_fb_info;

/* Header helpers use the existing fnd WAL's fnd_call; no new WAL ABI. */
static inline fnd_status fnd_fb_get_info(fnd_handle h, fnd_fb_info *info) {
    fnd_fb_info got;
    fnd_msg rep;
    fnd_status st = fnd_call(h, FND_FB_GET_INFO, 0, 0, 0, &got, sizeof got, &rep);
    if (st == FND_OK) {
        if (rep.len != sizeof got) return FND_E_IO;
        *info = got;
    }
    return st;
}

static inline fnd_status fnd_fb_set_format(fnd_handle h, uint32_t format) {
    uint32_t args[4] = {format, 0, 0, 0};
    return fnd_call(h, FND_FB_SET_FORMAT, args, 0, 0, 0, 0, 0);
}

static inline fnd_status fnd_fb_seek(fnd_handle h, uint32_t offset) {
    uint32_t args[4] = {FND_SEEK_SET, 0, offset, 0};
    return fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, 0);
}

static inline fnd_status fnd_fb_clut_read(fnd_handle h, uint32_t first, uint32_t count, void *rgb) {
    if (first > 256u || count > 256u - first) return FND_E_BAD_ARG;
    uint32_t args[4] = {first, count, 0, 0};
    fnd_msg rep;
    fnd_status st = fnd_call(h, FND_FB_CLUT_READ, args, 0, 0, rgb, count * 3u, &rep);
    return st == FND_OK && rep.len != count * 3u ? FND_E_IO : st;
}

static inline fnd_status fnd_fb_clut_write(fnd_handle h, uint32_t first, uint32_t count, const void *rgb) {
    if (first > 256u || count > 256u - first) return FND_E_BAD_ARG;
    uint32_t args[4] = {first, count, 0, 0};
    return fnd_call(h, FND_FB_CLUT_WRITE, args, rgb, count * 3u, 0, 0, 0);
}
#endif
