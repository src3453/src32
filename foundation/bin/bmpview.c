/* bmpview file.bmp: shows an uncompressed BMP (8, 24 or 32 bits per pixel) on
 * /dev/fb0 (doc/spec_Foundation_API.md §9.1) until a key is pressed. An 8-bit
 * image uses INDEX8 with its own palette in the CLUT; the others use RGB888.
 * Images larger than 320x240 are cropped around the centre, smaller ones are
 * centred on black (index 0 for 8-bit images). */
#include "cmd.h"
#include <foundation/fb.h>

#define SCREEN_W 320u
#define SCREEN_H 240u
#define LIMIT 8192u /* width and height, so that the offset arithmetic cannot wrap */

static uint32_t le16(const uint8_t *p) { return p[0] | (uint32_t)p[1] << 8; }
static uint32_t le32(const uint8_t *p) { return le16(p) | le16(p + 2) << 16; }

static fnd_status read_at(fnd_handle h, uint32_t offset, uint8_t *buf, uint32_t len) {
    uint32_t args[4] = {FND_SEEK_SET, 0, offset, 0};
    fnd_status st = fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, 0);
    while (st == FND_OK && len) {
        uint32_t got = 0;
        st = fnd_read(h, buf, len, &got);
        if (st == FND_OK && got == 0) return FND_E_FORMAT; /* truncated file */
        buf += got;
        len -= got;
    }
    return st;
}

static fnd_status show(fnd_handle file) {
    uint8_t head[54], palette[768], src[SCREEN_W * 4u], out[SCREEN_W * 3u];
    fnd_status st = read_at(file, 0, head, sizeof head);
    if (st != FND_OK) return st;
    uint32_t pixels = le32(head + 10), dib = le32(head + 14);
    uint32_t w = le32(head + 18), height = le32(head + 22), bpp = le16(head + 28), comp = le32(head + 30);
    int bottom_up = !(height & 0x80000000u);
    uint32_t h = bottom_up ? height : 0u - height;
    if (head[0] != 'B' || head[1] != 'M' || dib < 40u || le16(head + 26) != 1u) return FND_E_FORMAT;
    if (w == 0 || h == 0 || w > LIMIT || h > LIMIT || (w & 0x80000000u)) return FND_E_FORMAT;
    if (!(bpp == 8 || bpp == 24 || (bpp == 32 && (comp == 0 || comp == 3)))) return FND_E_UNSUPPORTED;
    if (bpp != 32 && comp != 0) return FND_E_UNSUPPORTED; /* RLE and the like */
    uint32_t bytes = bpp / 8u, stride = (w * bytes + 3u) & ~3u;

    if (bpp == 8) {
        uint32_t used = le32(head + 46);
        if (used == 0 || used > 256u) used = 256u;
        uint8_t entry[1024];
        st = read_at(file, 14u + dib, entry, used * 4u);
        if (st != FND_OK) return st;
        for (uint32_t i = 0; i < 256u; ++i) {
            palette[i * 3u] = i < used ? entry[i * 4u + 2u] : 0;
            palette[i * 3u + 1u] = i < used ? entry[i * 4u + 1u] : 0;
            palette[i * 3u + 2u] = i < used ? entry[i * 4u] : 0;
        }
    }

    fnd_handle fb;
    st = fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb);
    if (st != FND_OK) return st;
    fnd_fb_info info;
    st = fnd_fb_get_info(fb, &info);
    if (st == FND_OK && (info.width != SCREEN_W || info.height != SCREEN_H)) st = FND_E_UNSUPPORTED;
    if (st == FND_OK) st = fnd_fb_set_format(fb, bpp == 8 ? FND_FB_INDEX8 : FND_FB_RGB888);
    if (st == FND_OK && bpp == 8) st = fnd_fb_clut_write(fb, 0, 256, palette);

    uint32_t copy = w < SCREEN_W ? w : SCREEN_W, dst_x = (SCREEN_W - copy) / 2u, src_x = (w - copy) / 2u;
    uint32_t rows = h < SCREEN_H ? h : SCREEN_H, dst_y = (SCREEN_H - rows) / 2u, src_y = (h - rows) / 2u;
    uint32_t out_bytes = bpp == 8 ? 1u : 3u;
    if (st == FND_OK) st = fnd_fb_seek(fb, 0);
    for (uint32_t y = 0; y < SCREEN_H && st == FND_OK; ++y) {
        for (uint32_t i = 0; i < SCREEN_W * out_bytes; ++i) out[i] = 0;
        if (y >= dst_y && y < dst_y + rows) {
            uint32_t sy = src_y + (y - dst_y), line = bottom_up ? h - 1u - sy : sy;
            st = read_at(file, pixels + line * stride + src_x * bytes, src, copy * bytes);
            for (uint32_t x = 0; x < copy && st == FND_OK; ++x) {
                const uint8_t *s = src + x * bytes;
                uint8_t *d = out + (dst_x + x) * out_bytes;
                if (bpp == 8) {
                    d[0] = s[0];
                } else { /* BGR(A) -> RGB */
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                }
            }
        }
        if (st == FND_OK) st = fnd_write(fb, out, SCREEN_W * out_bytes, 0);
    }

    fnd_key_t key;
    while (st == FND_OK && fnd_con_pollkey(&key) == FND_OK) { /* drop keys typed before */ }
    while (st == FND_OK) {
        fnd_status ks = fnd_con_pollkey(&key);
        if (ks < 0) st = ks;
        else if (ks == FND_OK) break;
        else fnd_sys_sleep(1u);
    }
    fnd_close(fb); /* the console comes back */
    return st;
}

static int32_t bmpview(uint32_t argc) {
    if (argc != 2u) return cmd_usage("bmpview file.bmp (any key to quit)");
    fnd_handle file;
    fnd_status st = fnd_file_open(fnd_argv(1), FND_OPEN_READ, FND_MODE_DEFAULT, &file);
    if (st != FND_OK) return cmd_report("bmpview", fnd_argv(1), st);
    st = show(file);
    fnd_close(file);
    return st < 0 ? cmd_report("bmpview", fnd_argv(1), st) : FND_OK;
}

CMD_MAIN(bmpview)
