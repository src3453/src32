#ifndef FOUNDATION_SGC_H
#define FOUNDATION_SGC_H
#include <foundation/fnd.h>

/* /dev/sgc, spec_Foundation_API.md §9.3. FIFO words are big-endian SGC commands. */
#define FND_SGC_GET_STATUS 0x1200u
#define FND_SGC_SET_OUTPUT 0x1201u
#define FND_SGC_RESET 0x1202u
#define FND_SGC_FIFO_CLEAR 0x1203u

#define FND_SGC_STATUS_BUSY 0x01u
#define FND_SGC_STATUS_INVALID_CONFIG 0x02u
#define FND_SGC_STATUS_INVALID_SPRITE 0x04u
#define FND_SGC_STATUS_FIFO_FULL 0x08u
#define FND_SGC_STATUS_FIFO_ERROR 0x10u

#define FND_SGC_FIFO_WORDS 256u
#define FND_SGC_OP_SET_COLOR 0x01000000u
#define FND_SGC_OP_LINE 0x10000000u
#define FND_SGC_OP_RECT 0x11000000u
#define FND_SGC_OP_TRIANGLE 0x12000000u
#define FND_SGC_OP_GLYPH 0x13000000u
#define FND_SGC_OP_SCREEN_FILL 0x14000000u
#define FND_SGC_OP_NOP 0xFF000000u

typedef struct {
    uint32_t id, control, status, fifo_used, fifo_free, output_gp;
} fnd_sgc_status;

/* Header helpers use the existing fnd WAL's fnd_call; no new WAL ABI. */
static inline fnd_status fnd_sgc_get_status(fnd_handle h, uint32_t clear, fnd_sgc_status *out) {
    uint32_t args[4] = {clear, 0, 0, 0};
    fnd_sgc_status got;
    fnd_msg rep;
    fnd_status st = fnd_call(h, FND_SGC_GET_STATUS, args, 0, 0, &got, sizeof got, &rep);
    if (st == FND_OK) {
        if (rep.len != sizeof got) return FND_E_IO;
        *out = got;
    }
    return st;
}

static inline fnd_status fnd_sgc_set_output(fnd_handle h, uint32_t gp) {
    uint32_t args[4] = {gp, 0, 0, 0};
    return fnd_call(h, FND_SGC_SET_OUTPUT, args, 0, 0, 0, 0, 0);
}

static inline fnd_status fnd_sgc_reset(fnd_handle h) {
    return fnd_call(h, FND_SGC_RESET, 0, 0, 0, 0, 0, 0);
}

static inline fnd_status fnd_sgc_fifo_clear(fnd_handle h) {
    return fnd_call(h, FND_SGC_FIFO_CLEAR, 0, 0, 0, 0, 0, 0);
}

/* Queues as many whole words as the FIFO has room for; *done counts words.
 * Never blocks: FND_E_WOULD_BLOCK with *done == 0 means the FIFO is full. */
static inline fnd_status fnd_sgc_submit(fnd_handle h, const uint32_t *words, uint32_t count, uint32_t *done) {
    uint32_t total = 0;
    fnd_status st = FND_OK;
    while (total < count) {
        uint32_t chunk = count - total < FND_SGC_FIFO_WORDS ? count - total : FND_SGC_FIFO_WORDS;
        fnd_msg rep;
        st = fnd_call(h, FND_ST_WRITE, 0, words + total, chunk * 4u, 0, 0, &rep);
        if (st != FND_OK) break;
        total += rep.a0 / 4u;
        if (rep.a0 < chunk * 4u) break;
    }
    *done = total;
    return st == FND_E_WOULD_BLOCK && total ? FND_OK : st;
}
#endif
