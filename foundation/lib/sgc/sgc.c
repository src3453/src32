/* libsgc (foundation/include/foundation/libsgc.h): SGC command builder and
 * FIFO feeder over /dev/sgc. Built as the WAL "sgc" 1.0; it imports the WAL
 * "fnd" for the IPC calls and keeps no static state. */
#include <foundation/libsgc.h>
#include <foundation/fb.h>
#include <foundation/sys.h>

#define UNKNOWN_COLOR 0xFFFFFFFFu

static int32_t clamp16(int32_t v) { return v < -32768 ? -32768 : v > 32767 ? 32767 : v; }
static uint32_t coord(int32_t v) { return (uint32_t)clamp16(v) & 0xFFFFu; }

/* Moves the unsent words to the front of the buffer. */
static void compact(cpt_sgc *g) {
    if (!g->head) return;
    uint32_t n = g->len - g->head;
    for (uint32_t i = 0; i < n; ++i) g->buf[i] = g->buf[g->head + i];
    g->head = 0;
    g->len = n;
}

int32_t cpt_sgc_flush(cpt_sgc *g) {
    if (g->head == g->len) {
        g->head = g->len = 0;
        return FND_OK;
    }
    uint32_t done = 0;
    int32_t st = fnd_sgc_submit(g->dev, g->buf + g->head, g->len - g->head, &done);
    g->head += done;
    if (g->head == g->len) g->head = g->len = 0;
    return st == FND_E_WOULD_BLOCK ? FND_OK : st;
}

uint32_t cpt_sgc_pending(const cpt_sgc *g) { return g->len - g->head; }

/* Room for n more words, flushing once if the buffer is too full. */
static int32_t reserve(cpt_sgc *g, uint32_t n) {
    if (g->cap - g->len >= n) return FND_OK;
    compact(g);
    if (g->cap - g->len >= n) return FND_OK;
    int32_t st = cpt_sgc_flush(g);
    if (st < 0) return st;
    compact(g);
    return g->cap - g->len >= n ? FND_OK : FND_E_WOULD_BLOCK;
}

/* Reserves a command of n words plus the SET_COLOR it needs, then appends
 * the SET_COLOR. Returns the status; on FND_OK the caller writes n words. */
static int32_t begin(cpt_sgc *g, uint32_t color, uint32_t n) {
    if (color >= CPT_SGC_COLORS) return FND_E_BAD_ARG;
    uint32_t extra = color != g->color;
    int32_t st = reserve(g, n + extra);
    if (st < 0) return st;
    if (extra) {
        g->buf[g->len++] = FND_SGC_OP_SET_COLOR | color;
        g->color = color;
    }
    return FND_OK;
}

int32_t cpt_sgc_open(cpt_sgc *g, uint32_t *buf, uint32_t cap) {
    if (!g || !buf || cap < CPT_SGC_MAX_COMMAND) return FND_E_BAD_ARG;
    g->dev = g->fb = 0;
    g->buf = buf;
    g->cap = cap;
    g->head = g->len = 0;
    g->color = UNKNOWN_COLOR; /* the driver resets the SGC, whose color is then unspecified */
    return fnd_file_open("/dev/sgc", FND_OPEN_WRITE, FND_MODE_DEFAULT, &g->dev);
}

int32_t cpt_sgc_close(cpt_sgc *g) {
    int32_t st = FND_OK, r;
    g->head = g->len = 0;
    if (g->dev && (r = fnd_close(g->dev)) < 0) st = r;
    if (g->fb && (r = fnd_close(g->fb)) < 0 && st >= 0) st = r;
    g->dev = g->fb = 0;
    return st;
}

int32_t cpt_sgc_display_init(cpt_sgc *g, uint32_t first, uint32_t count, const uint8_t *rgb) {
    if ((count && !rgb) || first > 256u || count > 256u - first) return FND_E_BAD_ARG;
    fnd_handle fb = g->fb;
    if (!fb) {
        int32_t st = fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb);
        if (st < 0) return st;
    }
    fnd_fb_info info;
    int32_t st = fnd_fb_get_info(fb, &info);
    if (st == FND_OK && (info.width != CPT_SGC_WIDTH || info.height != CPT_SGC_HEIGHT || info.format != FND_FB_INDEX8))
        st = FND_E_UNSUPPORTED;
    if (st == FND_OK && count) st = fnd_fb_clut_write(fb, first, count, rgb);
    if (st < 0) {
        if (!g->fb) fnd_close(fb);
        return st;
    }
    g->fb = fb;
    return FND_OK;
}

int32_t cpt_sgc_flush_wait(cpt_sgc *g) {
    uint32_t idle = 0;
    for (;;) {
        uint32_t before = g->len - g->head;
        int32_t st = cpt_sgc_flush(g);
        if (st < 0) return st;
        if (g->head == g->len) return FND_OK;
        idle = g->len - g->head < before ? 0 : idle + 1;
        if (idle >= CPT_SGC_WAIT_TICKS) return FND_E_TIMEOUT;
        fnd_sys_sleep(1);
    }
}

int32_t cpt_sgc_cancel(cpt_sgc *g) {
    g->head = g->len = 0;
    g->color = UNKNOWN_COLOR; /* queued SET_COLORs may or may not have run */
    return fnd_sgc_fifo_clear(g->dev);
}

int32_t cpt_sgc_reset(cpt_sgc *g) {
    g->head = g->len = 0;
    g->color = UNKNOWN_COLOR;
    return fnd_sgc_reset(g->dev);
}

int32_t cpt_sgc_set_output(cpt_sgc *g, uint32_t gp) { return fnd_sgc_set_output(g->dev, gp); }

int32_t cpt_sgc_status(cpt_sgc *g, uint32_t clear, fnd_sgc_status *out) {
    return fnd_sgc_get_status(g->dev, clear, out);
}

int32_t cpt_sgc_set_color(cpt_sgc *g, uint32_t color) { return begin(g, color, 0); }

int32_t cpt_sgc_clear(cpt_sgc *g, uint32_t color) {
    int32_t st = begin(g, color, 1);
    if (st < 0) return st;
    g->buf[g->len++] = FND_SGC_OP_SCREEN_FILL;
    return FND_OK;
}

int32_t cpt_sgc_rect(cpt_sgc *g, int32_t x, int32_t y, int32_t w, int32_t h, uint32_t color) {
    if (color >= CPT_SGC_COLORS) return FND_E_BAD_ARG;
    if (w <= 0 || h <= 0) return FND_OK;
    x = clamp16(x), y = clamp16(y);
    if (w > 65535) w = 65535;
    if (h > 65535) h = 65535;
    int32_t st = begin(g, color, 5);
    if (st < 0) return st;
    g->buf[g->len++] = FND_SGC_OP_RECT;
    g->buf[g->len++] = coord(x);
    g->buf[g->len++] = coord(y);
    g->buf[g->len++] = coord(x + w);
    g->buf[g->len++] = coord(y + h);
    return FND_OK;
}

int32_t cpt_sgc_line(cpt_sgc *g, int32_t x0, int32_t y0, int32_t x1, int32_t y1, uint32_t color) {
    int32_t st = begin(g, color, 5);
    if (st < 0) return st;
    g->buf[g->len++] = FND_SGC_OP_LINE;
    g->buf[g->len++] = coord(x0);
    g->buf[g->len++] = coord(y0);
    g->buf[g->len++] = coord(x1);
    g->buf[g->len++] = coord(y1);
    return FND_OK;
}

static int32_t ellipse(cpt_sgc *g, int32_t cx, int32_t cy, int32_t rx, int32_t ry,
                       uint32_t color, uint32_t opcode) {
    if (rx < 0 || rx > 32767 || ry < 0 || ry > 32767) return FND_E_BAD_ARG;
    int32_t st = begin(g, color, 5);
    if (st < 0) return st;
    g->buf[g->len++] = opcode;
    g->buf[g->len++] = coord(cx);
    g->buf[g->len++] = coord(cy);
    g->buf[g->len++] = (uint32_t)rx;
    g->buf[g->len++] = (uint32_t)ry;
    return FND_OK;
}

int32_t cpt_sgc_ellipse(cpt_sgc *g, int32_t cx, int32_t cy, int32_t rx, int32_t ry, uint32_t color) {
    return ellipse(g, cx, cy, rx, ry, color, FND_SGC_OP_ELLIPSE);
}

int32_t cpt_sgc_fill_ellipse(cpt_sgc *g, int32_t cx, int32_t cy, int32_t rx, int32_t ry, uint32_t color) {
    return ellipse(g, cx, cy, rx, ry, color, FND_SGC_OP_FILL_ELLIPSE);
}

int32_t cpt_sgc_triangle(cpt_sgc *g, int32_t x0, int32_t y0, int32_t x1, int32_t y1,
                         int32_t x2, int32_t y2, uint32_t color) {
    if (color >= CPT_SGC_COLORS) return FND_E_BAD_ARG;
    x0 = clamp16(x0), y0 = clamp16(y0), x1 = clamp16(x1), y1 = clamp16(y1);
    x2 = clamp16(x2), y2 = clamp16(y2);
    /* Collinear vertices draw nothing and would set FIFO_ERROR. Deltas up to
     * 15 bits keep the products inside 32 bits; larger triangles go to the device. */
    int32_t ax = x1 - x0, ay = y1 - y0, bx = x2 - x0, by = y2 - y0;
    if (ax >= -16383 && ax <= 16383 && ay >= -16383 && ay <= 16383 &&
        bx >= -16383 && bx <= 16383 && by >= -16383 && by <= 16383 && ax * by == bx * ay)
        return FND_OK;
    int32_t st = begin(g, color, 7);
    if (st < 0) return st;
    g->buf[g->len++] = FND_SGC_OP_TRIANGLE;
    g->buf[g->len++] = coord(x0);
    g->buf[g->len++] = coord(y0);
    g->buf[g->len++] = coord(x1);
    g->buf[g->len++] = coord(y1);
    g->buf[g->len++] = coord(x2);
    g->buf[g->len++] = coord(y2);
    return FND_OK;
}

int32_t cpt_sgc_glyph(cpt_sgc *g, uint32_t code, int32_t x, int32_t y, uint32_t color) {
    if (code > 0xFFFFu) return FND_E_BAD_ARG;
    int32_t st = begin(g, color, 4);
    if (st < 0) return st;
    g->buf[g->len++] = FND_SGC_OP_GLYPH;
    g->buf[g->len++] = code;
    g->buf[g->len++] = coord(x);
    g->buf[g->len++] = coord(y);
    return FND_OK;
}

int32_t cpt_sgc_raw(cpt_sgc *g, const uint32_t *words, uint32_t n) {
    if (n > g->cap) return FND_E_LIMIT;
    int32_t st = reserve(g, n);
    if (st < 0) return st;
    for (uint32_t i = 0; i < n; ++i) g->buf[g->len++] = words[i];
    g->color = UNKNOWN_COLOR; /* raw words may contain SET_COLOR */
    return FND_OK;
}
