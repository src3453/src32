/* Exclusive stream for the SGC 2D command FIFO. User data always passes through
 * mm_copy_*; no MMIO is mapped into the client. Calls run with IRQs disabled and
 * never wait: the FIFO drains once per VDP frame, so a full FIFO is reported. */
#include "internal.h"
#include <foundation/sgc.h>
#include <cpt32/bmc.h>

#define REG(off) (*(volatile uint32_t *)(uintptr_t)(0x80010000u + (off)))
#define ID 0x00u
#define CONTROL 0x04u
#define STATUS 0x08u
#define SAT_BASE 0x10u
#define SPRITE_COUNT 0x14u
#define OUTPUT_GP 0x18u
#define FIFO_DATA 0x20u
#define FIFO_STATUS 0x24u
#define FIFO_CONTROL 0x28u
#define CONTROL_ENABLE 1u
#define CONTROL_RESET 2u
#define SGC_ID 0x53474331u
#define STATUS_MASK 0x1Eu /* sticky bits; BUSY is derived */

static uint32_t opened, rights;
static uint32_t saved_control, saved_sat, saved_count, saved_gp;

/* RESET restores gp 1 and clears the command plane, then the stream state is
 * the one fb-less clients expect: enabled, no sprites, empty FIFO. */
static void reset_enabled(void) {
    REG(CONTROL) = CONTROL_RESET;
    REG(CONTROL) = CONTROL_ENABLE;
}

int32_t sgc_open(uint32_t flags) {
    if (!flags || (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE))) return FND_E_BAD_ARG;
    if (opened) return FND_E_BUSY;
    if (REG(ID) != SGC_ID) return FND_E_NO_MEDIA;
    opened = 1;
    rights = flags;
    if (flags & FND_OPEN_WRITE) {
        saved_control = REG(CONTROL);
        saved_sat = REG(SAT_BASE);
        saved_count = REG(SPRITE_COUNT);
        saved_gp = REG(OUTPUT_GP);
        reset_enabled();
    }
    return FND_OK;
}

void sgc_close(void) {
    if (rights & FND_OPEN_WRITE) {
        /* RESET drops the FIFO and every drawn pixel; then put back the registers. */
        REG(CONTROL) = CONTROL_RESET;
        REG(SAT_BASE) = saved_sat;
        REG(SPRITE_COUNT) = saved_count;
        REG(OUTPUT_GP) = saved_gp;
        REG(CONTROL) = saved_control & CONTROL_ENABLE;
    }
    opened = 0;
}

int32_t sgc_call(struct fnd_proc *proc, const fnd_msg *m, fnd_msg *r, uint32_t cap) {
    switch (m->op) {
    case FND_SGC_GET_STATUS: {
        if (m->a0 && !(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (m->a0 & ~STATUS_MASK) return FND_E_BAD_ARG;
        if (cap < sizeof(fnd_sgc_status)) return FND_E_LIMIT;
        uint32_t fifo = REG(FIFO_STATUS);
        fnd_sgc_status st = {REG(ID), REG(CONTROL), REG(STATUS), fifo >> 16, fifo & 0xFFFFu, REG(OUTPUT_GP)};
        if (m->a0) REG(STATUS) = m->a0; /* write-1-to-clear, after the snapshot */
        mm_copy_to_user(proc, r->buf, &st, sizeof st);
        r->len = sizeof st;
        return FND_OK;
    }
    case FND_SGC_SET_OUTPUT:
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (m->a0 > 7u) return FND_E_BAD_ARG;
        REG(OUTPUT_GP) = m->a0;
        return FND_OK;
    case FND_SGC_RESET:
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        reset_enabled();
        return FND_OK;
    case FND_SGC_FIFO_CLEAR:
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        REG(FIFO_CONTROL) = 1u;
        return FND_OK;
    case FND_ST_STAT: {
        if (cap < sizeof(fnd_stat_t)) return FND_E_LIMIT;
        fnd_stat_t st = {FND_TYPE_DEVICE, 0x36u, 0, 0, 0, 0};
        if (!(rights & FND_OPEN_WRITE)) st.flags |= FND_STAT_READ_ONLY;
        mm_copy_to_user(proc, r->buf, &st, sizeof st);
        r->len = sizeof st;
        return FND_OK;
    }
    case FND_ST_SYNC:
        return FND_OK; /* commands run on the next VDP frame; there is no fence */
    case FND_ST_WRITE: {
        uint32_t words[FND_IPC_MAX_DATA / 4u];
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (m->len & 3u) return FND_E_BAD_ARG;
        uint32_t n = m->len / 4u;
        if (!n) {
            r->a0 = 0;
            return FND_OK;
        }
        uint32_t room = REG(FIFO_STATUS) & 0xFFFFu;
        if (!room) return FND_E_WOULD_BLOCK;
        if (n > room) n = room; /* whole words only, so no partial lane latch is left */
        mm_copy_from_user(words, proc, m->buf, n * 4u);
        for (uint32_t i = 0; i < n; ++i) REG(FIFO_DATA) = words[i];
        r->a0 = n * 4u;
        return FND_OK;
    }
    default:
        return FND_E_UNSUPPORTED;
    }
}
