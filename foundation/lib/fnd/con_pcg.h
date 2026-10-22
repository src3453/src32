#ifndef FOUNDATION_CON_PCG_H
#define FOUNDATION_CON_PCG_H
#include <stdint.h>

#include <foundation/fnd.h>

/* Provisional console on the PCG and the PeC keyboard, linked into the
 * program until the console service exists (doc/spec_Foundation_API.md §8).
 * Output follows §8.1 and keys follow the CON_READ_KEY codes of §8.3. The
 * process needs MMIO grants for CON_PCG_GRANTS. */

#define CON_PCG_COLS 80u
#define CON_PCG_ROWS 30u

/* MMIO the console touches: the VDP window (display and PCG registers), the
 * PCG text and attribute planes in VRAM, and the PeC input window. */
#define CON_PCG_VDP_START 0x80000000u
#define CON_PCG_VDP_END 0x80010000u
#define CON_PCG_TEXT_START 0x10000000u
#define CON_PCG_TEXT_END 0x10003000u
#define CON_PCG_INPUT_START 0x80040030u
#define CON_PCG_INPUT_END 0x80040180u

fnd_status con_pcg_init(void);
void con_pcg_write(const uint8_t *text, uint32_t len);
void con_pcg_clear(void);
/* Waits for the next key press. */
void con_pcg_read_key(fnd_key_t *key);
int con_pcg_poll_key(fnd_key_t *key);

#endif
