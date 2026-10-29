#ifndef FOUNDATION_CON_PCG_H
#define FOUNDATION_CON_PCG_H
#include <stdint.h>

#include <foundation/fnd.h>
#include <foundation/keyboard.h>

/* The console service's screen and keyboard: the PCG and the PeC input
 * (doc/spec_Foundation_API.md §8). Output follows §8.1 and keys follow the
 * CON_READ_KEY codes of §8.3. The service needs MMIO grants for the ranges
 * below. Tang builds use UART input only; build.py grants the small font and
 * CLUT windows instead of wide-font/PeC windows for con_tang_init. */

#define CON_PCG_ROWS 30u

/* MMIO the console touches: the VDP window (display and PCG registers), the
 * PCG text/attribute planes in VRAM, the wide font area, PeC input and UART. The 1 byte
 * font VRAM (0x21000) is preserved by the normal backend; only the Tang startup
 * backend initializes it because the FPGA UART boot path has no BIOS font loader. */
#define CON_PCG_VDP_START 0x80000000u
#define CON_PCG_VDP_END 0x80010000u
#define CON_PCG_TEXT_START 0x10000000u
#define CON_PCG_TEXT_END 0x10003000u
/* The wide font installed for mode 3 (con_wide.h): VRAM 0x200000 and up. */
#define CON_PCG_FONT_START 0x10200000u
#define CON_PCG_FONT_END 0x10400000u
#define CON_PCG_INPUT_START 0x80040030u
#define CON_PCG_INPUT_END 0x80040180u
#define CON_PCG_UART_START 0x80040000u
#define CON_PCG_UART_END 0x80040010u

fnd_status con_pcg_init(void);
uint32_t con_pcg_columns(void);
uint32_t con_pcg_display(void);
fnd_status con_pcg_set_display(uint32_t mode);
void con_pcg_write(const uint8_t *text, uint32_t len);
void con_pcg_clear(void);
int con_pcg_poll_key(fnd_key_t *key);
void con_pcg_key_state(fnd_keyboard *state);
void con_pcg_get_cursor(uint32_t *x, uint32_t *y);
void con_pcg_set_cursor(uint32_t x, uint32_t y);
/* ANSI color indices 0..15, validated by the service. Clears bold/inverse. */
void con_pcg_set_color(uint32_t front, uint32_t back);

#endif
