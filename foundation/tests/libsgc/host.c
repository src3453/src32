/* libsgc on the host against a model of /dev/sgc: a 256-word FIFO that takes
 * whole words, reports WOULD_BLOCK when full and drains when the program
 * sleeps (one VDP frame per tick). Compiled together with lib/sgc/sgc.c. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <foundation/libsgc.h>
#include <foundation/fb.h>
#include <foundation/sys.h>

static uint32_t fifo_used, drained[65536], drained_n, sleeps, closes, fifo_clears, resets;
static uint32_t submit_calls, partial_calls, would_block_calls;
static uint8_t clut[256 * 3];
static uint32_t fb_format = FND_FB_INDEX8, fb_width = 320, fb_open_ok = 1, sgc_open_ok = 1;
static uint32_t sleep_drains = 1;

uint32_t cpt32_syscall(uint32_t n, uint32_t c, uint32_t a, uint32_t b, uint32_t d, uint32_t e) {
    (void)c; (void)a; (void)b; (void)d; (void)e;
    assert(n == FND_SYS_SLEEP);
    ++sleeps;
    if (sleep_drains) {
        for (uint32_t i = 0; i < fifo_used; ++i) drained_n++;
        fifo_used = 0;
    }
    return 0;
}

fnd_status fnd_file_open(const char *path, uint32_t flags, uint32_t mode, fnd_handle *out) {
    (void)mode;
    assert(flags == FND_OPEN_WRITE);
    if (!strcmp(path, "/dev/sgc")) { if (!sgc_open_ok) return FND_E_BUSY; *out = 7; return FND_OK; }
    assert(!strcmp(path, "/dev/fb0"));
    if (!fb_open_ok) return FND_E_NOT_FOUND;
    *out = 9;
    return FND_OK;
}
fnd_status fnd_close(fnd_handle h) { assert(h == 7 || h == 9); ++closes; return FND_OK; }
fnd_status fnd_call(fnd_handle h, uint32_t op, const uint32_t args[4], const void *data, uint32_t len,
                    void *rep_data, uint32_t rep_cap, fnd_msg *reply) {
    if (!h) return FND_E_BAD_HANDLE;
    fnd_msg rep = {0};
    if (h == 9) {
        if (op == FND_FB_GET_INFO) {
            fnd_fb_info info = {fb_width, 240, fb_width, fb_width * 240, fb_format, 256};
            assert(rep_cap >= sizeof info);
            memcpy(rep_data, &info, sizeof info);
            rep.len = sizeof info;
        } else {
            assert(op == FND_FB_CLUT_WRITE);
            memcpy(clut + args[0] * 3, data, len);
        }
    } else if (op == FND_ST_WRITE) {
        ++submit_calls;
        assert(!(len & 3) && len <= 1024);
        uint32_t room = FND_SGC_FIFO_WORDS - fifo_used, n = len / 4;
        if (!room) { ++would_block_calls; return FND_E_WOULD_BLOCK; }
        if (n > room) n = room, ++partial_calls;
        for (uint32_t i = 0; i < n; ++i) {
            uint32_t w;
            memcpy(&w, (const uint8_t *)data + i * 4, 4);
            assert(drained_n + fifo_used + i < 65536);
            drained[drained_n + fifo_used + i] = w;
        }
        fifo_used += n;
        rep.a0 = n * 4;
    } else if (op == FND_SGC_FIFO_CLEAR) {
        ++fifo_clears;
        fifo_used = 0;
    } else if (op == FND_SGC_RESET) {
        ++resets;
        fifo_used = 0;
    } else if (op == FND_SGC_SET_OUTPUT) {
        assert(args[0] <= 7);
    } else if (op == FND_SGC_GET_STATUS) {
        fnd_sgc_status st = {0x53474331u, 1, 0x10, fifo_used, FND_SGC_FIFO_WORDS - fifo_used, 1};
        memcpy(rep_data, &st, sizeof st);
        rep.len = sizeof st;
    } else {
        assert(!"unexpected op");
    }
    if (reply) *reply = rep;
    return FND_OK;
}

static uint32_t buf[64];
static cpt_sgc g;
static void reset_model(void) {
    fifo_used = drained_n = sleeps = closes = fifo_clears = resets = 0;
    submit_calls = partial_calls = would_block_calls = 0;
}
/* Everything the device has taken so far, in order, including the FIFO. */
static uint32_t taken(void) { return drained_n + fifo_used; }
static void open_ctx(uint32_t cap) {
    reset_model();
    assert(cpt_sgc_open(&g, buf, cap) == FND_OK);
}
#define W(i) drained[i]
#define OK(x) assert((x) == FND_OK)

int main(void) {
    /* Open and argument checks. */
    assert(cpt_sgc_open(&g, buf, CPT_SGC_MAX_COMMAND - 1) == FND_E_BAD_ARG);
    assert(cpt_sgc_open(&g, 0, 64) == FND_E_BAD_ARG);
    sgc_open_ok = 0;
    assert(cpt_sgc_open(&g, buf, 64) == FND_E_BUSY);
    sgc_open_ok = 1;
    open_ctx(64);
    assert(cpt_sgc_pending(&g) == 0);
    assert(cpt_sgc_set_color(&g, 64) == FND_E_BAD_ARG);
    assert(cpt_sgc_rect(&g, 0, 0, 1, 1, 64) == FND_E_BAD_ARG);
    assert(cpt_sgc_glyph(&g, 0x10000, 0, 0, 1) == FND_E_BAD_ARG);
    assert(cpt_sgc_pending(&g) == 0);

    /* Encoding; a repeated color is not repeated on the wire. */
    OK(cpt_sgc_rect(&g, 20, 30, 10, 6, 5));
    OK(cpt_sgc_rect(&g, -3, 4, 2, 2, 5));
    OK(cpt_sgc_line(&g, 1, 2, 3, -4, 5));
    OK(cpt_sgc_triangle(&g, 0, 0, 10, 0, 0, 10, 6));
    OK(cpt_sgc_glyph(&g, 0x41, 9, 8, 6));
    OK(cpt_sgc_clear(&g, 0));
    assert(cpt_sgc_pending(&g) == 1 + 5 + 5 + 5 + 1 + 7 + 4 + 1 + 1);
    OK(cpt_sgc_flush(&g));
    assert(cpt_sgc_pending(&g) == 0);
    static const uint32_t expect[] = {
        0x01000005, 0x11000000, 20, 30, 30, 36,
        0x11000000, 0xFFFD, 4, 0xFFFF, 6,
        0x10000000, 1, 2, 3, 0xFFFC,
        0x01000006, 0x12000000, 0, 0, 10, 0, 0, 10,
        0x13000000, 0x41, 9, 8,
        0x01000000, 0x14000000,
    };
    assert(taken() == sizeof expect / 4);
    for (uint32_t i = 0; i < sizeof expect / 4; ++i) assert(W(i) == expect[i]);

    /* Degenerate shapes queue nothing: the hardware would report FIFO_ERROR. */
    uint32_t before = cpt_sgc_pending(&g);
    OK(cpt_sgc_rect(&g, 0, 0, 0, 5, 1));
    OK(cpt_sgc_rect(&g, 0, 0, 5, -1, 1));
    OK(cpt_sgc_triangle(&g, 0, 0, 5, 5, 10, 10, 1));
    OK(cpt_sgc_triangle(&g, 3, 3, 3, 3, 3, 3, 1));
    assert(cpt_sgc_pending(&g) == before);
    /* Huge values are clamped to the signed 16 bit range. */
    drained_n = fifo_used = 0;
    OK(cpt_sgc_rect(&g, -100000, 100000, 1000000, 0x7FFFFFFF, 1));
    OK(cpt_sgc_flush(&g));
    assert(W(0) == 0x01000001 && W(1) == 0x11000000 && W(2) == 0x8000 && W(3) == 0x7FFF);
    assert(W(4) == 0x7FFF && W(5) == 0x7FFF); /* x + w and y + h clamp too */

    /* A command never straddles the buffer end, and a full buffer first tries to flush. */
    cpt_sgc_close(&g);
    open_ctx(8);
    OK(cpt_sgc_rect(&g, 0, 0, 1, 1, 1)); /* 6 words */
    assert(cpt_sgc_pending(&g) == 6);
    OK(cpt_sgc_rect(&g, 0, 0, 1, 1, 2)); /* needs 6 more: flushes the first */
    assert(cpt_sgc_pending(&g) == 6 && taken() == 6);
    OK(cpt_sgc_flush(&g));
    assert(taken() == 12 && W(6) == 0x01000002);

    /* A full FIFO: flush is OK but moves nothing; the command is refused whole. */
    sleep_drains = 0;
    while (taken() < FND_SGC_FIFO_WORDS - 3) OK(cpt_sgc_rect(&g, 0, 0, 1, 1, 1 + (taken() & 1)));
    OK(cpt_sgc_flush(&g));
    uint32_t pend, words = taken();
    uint32_t calls = 0;
    for (;;) { /* fill FIFO, then buffer, then refuse */
        int32_t st = cpt_sgc_rect(&g, 0, 0, 1, 1, 1 + (calls & 1));
        ++calls;
        if (st == FND_E_WOULD_BLOCK) break;
        assert(st == FND_OK && calls < 100);
    }
    pend = cpt_sgc_pending(&g);
    assert(fifo_used == FND_SGC_FIFO_WORDS && pend > 0 && pend <= 8);
    assert(partial_calls >= 1);
    uint32_t seen = taken();
    OK(cpt_sgc_flush(&g)); /* WOULD_BLOCK from the device is not an error */
    assert(cpt_sgc_pending(&g) == pend && taken() == seen && would_block_calls >= 1);
    assert(cpt_sgc_rect(&g, 0, 0, 1, 1, 1) == FND_E_WOULD_BLOCK && cpt_sgc_pending(&g) == pend);
    (void)words;
    /* flush_wait drains it once the frame passes; the stream stays intact. */
    sleep_drains = 1;
    OK(cpt_sgc_flush_wait(&g));
    assert(cpt_sgc_pending(&g) == 0 && sleeps >= 1);
    for (uint32_t i = 0, n = 0; n < taken();) { /* every word is part of a complete command */
        uint32_t w = W(n);
        uint32_t len = (w >> 24) == 0x01 ? 1 : 5;
        assert((w >> 24) == 0x01 || (w >> 24) == 0x11);
        n += len;
        i = n;
        assert(i <= taken());
    }

    /* A device that never drains times out instead of spinning forever. */
    sleep_drains = 0;
    for (calls = 0; cpt_sgc_rect(&g, 0, 0, 1, 1, 1 + (calls & 1)) == FND_OK; ++calls) {}
    OK(cpt_sgc_flush(&g));
    assert(cpt_sgc_flush_wait(&g) == FND_E_TIMEOUT && sleeps >= CPT_SGC_WAIT_TICKS);
    sleep_drains = 1;

    /* cancel drops unsent words and forgets the color. */
    OK(cpt_sgc_cancel(&g));
    assert(cpt_sgc_pending(&g) == 0 && fifo_clears == 1);
    drained_n = 0; fifo_used = 0;
    OK(cpt_sgc_rect(&g, 0, 0, 1, 1, 1));
    OK(cpt_sgc_flush(&g));
    assert(W(0) == 0x01000001); /* SET_COLOR sent again even though 1 was the last color */
    OK(cpt_sgc_reset(&g));
    assert(resets == 1);

    /* raw: all or nothing, and the color is unknown afterwards. */
    cpt_sgc_close(&g);
    open_ctx(8);
    uint32_t nops[9] = {FND_SGC_OP_NOP};
    assert(cpt_sgc_raw(&g, nops, 9) == FND_E_LIMIT);
    OK(cpt_sgc_raw(&g, nops, 3));
    OK(cpt_sgc_rect(&g, 0, 0, 1, 1, 2));
    OK(cpt_sgc_raw(&g, nops, 1));
    OK(cpt_sgc_rect(&g, 0, 0, 1, 1, 2));
    OK(cpt_sgc_flush_wait(&g));
    assert(taken() == 3 + 6 + 1 + 6 && W(9) == FND_SGC_OP_NOP && W(10) == 0x01000002);

    /* Status and output pass through. */
    fnd_sgc_status st;
    OK(cpt_sgc_status(&g, 0x10, &st));
    assert(st.id == 0x53474331u && (st.status & 0x10));
    OK(cpt_sgc_set_output(&g, 3));

    /* display_init: CLUT written, fb0 kept until close, wrong mode refused. */
    const uint8_t rgb[6] = {0, 0, 0, 255, 128, 1};
    assert(cpt_sgc_display_init(&g, 250, 7, rgb) == FND_E_BAD_ARG);
    assert(cpt_sgc_display_init(&g, 0, 2, 0) == FND_E_BAD_ARG);
    fb_open_ok = 0;
    assert(cpt_sgc_display_init(&g, 0, 2, rgb) == FND_E_NOT_FOUND && g.fb == 0);
    fb_open_ok = 1;
    fb_format = FND_FB_RGB555;
    uint32_t c0 = closes;
    assert(cpt_sgc_display_init(&g, 0, 2, rgb) == FND_E_UNSUPPORTED && g.fb == 0 && closes == c0 + 1);
    fb_format = FND_FB_INDEX8;
    fb_width = 640;
    assert(cpt_sgc_display_init(&g, 0, 2, rgb) == FND_E_UNSUPPORTED);
    fb_width = 320;
    OK(cpt_sgc_display_init(&g, 4, 2, rgb));
    assert(g.fb == 9 && clut[12] == 0 && clut[15] == 255 && clut[16] == 128 && clut[17] == 1);
    OK(cpt_sgc_display_init(&g, 0, 0, 0)); /* again, no CLUT: reuses the open fb0 */
    assert(g.fb == 9);
    c0 = closes;
    OK(cpt_sgc_close(&g));
    assert(closes == c0 + 2 && g.dev == 0 && g.fb == 0);
    /* Ellipse commands preserve signed centers, validate radii before queuing,
     * and keep the existing color coalescing/whole-command guarantees. */
    open_ctx(32);
    assert(cpt_sgc_ellipse(&g, 1, 2, -1, 3, 4) == FND_E_BAD_ARG);
    assert(cpt_sgc_fill_ellipse(&g, 1, 2, 3, 32768, 4) == FND_E_BAD_ARG);
    assert(cpt_sgc_pending(&g) == 0);
    OK(cpt_sgc_ellipse(&g, -1, 2, 3, 4, 5));
    OK(cpt_sgc_fill_ellipse(&g, 319, 239, 0, 0, 5));
    OK(cpt_sgc_flush(&g));
    assert(taken() == 11 && W(0) == 0x01000005u && W(1) == FND_SGC_OP_ELLIPSE);
    assert(W(2) == 0xffff && W(3) == 2 && W(4) == 3 && W(5) == 4);
    assert(W(6) == FND_SGC_OP_FILL_ELLIPSE && W(7) == 319 && W(8) == 239 && W(9) == 0 && W(10) == 0);
    OK(cpt_sgc_close(&g));
    puts("libsgc host passed");
    return 0;
}
