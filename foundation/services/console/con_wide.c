/* Wide font installer of the console, see con_wide.h. */
#include "con_wide.h"
#include "con_wide_font.h"

#ifndef CON_WIDE_STORE
#define CON_WIDE_STORE(address, value) (*(volatile uint32_t *)(uintptr_t)(address) = (value))
#endif
#ifndef CON_WIDE_REG
#define CON_WIDE_REG(address) (*(volatile uint8_t *)(uintptr_t)(address))
#endif

#define PCG_WIDE_FONT_BASE 0x8000F010u /* 32-bit, low byte first */

static int installed;

int con_wide_has(uint32_t unit) {
    uint32_t low = 0, high = CON_WIDE_RUNS;
    while (low < high) {
        uint32_t mid = (low + high) / 2u, run = con_wide_runs[mid];
        uint32_t start = run >> 16, end = start + (run & 0xFFFFu);
        if (unit < start) high = mid;
        else if (unit >= end) low = mid + 1u;
        else return 1;
    }
    return 0;
}

/* Words, not bytes: the generated code costs about 25 instructions per byte. */
void con_wide_install(void) {
    if (installed)
        return;
    uint32_t next = 0;
    for (uint32_t i = 0; i < CON_WIDE_RUNS; ++i) {
        uint32_t run = con_wide_runs[i], words = (run & 0xFFFFu) * 8u;
        uint32_t address = CON_WIDE_VRAM + CON_WIDE_FONT_BASE + (run >> 16) * 32u;
        for (uint32_t k = 0; k < words; ++k)
            CON_WIDE_STORE(address + k * 4u, con_wide_glyphs[next++]);
    }
    for (uint32_t b = 0; b < 4u; ++b)
        CON_WIDE_REG(PCG_WIDE_FONT_BASE + b) = (uint8_t)(CON_WIDE_FONT_BASE >> (8u * b));
    installed = 1;
}
