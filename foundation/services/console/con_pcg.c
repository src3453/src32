/* Provisional console on the PCG (doc/spec_VDP.md) and the PeC keyboard
 * (doc/spec_PeC.md), see con_pcg.h. The default is SCREEN_MODE 1, 80x30
 * cells of 8x8; modes 0 and 3 provide 40 columns and 8x16 glyphs.
 * Scrolling moves SCROLL_Y by one cell row and clears only the
 * ring row that comes back at the bottom; the cursor keeps its screen row
 * apart from the physical ring row. */
#include "con_pcg.h"

#ifndef MMIO8
#define MMIO8(address) (*(volatile uint8_t *)(uintptr_t)(address))
#endif
#ifndef MMIO32
#define MMIO32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#endif

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

#define PEC_EVENT_DATA 0x80040030u
#define PEC_EVENT_STATUS 0x80040034u
#define PEC_EVENT_POP 0x80040038u

#define SUBSTITUTE '?'

static uint32_t col, row, top; /* cursor cell; ring row shown at screen row 0 */
static uint32_t columns = 80, cell_height = 8, display = FND_CON_DISPLAY_80;

uint32_t con_pcg_columns(void) { return columns; }
uint32_t con_pcg_display(void) { return display; }
/* ANSI RGB order mapped onto the existing RGB222 CLUT; never rewrite it. */
static const uint8_t colors[16] = {0, 32, 8, 40, 2, 34, 10, 42,
                                 21, 53, 29, 61, 23, 55, 31, 63};
static uint32_t fg = 15, bg, bold, inverse;
static uint32_t saved_col, saved_row;
enum { GROUND, ESCAPE, ESC_INTERMEDIATE, CSI, STRING, STRING_ESC };
static uint32_t parser, string_osc;
#define ANSI_PARAMS 16u
#define ANSI_MAX 9999u
static uint32_t params[ANSI_PARAMS], param_count, private_mode, invalid, param_started;

static uint32_t ring_row(uint32_t screen_row) { return (top + screen_row) % CON_PCG_ROWS; }

static void put_cell(uint32_t x, uint32_t y, uint32_t ch) {
    uint32_t cell = CON_PCG_TEXT_START + ring_row(y) * columns + x;
    MMIO8(cell) = (uint8_t)ch;
    uint8_t front = colors[fg < 8u && bold ? fg + 8u : fg], back = colors[bg];
    MMIO8(cell + PCG_FG) = inverse ? back : front;
    MMIO8(cell + PCG_BG) = inverse ? front : back;
}

static void clear_row(uint32_t y) {
    for (uint32_t x = 0; x < columns; ++x)
        put_cell(x, y, ' ');
}

/* SCROLL_Y is a 16-bit pixel offset written low byte first. */
static void set_scroll(void) {
    uint32_t y = top * cell_height;
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
    if (++col == columns)
        newline();
}

static uint32_t clamp(uint32_t value, uint32_t size) {
    return value < size ? value : size - 1u;
}

static void erase_cells(uint32_t first, uint32_t end) {
    for (; first < end; ++first)
        put_cell(first % columns, first / columns, ' ');
}

void con_pcg_set_color(uint32_t front, uint32_t back) {
    fg = front;
    bg = back;
    bold = inverse = 0;
}

static void sgr(void) {
    for (uint32_t i = 0; i < param_count; ++i) {
        uint32_t p = params[i];
        if (p == 0) con_pcg_set_color(15, 0);
        else if (p == 1) bold = 1;
        else if (p == 22) bold = 0;
        else if (p == 7) inverse = 1;
        else if (p == 27) inverse = 0;
        else if (p >= 30 && p <= 37) fg = p - 30;
        else if (p >= 40 && p <= 47) bg = p - 40;
        else if (p >= 90 && p <= 97) fg = p - 90 + 8;
        else if (p >= 100 && p <= 107) bg = p - 100 + 8;
        else if (p == 39) fg = 15;
        else if (p == 49) bg = 0;
        /* Consume the operands of unsupported extended colors as a unit.
         * In particular 38;2;... must not turn RGB values into SGR codes. */
        else if (p == 38 || p == 48) {
            if (i + 1u >= param_count) return;
            uint32_t count = params[++i] == 2 ? 3u : params[i] == 5 ? 1u : ANSI_PARAMS;
            if (count >= param_count - i) return;
            i += count;
        }
    }
}

static void csi_final(uint32_t ch) {
    if (invalid) return;
    uint32_t p = params[0], n = p ? p : 1u;
    if (private_mode) {
        if (param_count == 1 && p == 25 && (ch == 'h' || ch == 'l'))
            MMIO8(PCG_CURSOR_ENABLE) = ch == 'h';
        return;
    }
    if (ch == 'm') { sgr(); return; }
    if (ch == 'H' || ch == 'f') {
        if (param_count > 2) return;
        row = clamp(n - 1u, CON_PCG_ROWS);
        col = clamp(params[1] ? params[1] - 1u : 0, columns);
        return;
    }
    if (param_count != 1) return;
    switch (ch) {
    case 'A': row = n > row ? 0 : row - n; break;
    case 'B': row = clamp(row + n, CON_PCG_ROWS); break;
    case 'C': col = clamp(col + n, columns); break;
    case 'D': col = n > col ? 0 : col - n; break;
    case 'E': row = clamp(row + n, CON_PCG_ROWS); col = 0; break;
    case 'F': row = n > row ? 0 : row - n; col = 0; break;
    case 'G': col = clamp(n - 1u, columns); break;
    case 'd': row = clamp(n - 1u, CON_PCG_ROWS); break;
    case 'J': {
        uint32_t pos = row * columns + col, end = CON_PCG_ROWS * columns;
        if (p == 0) erase_cells(pos, end);
        else if (p == 1) erase_cells(0, pos + 1u);
        else if (p == 2) erase_cells(0, end);
        break;
    }
    case 'K': {
        uint32_t start = row * columns, pos = start + col;
        if (p == 0) erase_cells(pos, start + columns);
        else if (p == 1) erase_cells(start, pos + 1u);
        else if (p == 2) erase_cells(start, start + columns);
        break;
    }
    case 's': if (!p) { saved_col = col; saved_row = row; } break;
    case 'u': if (!p) { col = saved_col; row = saved_row; } break;
    default: break;
    }
}

/* Parse bytes before UTF-8 decoding. State belongs to the shared screen and
 * survives IPC chunks. Unknown sequences are consumed without buffering. */
static int ansi_byte(uint32_t b) {
    if (b == 0x18 || b == 0x1A) { parser = GROUND; return 1; }
    if (parser == STRING || parser == STRING_ESC) {
        if ((string_osc && b == 7) || (parser == STRING_ESC && b == '\\'))
            parser = GROUND;
        else parser = b == 0x1B ? STRING_ESC : STRING;
        return 1;
    }
    if (b == 0x1B) { parser = ESCAPE; return 1; }
    if (parser == GROUND) return 0;
    if (b < 0x20 || b == 0x7F) return 1;
    if (parser == ESCAPE) {
        parser = GROUND;
        if (b == '[') {
            parser = CSI;
            param_count = 1;
            private_mode = invalid = param_started = 0;
            for (uint32_t i = 0; i < ANSI_PARAMS; ++i) params[i] = 0;
        } else if (b == ']' || b == 'P' || b == 'X' || b == '^' || b == '_') {
            parser = STRING;
            string_osc = b == ']';
        } else if (b == '7') { saved_col = col; saved_row = row; }
        else if (b == '8') { col = saved_col; row = saved_row; }
        else if (b >= 0x20 && b <= 0x2F) parser = ESC_INTERMEDIATE;
    } else if (parser == ESC_INTERMEDIATE) {
        if (b >= 0x30) parser = GROUND;
    } else if (b >= 0x40 && b <= 0x7E) {
        csi_final(b);
        parser = GROUND;
    } else if (b == '?' && !param_started && !private_mode && !invalid) {
        private_mode = 1;
    } else if (b >= '0' && b <= '9') {
        param_started = 1;
        uint32_t value = params[param_count - 1u] * 10u + b - '0';
        if (value > ANSI_MAX) invalid = 1;
        else params[param_count - 1u] = value;
    } else if (b == ';') {
        param_started = 1;
        if (param_count == ANSI_PARAMS) invalid = 1;
        else ++param_count;
    } else {
        invalid = 1;
        if (b > 0x7F) parser = GROUND;
    }
    return 1;
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
            col = columns - 1u;
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
    /* Private-use scalars address configured PCG glyphs without interpreting
     * glyph 00/09/0A/1B as console controls. No font or CLUT mutation. */
    put_cell(col, row, cp >= 0xE000u && cp <= 0xE0FFu ? cp - 0xE000u :
                       cp < 0x7Fu ? cp : SUBSTITUTE);
    advance();
}

void con_pcg_write(const uint8_t *text, uint32_t len) {
    uint32_t i = 0;
    while (i < len) {
        if (ansi_byte(text[i])) { ++i; continue; }
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
    parser = GROUND;
    top = 0;
    col = row = 0;
    set_scroll();
    for (uint32_t y = 0; y < CON_PCG_ROWS; ++y)
        clear_row(y);
    show_cursor();
}

fnd_status con_pcg_init(void) {
    con_pcg_set_color(15, 0);
    MMIO8(VDP_DISPLAY_MODE) = 1;
    MMIO8(PCG_DISABLE) = 0;
    MMIO8(PCG_SCROLL_X_LO) = 0;
    MMIO8(PCG_SCROLL_X_HI) = 0;
    MMIO8(PCG_CURSOR_ENABLE) = 1;
    MMIO8(PCG_CURSOR_BLINK) = 10;
    return con_pcg_set_display(FND_CON_DISPLAY_80);
}

fnd_status con_pcg_set_display(uint32_t mode) {
    if (mode != FND_CON_DISPLAY_40 && mode != FND_CON_DISPLAY_80 && mode != FND_CON_DISPLAY_8X16)
        return FND_E_BAD_ARG;
    /* Font VRAM belongs to the startup configuration; preserve it in every mode. */
    display = mode;
    columns = mode == FND_CON_DISPLAY_40 ? 40u : 80u;
    cell_height = mode == FND_CON_DISPLAY_8X16 ? 16u : 8u;
    saved_col = saved_row = 0;
    MMIO8(PCG_SCREEN_MODE) = (uint8_t)mode;
    MMIO8(PCG_SCROLL_X_LO) = 0;
    MMIO8(PCG_SCROLL_X_HI) = 0;
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
static fnd_keyboard keyboard;

void con_pcg_key_state(fnd_keyboard *state) {
    *state = keyboard;
    for (uint32_t i = 0; i < 8; ++i)
        keyboard.pressed[i] = keyboard.released[i] = 0;
}

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
    uint32_t word = usage / 32u, bit = 1u << (usage % 32u);
    if (down) {
        if (!(keyboard.down[word] & bit)) keyboard.pressed[word] |= bit;
        keyboard.down[word] |= bit;
    } else {
        if (keyboard.down[word] & bit) keyboard.released[word] |= bit;
        keyboard.down[word] &= ~bit;
    }
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

/* Takes one key press from the PeC event FIFO; 0 when it is empty. No
 * irq_bind yet, so the service polls this between messages. */
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

void con_pcg_get_cursor(uint32_t *x, uint32_t *y) {
    *x = col;
    *y = row;
}

void con_pcg_set_cursor(uint32_t x, uint32_t y) {
    col = x;
    row = y;
    show_cursor();
}
