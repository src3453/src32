#ifndef FOUNDATION_LIBSGC_H
#define FOUNDATION_LIBSGC_H
#include <foundation/fnd.h>
#include <foundation/sgc.h>

/* libsgc: the WAL "sgc" 1.0 (/lib/sgc.1.wal), a command builder over /dev/sgc.
 *
 * Drawing calls append SGC commands to a caller-owned word buffer. Nothing
 * reaches the device until cpt_sgc_flush, which queues whole words into the
 * 256-word FIFO and never blocks: the FIFO drains once per VDP frame, so a
 * flush that finds it full simply leaves the rest queued for a later call.
 * A drawing call that does not fit in the buffer (after one flush attempt)
 * queues nothing and returns FND_E_WOULD_BLOCK. Commands are never split.
 *
 * Coordinates are in the SGC plane (320x240), signed, clipped by the hardware;
 * values outside the signed 16 bit range are clamped. A RECT with no area and
 * a degenerate TRIANGLE draw nothing and are not queued (the hardware would
 * report FIFO_ERROR for them). Colors are CLUT indices 0..63.
 *
 * The context holds no hidden state: a program may keep several. WAL calls
 * carry only 32 bit scalars and pointers, so signed values are int32_t. */
typedef struct cpt_sgc {
    fnd_handle dev; /* /dev/sgc, 0 when closed */
    fnd_handle fb;  /* /dev/fb0 kept open by cpt_sgc_display_init, else 0 */
    uint32_t *buf;
    uint32_t cap;   /* words */
    uint32_t head;  /* first word not yet in the FIFO */
    uint32_t len;   /* words queued in buf, head <= len <= cap */
    uint32_t color; /* color after the last queued SET_COLOR, ~0u if unknown */
} cpt_sgc;

#define CPT_SGC_WIDTH 320
#define CPT_SGC_HEIGHT 240
#define CPT_SGC_COLORS 64u
/* Largest single command (TRIANGLE); a buffer needs at least this many words. */
#define CPT_SGC_MAX_COMMAND 8u
#define CPT_SGC_WAIT_TICKS 600u

/* Opens /dev/sgc for writing (exclusive; the driver resets the SGC) and makes
 * buf[cap] the command buffer. FND_E_BAD_ARG if cap < CPT_SGC_MAX_COMMAND. */
int32_t cpt_sgc_open(cpt_sgc *g, uint32_t *buf, uint32_t cap);
/* Drops anything unsent, closes the streams and restores the SGC (driver close). */
int32_t cpt_sgc_close(cpt_sgc *g);
/* Opens /dev/fb0, requires 320x240 INDEX8, writes count RGB triples (3 bytes
 * each) to the CLUT from entry first, and keeps fb0 open until cpt_sgc_close.
 * The SGC draws into its own plane; fb0 only sets up the display and CLUT.
 * rgb may be 0 with count 0 to leave the CLUT unchanged. */
int32_t cpt_sgc_display_init(cpt_sgc *g, uint32_t first, uint32_t count, const uint8_t *rgb);

/* Queues as much as the FIFO accepts now. FND_OK even when the FIFO is full
 * (see cpt_sgc_pending). Errors are device errors. */
int32_t cpt_sgc_flush(cpt_sgc *g);
/* Words still waiting in the buffer. */
uint32_t cpt_sgc_pending(const cpt_sgc *g);
/* Flushes, sleeping one tick between attempts, until the buffer is empty.
 * FND_E_TIMEOUT if the FIFO takes nothing for CPT_SGC_WAIT_TICKS attempts. */
int32_t cpt_sgc_flush_wait(cpt_sgc *g);
/* Discards unsent words and clears the device FIFO (not pixels already drawn). */
int32_t cpt_sgc_cancel(cpt_sgc *g);
/* Soft reset: clears the FIFO and every drawn pixel, as at open. */
int32_t cpt_sgc_reset(cpt_sgc *g);
/* Selects the GP (0..7) the SGC draws to. */
int32_t cpt_sgc_set_output(cpt_sgc *g, uint32_t gp);
/* Reads the status; clear is a mask of sticky bits (FND_SGC_STATUS_*) to clear. */
int32_t cpt_sgc_status(cpt_sgc *g, uint32_t clear, fnd_sgc_status *out);

int32_t cpt_sgc_set_color(cpt_sgc *g, uint32_t color);
int32_t cpt_sgc_clear(cpt_sgc *g, uint32_t color);
int32_t cpt_sgc_rect(cpt_sgc *g, int32_t x, int32_t y, int32_t w, int32_t h, uint32_t color);
int32_t cpt_sgc_line(cpt_sgc *g, int32_t x0, int32_t y0, int32_t x1, int32_t y1, uint32_t color);
/* Center and radii 0..32767; equal radii draw a circle. Negative/too-large
 * radii return BAD_ARG. A zero radius degenerates to an inclusive line/point. */
int32_t cpt_sgc_ellipse(cpt_sgc *g, int32_t cx, int32_t cy, int32_t rx, int32_t ry, uint32_t color);
int32_t cpt_sgc_fill_ellipse(cpt_sgc *g, int32_t cx, int32_t cy, int32_t rx, int32_t ry, uint32_t color);
int32_t cpt_sgc_triangle(cpt_sgc *g, int32_t x0, int32_t y0, int32_t x1, int32_t y1,
                         int32_t x2, int32_t y2, uint32_t color);
/* One Unifont BMP code unit with its top left at (x, y); 0 bits stay transparent. */
int32_t cpt_sgc_glyph(cpt_sgc *g, uint32_t code, int32_t x, int32_t y, uint32_t color);
/* Appends n raw SGC words as one unit (all or nothing). */
int32_t cpt_sgc_raw(cpt_sgc *g, const uint32_t *words, uint32_t n);
#endif
