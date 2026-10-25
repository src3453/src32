/* Exclusive stream for both SGUs and shared PCM RAM. */
#include "internal.h"
#include <foundation/sgu.h>
#define BYTE(a) (*(volatile uint8_t *)(uintptr_t)(a))
static uint32_t opened, rights, position, dac_channels;
static uint32_t address(uint32_t off) {
    return off < FND_SGU_PCM ? 0x80020000u + off : 0x18000000u + off - FND_SGU_PCM;
}
static void silence(void) {
    for (uint32_t core = 0; core < 2; ++core)
        for (uint32_t ch = 0; ch < 8; ++ch) {
            uint32_t base = 0x80020800u + core * 0x1000u + ch * 32u;
            BYTE(base + 3) = 0;
            BYTE(base + 0x19) = 0;
        }
}
int32_t sgu_open(uint32_t flags) {
    if (!flags || (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE))) return FND_E_BAD_ARG;
    if (opened) return FND_E_BUSY;
    opened = 1; rights = flags; position = 0; dac_channels = 0;
    if (rights & FND_OPEN_WRITE) silence();
    return FND_OK;
}
void sgu_close(void) {
    if (rights & FND_OPEN_WRITE) silence();
    opened = 0;
}
int32_t sgu_call(struct fnd_proc *proc, const fnd_msg *m, fnd_msg *r, uint32_t cap) {
    uint8_t data[FND_IPC_MAX_DATA];
    switch (m->op) {
    case FND_SGU_DAC_CONFIG:
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (m->a0 != 1 && m->a0 != 2) return FND_E_BAD_ARG;
        silence(); dac_channels = 0;
        for (uint32_t ch = 0; ch < m->a0; ++ch) {
            uint32_t base = 0x80020800u + ch * 32u;
            BYTE(base) = 0; BYTE(base + 1) = 0;
            BYTE(base + 2) = 5;
            if (BYTE(base + 2) != 5) return FND_E_UNSUPPORTED;
            BYTE(base + 3) = 255;
            BYTE(base + 4) = m->a0 == 1 ? 0xFF : ch == 0 ? 0xF0 : 0x0F;
            BYTE(base + 5) = 0;
            BYTE(base + 10) = 0;
        }
        dac_channels = m->a0;
        return FND_OK;
    case FND_SGU_DAC_START:
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (!dac_channels) return FND_E_BAD_ARG;
        for (uint32_t ch = 0; ch < dac_channels; ++ch)
            BYTE(0x80020819u + ch * 32u) = 1;
        return FND_OK;
    case FND_SGU_DAC_SUBMIT: {
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (!dac_channels || m->len > FND_IPC_MAX_DATA || m->len % (dac_channels * 2u))
            return FND_E_BAD_ARG;
        uint32_t used = 0;
        for (uint32_t ch = 0; ch < dac_channels; ++ch) {
            uint32_t base = 0x80020800u + ch * 32u;
            /* Rendering can drain the FIFO between byte reads. Retry a
             * high-byte rollover rather than interpreting a torn count. */
            uint32_t hi, lo, again;
            do {
                hi = BYTE(base + 14); lo = BYTE(base + 15); again = BYTE(base + 14);
            } while (hi != again);
            uint32_t n = (hi << 8) | lo;
            if (n > used) used = n;
        }
        if (used > 2048u) return FND_E_IO;
        uint32_t frames = m->len / (dac_channels * 2u);
        if (frames > 2048u - used) frames = 2048u - used;
        uint32_t bytes = frames * dac_channels * 2u;
        if (bytes) mm_copy_from_user(data, proc, m->buf, bytes);
        for (uint32_t i = 0; i < frames; ++i)
            for (uint32_t ch = 0; ch < dac_channels; ++ch) {
                uint32_t base = 0x80020800u + ch * 32u;
                uint32_t off = (i * dac_channels + ch) * 2u;
                BYTE(base + 12) = data[off]; BYTE(base + 13) = data[off + 1];
            }
        r->a0 = bytes; r->a1 = used + frames;
        return m->len && !bytes ? FND_E_WOULD_BLOCK : FND_OK;
    }
    case FND_ST_SEEK: {
        int64_t off = (int64_t)(int32_t)m->a1 * 4294967296LL + m->a2;
        if (m->a0 > FND_SEEK_END) return FND_E_BAD_ARG;
        int64_t next = off + (m->a0 == FND_SEEK_CUR ? position : m->a0 == FND_SEEK_END ? FND_SGU_SIZE : 0);
        if (next < 0 || next > FND_SGU_SIZE) return FND_E_BAD_ARG;
        position = (uint32_t)next; r->a1 = position; return FND_OK;
    }
    case FND_ST_STAT: {
        if (cap < sizeof(fnd_stat_t)) return FND_E_LIMIT;
        fnd_stat_t st = {FND_TYPE_DEVICE, 0x36u, 0, 0, 0, FND_SGU_SIZE};
        mm_copy_to_user(proc, r->buf, &st, sizeof st); r->len = sizeof st; return FND_OK;
    }
    case FND_ST_SYNC: return FND_OK;
    case FND_ST_READ: case FND_ST_WRITE: {
        uint32_t write = m->op == FND_ST_WRITE;
        if (!(rights & (write ? FND_OPEN_WRITE : FND_OPEN_READ))) return FND_E_ACCESS;
        uint32_t n = write ? m->len : (m->a0 < cap ? m->a0 : cap);
        if (n > FND_IPC_MAX_DATA) n = FND_IPC_MAX_DATA;
        if (write && n > FND_SGU_SIZE - position) return FND_E_NO_SPACE;
        if (n > FND_SGU_SIZE - position) n = FND_SGU_SIZE - position;
        if (write) mm_copy_from_user(data, proc, m->buf, n);
        for (uint32_t i = 0; i < n; ++i) {
            uint32_t off = position + i;
            if (write) BYTE(address(off)) = data[i];
            else {
                /* Never trigger the read-side effect of PHASE_RESET. */
                uint32_t local = off & 0xFFFu;
                data[i] = off < FND_SGU_PCM && local >= 0x800u && local < 0x900u &&
                    (local & 31u) == 10u ? 0 : BYTE(address(off));
            }
        }
        position += n;
        if (write) r->a0 = n;
        else { mm_copy_to_user(proc, r->buf, data, n); r->len = n; }
        return FND_OK;
    }
    default: return FND_E_UNSUPPORTED;
    }
}
