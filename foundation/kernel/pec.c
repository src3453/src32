/* PeC byte streams. WOULD_BLOCK is internal: IPC parks an empty read and
 * retries it from the scheduler tick, with IRQs enabled between attempts. */
#include "internal.h"
#include <foundation/fnd.h>

#ifndef PEC_READ8
#define PEC_READ8(a) (*(volatile uint8_t *)(uintptr_t)(a))
#define PEC_WRITE8(a, v) (*(volatile uint8_t *)(uintptr_t)(a) = (v))
#endif

int32_t pec_open(uint32_t kind, uint32_t flags) {
    if (!flags || (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE))) return FND_E_BAD_ARG;
    if (kind != KDEV_UART0 && (flags & FND_OPEN_WRITE)) return FND_E_ACCESS;
    return FND_OK;
}

int32_t pec_call(struct fnd_proc *proc, uint32_t kind, uint32_t rights,
                 const fnd_msg *m, fnd_msg *r, uint32_t cap) {
    switch (m->op) {
    case FND_ST_READ: {
        uint8_t bytes[FND_IPC_MAX_DATA];
        if (!(rights & FND_OPEN_READ)) return FND_E_ACCESS;
        uint32_t n = m->a0 < cap ? m->a0 : cap;
        if (n > FND_IPC_MAX_DATA) n = FND_IPC_MAX_DATA;
        uint32_t done = 0;
        while (done < n) {
            if (kind == KDEV_UART0) {
                if (!(PEC_READ8(0x80040001u) & 2u)) break;
                bytes[done++] = PEC_READ8(0x80040000u);
            } else {
                if (kind == KDEV_TRNG && !(PEC_READ8(0x8004001Cu) & 1u)) break;
                uint32_t base = kind == KDEV_TRNG ? 0x80040018u : 0x80040010u;
                /* Reading lane 0 advances/latches one word. Consume all four
                 * lanes even when the request ends partway through the word. */
                for (uint32_t i = 0; i < 4u; ++i) {
                    uint8_t b = PEC_READ8(base + i);
                    if (done < n) bytes[done++] = b;
                }
            }
        }
        if (n && !done) return FND_E_WOULD_BLOCK;
        mm_copy_to_user(proc, r->buf, bytes, done);
        r->len = done;
        return FND_OK;
    }
    case FND_ST_WRITE: {
        uint8_t bytes[FND_IPC_MAX_DATA];
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        if (kind != KDEV_UART0) return FND_E_ACCESS;
        mm_copy_from_user(bytes, proc, m->buf, m->len);
        /* PeC UART DATA enqueues bytes, as debug_write does. TX_READY reports
         * queue state, not a requirement for accepting another byte. */
        for (uint32_t i = 0; i < m->len; ++i) PEC_WRITE8(0x80040000u, bytes[i]);
        r->a0 = m->len;
        return FND_OK;
    }
    case FND_ST_STAT: {
        if (cap < sizeof(fnd_stat_t)) return FND_E_LIMIT;
        fnd_stat_t st = {0};
        st.type = FND_TYPE_DEVICE;
        st.mode = kind == KDEV_UART0 ? 0x36u : 0x24u;
        st.flags = kind == KDEV_UART0 ? 0u : FND_STAT_READ_ONLY;
        mm_copy_to_user(proc, r->buf, &st, sizeof st);
        r->len = sizeof st;
        return FND_OK;
    }
    case FND_ST_SYNC:
        return FND_OK;
    default:
        return FND_E_UNSUPPORTED;
    }
}
