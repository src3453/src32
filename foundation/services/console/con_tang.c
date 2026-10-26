/* Tang Console startup owns the small PCG fonts and first 64 CLUT colors.
 * No BIOS font loader exists on the UART boot path. Wide characters use the
 * existing '?' fallback; no access to the absent wide-font or input windows. */
#include <stdint.h>
#include "con_tang_font.h"

#ifndef MMIO8
#define MMIO8(address) (*(volatile uint8_t *)(uintptr_t)(address))
#endif

void con_tang_init(void) {
    MMIO8(0x8000FFFFu) = 1; /* register reset, preserves VRAM */
    MMIO8(0x8000F000u) = 1; /* disabled during font/palette setup */
    for (uint32_t i = 0; i < sizeof tang_fonts; ++i)
        MMIO8(0x10020000u + i) = tang_fonts[i];
    for (uint32_t i = 0; i < sizeof tang_palette; ++i)
        MMIO8(0x10012C00u + i) = tang_palette[i];
    MMIO8(0x8000F008u) = 0xFE; /* underline; two lines in 8x16 mode */
}
