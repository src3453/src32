#ifndef FOUNDATION_CON_WIDE_H
#define FOUNDATION_CON_WIDE_H
#include <stdint.h>

/* The wide (2 cell) font of PCG mode 3 (doc/spec_VDP.md, WIDE_FONT_BASE). The
 * glyph of code unit u is at VRAM CON_WIDE_FONT_BASE + u * 32, the layout of the
 * Unifont ROM. Only the characters of JIS X 0208 and U+FFFD have glyphs; the
 * data is generated at build time (boot/wide_font.py). */
#define CON_WIDE_FONT_BASE 0x00200000u
#define CON_WIDE_VRAM 0x10000000u

/* 1 if the BMP code unit has a glyph in the wide font. */
int con_wide_has(uint32_t unit);
/* Writes the glyphs into VRAM and sets WIDE_FONT_BASE, once. */
void con_wide_install(void);

#endif
