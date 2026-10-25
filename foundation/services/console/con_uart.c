/* Serial terminal clone. No UART configuration changes or IRQ ownership.
 * DATA accepts queued writes, like kernel/debug_write; do not spin on TX_READY. */
#include "con_uart.h"
#ifndef CON_UART_READ8
#define CON_UART_READ8(a) (*(volatile uint8_t *)(uintptr_t)(a))
#define CON_UART_WRITE8(a, v) (*(volatile uint8_t *)(uintptr_t)(a) = (v))
#endif

static uint32_t utf, minimum, left, escape, parameter, deadline;
static int after_cr, output_cr;

void con_uart_write(const uint8_t *text, uint32_t len) {
    for (uint32_t i = 0; i < len; ++i) {
        uint8_t b = text[i];
        if (b == '\n' && !output_cr) CON_UART_WRITE8(0x80040000u, '\r');
        CON_UART_WRITE8(0x80040000u, b);
        output_cr = b == '\r';
    }
}

static void number(uint32_t n) {
    uint8_t digits[10];
    uint32_t len = 0;
    do { digits[len++] = (uint8_t)('0' + n % 10); n /= 10; } while (n);
    while (len) con_uart_write(&digits[--len], 1);
}
void con_uart_clear(void) { con_uart_write((const uint8_t *)"\033[2J\033[H", 7); }
void con_uart_cursor(uint32_t x, uint32_t y) {
    con_uart_write((const uint8_t *)"\033[", 2); number(y + 1);
    con_uart_write((const uint8_t *)";", 1); number(x + 1);
    con_uart_write((const uint8_t *)"H", 1);
}
void con_uart_color(uint32_t front, uint32_t back) {
    con_uart_write((const uint8_t *)"\033[0;", 4);
    number(front < 8 ? 30 + front : 90 + front - 8);
    con_uart_write((const uint8_t *)";", 1);
    number(back < 8 ? 40 + back : 100 + back - 8);
    con_uart_write((const uint8_t *)"m", 1);
}

int con_uart_poll_key(fnd_key_t *key, uint32_t now) {
    *key = (fnd_key_t){0};
    /* Bound work per call, including malformed/unknown input. */
    for (uint32_t count = 0; count < 256; ++count) {
        if (!(CON_UART_READ8(0x80040001u) & 2u)) {
            if (escape && (int32_t)(now - deadline) >= 0) {
                uint32_t old = escape;
                escape = 0;
                if (old == 1) { key->code = FND_KEY_ESC; return 1; }
            }
            return 0;
        }
        uint8_t b = CON_UART_READ8(0x80040000u);
        if (escape) {
            uint32_t old = escape;
            escape = 0;
            if (old == 1 && (b == '[' || b == 'O')) {
                escape = 2; parameter = 0; continue;
            }
            if (old == 2) {
                if (b >= '0' && b <= '9') {
                    parameter = parameter < 100 ? parameter * 10 + b - '0' : 100;
                    escape = 2; continue;
                }
                switch (b) {
                case 'A': key->code = FND_KEY_UP; break;
                case 'B': key->code = FND_KEY_DOWN; break;
                case 'C': key->code = FND_KEY_RIGHT; break;
                case 'D': key->code = FND_KEY_LEFT; break;
                case 'H': key->code = FND_KEY_HOME; break;
                case 'F': key->code = FND_KEY_END; break;
                case '~':
                    if (parameter == 1 || parameter == 7) key->code = FND_KEY_HOME;
                    if (parameter == 4 || parameter == 8) key->code = FND_KEY_END;
                    if (parameter == 3) key->code = FND_KEY_DELETE;
                    break;
                default:
                    /* Discard unsupported CSI through its final byte. */
                    if (b < 0x40 || b > 0x7e) escape = 3;
                    break;
                }
                if (key->code) return 1;
                continue;
            }
            if (old == 3) {
                if (b < 0x40 || b > 0x7e) escape = 3;
                continue;
            }
            /* ESC plus an ordinary character: retain the character as Alt. */
            key->mods = FND_MOD_ALT;
        }
        if (left) {
            if ((b & 0xc0) == 0x80) {
                utf = (utf << 6) | (b & 63);
                if (--left) continue;
                key->code = FND_KEY_CHAR;
                key->ch = utf < minimum || utf > 0x10ffff || (utf >= 0xd800 && utf <= 0xdfff)
                              ? 0xfffd : utf;
                return 1;
            }
            left = 0; /* resynchronize on the new byte */
        }
        if (b == '\n' && after_cr) { after_cr = 0; continue; }
        after_cr = b == '\r';
        if (b == '\r' || b == '\n') key->code = FND_KEY_ENTER;
        else if (b == 8 || b == 127) key->code = FND_KEY_BACKSPACE;
        else if (b == 9) key->code = FND_KEY_TAB;
        else if (b == 27) { escape = 1; deadline = now + 5; continue; }
        else if (b < 32) {
            key->code = FND_KEY_CHAR;
            key->ch = !b ? ' ' : b <= 26 ? 'a' + b - 1 : '@' + b;
            key->mods |= FND_MOD_CTRL;
        } else if (b < 128) { key->code = FND_KEY_CHAR; key->ch = b; }
        else if (b >= 0xc2 && b <= 0xf4) {
            left = b < 0xe0 ? 1 : b < 0xf0 ? 2 : 3;
            minimum = left == 1 ? 0x80 : left == 2 ? 0x800 : 0x10000;
            utf = b & (left == 1 ? 31 : left == 2 ? 15 : 7);
            continue;
        } else { key->code = FND_KEY_CHAR; key->ch = 0xfffd; }
        return 1;
    }
    return 0;
}
