/* Kernel bitmap/CLUT driver. User buffers always pass through mm_copy_*;
 * no VRAM or MMIO is mapped into the client. Calls run with IRQs disabled. */
#include "internal.h"
#include <foundation/fb.h>
#include <cpt32/bmc.h>

#define REG8(a) (*(volatile uint8_t *)(uintptr_t)(a))
#define VRAM 0x10000000u
#define CLUT (VRAM + 0x12C00u)
#define ENABLE 0x80000000u
#define DISPLAY 0x80000001u
#define FORMAT 0x80000004u
#define OVERLAY 0x8000F00Bu

static uint32_t opened, access_flags, position, format;
static uint8_t saved_enable, saved_display, saved_format, saved_overlay;
static uint8_t saved_clut[768];
/* INDEX8 overlaps the console's PCG character and attribute planes. */
static uint8_t saved_text[0x3000];
static const uint32_t bases[3] = {0, 0x100000u, 0x200000u};

static uint32_t size_bytes(void) { return 320u * 240u * (format + 1u); }

static void read_vram(uint8_t *data, uint32_t address, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i) data[i] = REG8(address + i);
}

static void write_vram(uint32_t address, const uint8_t *data, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i) REG8(address + i) = data[i];
}

int32_t fb_open(uint32_t flags) {
    if (!flags || (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE))) return FND_E_BAD_ARG;
    if (opened) return FND_E_BUSY;
    /* BMC's zero means unknown: fail closed instead of touching absent VRAM. */
    if (bmc_vram_bytes() < 0x12F00u) return FND_E_NO_MEDIA;
    read_vram(saved_clut, CLUT, sizeof saved_clut);
    read_vram(saved_text, VRAM, sizeof saved_text);
    saved_enable = REG8(ENABLE);
    saved_display = REG8(DISPLAY);
    saved_format = REG8(FORMAT);
    saved_overlay = REG8(OVERLAY);
    opened = 1;
    access_flags = flags;
    position = format = 0;
    /* Read-only opens inspect INDEX8/CLUT without changing display state. */
    if (flags & FND_OPEN_WRITE) {
        REG8(FORMAT) = 0;
        REG8(OVERLAY) = 0;
        REG8(DISPLAY) = 0;
        /* Common DISPLAY_ENABLE is active-high; PCG_DISABLE at F000 is
         * active-low, but is not the register used here. */
        REG8(ENABLE) = 1;
    }
    return FND_OK;
}

void fb_close(void) {
    if (access_flags & FND_OPEN_WRITE) {
        write_vram(VRAM, saved_text, sizeof saved_text);
        write_vram(CLUT, saved_clut, sizeof saved_clut);
        REG8(FORMAT) = saved_format;
        REG8(OVERLAY) = saved_overlay;
        REG8(DISPLAY) = saved_display;
        REG8(ENABLE) = saved_enable;
    }
    opened = 0;
}

/* r.buf is validated writable memory, cap is its capacity. The dispatcher
 * sets reply status and copies the reply struct after this returns. */
int32_t fb_call(struct fnd_proc *proc, const fnd_msg *m, fnd_msg *r, uint32_t cap) {
    uint8_t data[FND_IPC_MAX_DATA];
    uint32_t n, address;
    switch (m->op) {
    case FND_FB_GET_INFO: {
        fnd_fb_info info = {320, 240, 320u * (format + 1u), size_bytes(), format, 256};
        if (cap < sizeof info) return FND_E_LIMIT;
        mm_copy_to_user(proc, r->buf, &info, sizeof info);
        r->len = sizeof info;
        return FND_OK;
    }
    case FND_FB_SET_FORMAT:
        if (!(access_flags & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (m->a0 > FND_FB_RGB888) return FND_E_BAD_ARG;
        if (bmc_vram_bytes() < bases[m->a0] + 320u * 240u * (m->a0 + 1u)) return FND_E_NO_MEDIA;
        format = m->a0;
        position = 0;
        REG8(FORMAT) = (uint8_t)format;
        return FND_OK;
    case FND_ST_SEEK: {
        int64_t offset = (int64_t)(int32_t)m->a1 * 4294967296LL + m->a2;
        if (m->a0 > FND_SEEK_END) return FND_E_BAD_ARG;
        int64_t base = m->a0 == FND_SEEK_CUR ? position : m->a0 == FND_SEEK_END ? size_bytes() : 0;
        /* Check before addition to avoid signed overflow. */
        if (offset < -base || offset > (int64_t)size_bytes() - base) return FND_E_BAD_ARG;
        position = (uint32_t)(base + offset);
        r->a1 = position;
        return FND_OK;
    }
    case FND_ST_STAT: {
        fnd_stat_t stat = {0};
        if (cap < sizeof stat) return FND_E_LIMIT;
        stat.type = FND_TYPE_DEVICE;
        stat.mode = 0x0036u;
        stat.flags = FND_STAT_SEEKABLE;
        if (!(access_flags & FND_OPEN_WRITE)) stat.flags |= FND_STAT_READ_ONLY;
        stat.size_lo = size_bytes();
        mm_copy_to_user(proc, r->buf, &stat, sizeof stat);
        r->len = sizeof stat;
        return FND_OK;
    }
    case FND_ST_SYNC:
        return FND_OK; /* writes are immediate, not a VSync fence */
    case FND_ST_TRUNCATE:
        return access_flags & FND_OPEN_WRITE ? FND_E_UNSUPPORTED : FND_E_ACCESS;
    case FND_ST_READ:
    case FND_ST_WRITE:
        if (!(access_flags & (m->op == FND_ST_READ ? FND_OPEN_READ : FND_OPEN_WRITE))) return FND_E_ACCESS;
        address = VRAM + bases[format] + position;
        if (m->op == FND_ST_WRITE) {
            if (m->len > size_bytes() - position) return FND_E_NO_SPACE;
            mm_copy_from_user(data, proc, m->buf, m->len);
            write_vram(address, data, m->len);
            position += m->len;
            r->a0 = m->len;
        } else {
            n = m->a0 < cap ? m->a0 : cap;
            if (n > FND_IPC_MAX_DATA) n = FND_IPC_MAX_DATA;
            if (n > size_bytes() - position) n = size_bytes() - position;
            read_vram(data, address, n);
            mm_copy_to_user(proc, r->buf, data, n);
            position += n;
            r->len = n;
        }
        return FND_OK;
    case FND_FB_CLUT_READ:
    case FND_FB_CLUT_WRITE:
        if (!(access_flags & (m->op == FND_FB_CLUT_READ ? FND_OPEN_READ : FND_OPEN_WRITE))) return FND_E_ACCESS;
        if (m->a0 > 256u || m->a1 > 256u - m->a0) return FND_E_BAD_ARG;
        n = m->a1 * 3u;
        address = CLUT + m->a0 * 3u;
        if (m->op == FND_FB_CLUT_WRITE) {
            if (m->len != n) return FND_E_BAD_ARG;
            mm_copy_from_user(data, proc, m->buf, n);
            write_vram(address, data, n);
            r->a0 = n;
        } else {
            if (cap < n) return FND_E_LIMIT;
            read_vram(data, address, n);
            mm_copy_to_user(proc, r->buf, data, n);
            r->len = n;
        }
        return FND_OK;
    default:
        return FND_E_UNSUPPORTED;
    }
}
