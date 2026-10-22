/* Provisional console on the PCG (doc/spec_VDP.md) and the PeC keyboard
 * (doc/spec_PeC.md), see con_pcg.h. The screen is SCREEN_MODE 1, 80x30
 * cells of 8x8. Scrolling moves SCROLL_Y by one cell row and clears only the
 * ring row that comes back at the bottom; the cursor keeps its screen row
 * apart from the physical ring row. */
#include <foundation/sys.h>

#include "con_pcg.h"

#define MMIO8(address) (*(volatile uint8_t *)(uintptr_t)(address))
#define MMIO32(address) (*(volatile uint32_t *)(uintptr_t)(address))

#define VDP_DISPLAY_MODE 0x80000001u
#define PCG_DISABLE 0x8000F000u
#define PCG_SCREEN_MODE 0x8000F003u
#define PCG_CURSOR_X 0x8000F005u
#define PCG_CURSOR_Y 0x8000F006u
#define PCG_CURSOR_ENABLE 0x8000F007u
#define PCG_CURSOR_BLINK 0x8000F009u
#define PCG_SCROLL_X_LO 0x8000F00Cu
#define PCG_SCROLL_X_HI 0x8000F00Du
#define PCG_SCROLL_Y_LO 0x8000F00Eu
#define PCG_SCROLL_Y_HI 0x8000F00Fu
#define PCG_FG 0x1000u
#define PCG_BG 0x2000u
#define CELL_HEIGHT 8u

#define PEC_EVENT_DATA 0x80040030u
#define PEC_EVENT_STATUS 0x80040034u
#define PEC_EVENT_POP 0x80040038u
#define POLL_TICKS 4u

#define COLOR_FG 0x3Fu
#define COLOR_BG 0x00u
#define SUBSTITUTE '?'

static uint32_t col, row, top; /* cursor cell; ring row shown at screen row 0 */

static uint32_t ring_row(uint32_t screen_row) { return (top + screen_row) % CON_PCG_ROWS; }

static void put_cell(uint32_t x, uint32_t y, uint32_t ch) {
    uint32_t cell = CON_PCG_TEXT_START + ring_row(y) * CON_PCG_COLS + x;
    MMIO8(cell) = (uint8_t)ch;
    MMIO8(cell + PCG_FG) = COLOR_FG;
    MMIO8(cell + PCG_BG) = COLOR_BG;
}

static void clear_row(uint32_t y) {
    for (uint32_t x = 0; x < CON_PCG_COLS; ++x)
        put_cell(x, y, ' ');
}

/* SCROLL_Y is a 16-bit pixel offset written low byte first. */
static void set_scroll(void) {
    uint32_t y = top * CELL_HEIGHT;
    MMIO8(PCG_SCROLL_Y_LO) = (uint8_t)y;
    MMIO8(PCG_SCROLL_Y_HI) = (uint8_t)(y >> 8);
}

static void show_cursor(void) {
    MMIO8(PCG_CURSOR_X) = (uint8_t)col;
    MMIO8(PCG_CURSOR_Y) = (uint8_t)ring_row(row);
}

static void newline(void) {
    col = 0;
    if (row + 1u < CON_PCG_ROWS) {
        ++row;
        return;
    }
    top = (top + 1u) % CON_PCG_ROWS;
    set_scroll();
    clear_row(CON_PCG_ROWS - 1u);
}

static void advance(void) {
    if (++col == CON_PCG_COLS)
        newline();
}

/* §8.1: LF, CR, BS and TAB are controls; other controls are not shown. The
 * PCG font has one byte per cell, so scalars above ASCII show SUBSTITUTE. */
static void put_scalar(uint32_t cp) {
    switch (cp) {
    case '\n':
        newline();
        return;
    case '\r':
        col = 0;
        return;
    case '\b':
        if (col) {
            --col;
        } else if (row) {
            --row;
            col = CON_PCG_COLS - 1u;
        }
        return;
    case '\t':
        do {
            put_cell(col, row, ' ');
            advance();
        } while (col % 8u);
        return;
    default:
        break;
    }
    if (cp < 0x20u || cp == 0x7Fu || (cp >= 0x80u && cp < 0xA0u))
        return;
    put_cell(col, row, cp < 0x7Fu ? cp : SUBSTITUTE);
    advance();
}

void con_pcg_write(const uint8_t *text, uint32_t len) {
    uint32_t i = 0;
    while (i < len) {
        uint32_t b = text[i], cp = SUBSTITUTE, n = 1, need = 0;
        if (b < 0x80u) {
            cp = b;
        } else if ((b & 0xE0u) == 0xC0u) {
            need = 1, cp = b & 0x1Fu;
        } else if ((b & 0xF0u) == 0xE0u) {
            need = 2, cp = b & 0x0Fu;
        } else if ((b & 0xF8u) == 0xF0u) {
            need = 3, cp = b & 0x07u;
        }
        for (uint32_t k = 0; k < need; ++k) {
            if (i + n >= len || (text[i + n] & 0xC0u) != 0x80u) {
                cp = SUBSTITUTE; /* malformed: one substitute, resync */
                break;
            }
            cp = cp << 6 | (text[i + n++] & 0x3Fu);
        }
        put_scalar(cp);
        i += n;
    }
    show_cursor();
}

void con_pcg_clear(void) {
    top = 0;
    col = row = 0;
    set_scroll();
    for (uint32_t y = 0; y < CON_PCG_ROWS; ++y)
        clear_row(y);
    show_cursor();
}

fnd_status con_pcg_init(void) {
    MMIO8(VDP_DISPLAY_MODE) = 1;
    MMIO8(PCG_DISABLE) = 0;
    MMIO8(PCG_SCREEN_MODE) = 1;
    MMIO8(PCG_SCROLL_X_LO) = 0;
    MMIO8(PCG_SCROLL_X_HI) = 0;
    MMIO8(PCG_CURSOR_ENABLE) = 1;
    MMIO8(PCG_CURSOR_BLINK) = 10;
    con_pcg_clear();
    return FND_OK;
}

/* Keyboard state from the HID usages (page 0x07) of PeC KEY events. */
#define HID_ENTER 40u
#define HID_ESC 41u
#define HID_BACKSPACE 42u
#define HID_TAB 43u
#define HID_CAPS_LOCK 57u
#define HID_HOME 74u
#define HID_DELETE 76u
#define HID_END 77u
#define HID_RIGHT 79u
#define HID_LEFT 80u
#define HID_DOWN 81u
#define HID_UP 82u
#define HID_KP_ENTER 88u
#define HID_LEFT_CTRL 224u

static uint32_t held;      /* bit n: HID_LEFT_CTRL + n is down */
static uint32_t caps_lock;

static uint32_t mods(void) {
    uint32_t m = 0;
    if (held & 0x22u)
        m |= FND_MOD_SHIFT;
    if (held & 0x11u)
        m |= FND_MOD_CTRL;
    if (held & 0x44u)
        m |= FND_MOD_ALT;
    return m;
}

/* US layout: usages 30..56 without and with Shift (0 = no character). */
static const char plain[] = "1234567890\0\0\0\0 -=[]\\\0;'`,./";
static const char shifted[] = "!@#$%^&*()\0\0\0\0 _+{}|\0:\"~<>?";

static uint32_t key_code(uint32_t usage) {
    switch (usage) {
    case HID_ENTER:
    case HID_KP_ENTER:
        return FND_KEY_ENTER;
    case HID_BACKSPACE:
        return FND_KEY_BACKSPACE;
    case HID_DELETE:
        return FND_KEY_DELETE;
    case HID_LEFT:
        return FND_KEY_LEFT;
    case HID_RIGHT:
        return FND_KEY_RIGHT;
    case HID_UP:
        return FND_KEY_UP;
    case HID_DOWN:
        return FND_KEY_DOWN;
    case HID_HOME:
        return FND_KEY_HOME;
    case HID_END:
        return FND_KEY_END;
    case HID_TAB:
        return FND_KEY_TAB;
    case HID_ESC:
        return FND_KEY_ESC;
    default:
        return 0;
    }
}

static uint32_t key_char(uint32_t usage, uint32_t m) {
    uint32_t shift = (m & FND_MOD_SHIFT) != 0;
    if (usage >= 4u && usage <= 29u) {
        uint32_t ch = 'a' + usage - 4u;
        /* §8.3: Ctrl with a letter reports the letter itself (Ctrl+C = 'c'). */
        return !(m & FND_MOD_CTRL) && (shift ^ caps_lock) ? ch - 32u : ch;
    }
    if (usage >= 30u && usage <= 56u)
        return (uint8_t)(shift ? shifted : plain)[usage - 30u];
    return 0;
}

/* Turns one PeC event into a key press; 0 if it is none. */
static int decode(uint32_t event, fnd_key_t *key) {
    if ((event >> 28) != 0 || ((event >> 24) & 0xFu) != 0)
        return 0; /* not a keyboard KEY event */
    uint32_t usage = (event >> 16) & 0xFFu, down = (event & 0xFFFFu) != 0;
    if (usage >= HID_LEFT_CTRL && usage < HID_LEFT_CTRL + 8u) {
        uint32_t bit = 1u << (usage - HID_LEFT_CTRL);
        held = down ? held | bit : held & ~bit;
        return 0;
    }
    if (!down)
        return 0;
    if (usage == HID_CAPS_LOCK) {
        caps_lock ^= 1u;
        return 0;
    }
    uint32_t m = mods(), code = key_code(usage), ch = 0;
    if (!code) {
        ch = key_char(usage, m);
        if (!ch)
            return 0;
        code = FND_KEY_CHAR;
    }
    key->code = code;
    key->ch = ch;
    key->mods = m;
    return 1;
}

/* No irq_bind yet, so the FIFO is polled, sleeping between empty polls. */
int con_pcg_poll_key(fnd_key_t *key) {
    for (;;) {
        uint32_t status = MMIO32(PEC_EVENT_STATUS);
        if (status & 2u)
            MMIO32(PEC_EVENT_STATUS) = 2u; /* overflow: W1C, keep reading */
        if (!(status & 1u))
            return 0;
        uint32_t event = MMIO32(PEC_EVENT_DATA);
        MMIO32(PEC_EVENT_POP) = 1u;
        if (decode(event, key))
            return 1;
    }
}

void con_pcg_read_key(fnd_key_t *key) {
    while (!con_pcg_poll_key(key))
        fnd_sys_sleep(POLL_TICKS);
}
