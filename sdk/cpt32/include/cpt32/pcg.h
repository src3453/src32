#ifndef CPT32_PCG_H
#define CPT32_PCG_H
#include <stdint.h>
/* sol pcg namespace state is public and intentionally mutable by callers. */
extern uint32_t pcg_cur_x, pcg_cur_y, pcg_width, pcg_height;
extern uint32_t pcg_fg_color, pcg_bg_color, pcg_mode, pcg_cursor;
extern uint32_t pcg_vram_base, pcg_scroll_y, pcg_vram_row;
void pcg_col40(void);
void pcg_col80(void);
void pcg_locate(uint32_t x, uint32_t y);
void pcg_cls(uint32_t color);
void pcg_initPCG(void);
void pcg_putcscr(uint32_t character);
void pcg_prnscr(const char *string);
void pcg_prnlnscr(const char *string);
void pcg_colorfg(uint32_t color);
void pcg_colorbg(uint32_t color);
#endif
