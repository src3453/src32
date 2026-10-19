#include <cpt32/mmio.h>
#include <cpt32/pcg.h>

uint32_t pcg_cur_x = 0, pcg_cur_y = 0, pcg_width = 40, pcg_height = 30;
uint32_t pcg_cell_height = 8;
uint32_t pcg_fg_color = 0x3f, pcg_bg_color = 0, pcg_mode = 0, pcg_cursor = 0;
uint32_t pcg_vram_base = 0x10000000u, pcg_scroll_y = 0, pcg_vram_row = 0;

static void pcg_fill(uint32_t address, uint32_t size, uint8_t value) {
  uint32_t i;
  for (i = 0; i < size; ++i)
    CPT32_MMIO8(address + i) = value;
}
void pcg_col40(void) {
  pcg_width = 40;
  pcg_cell_height = 8;
  CPT32_MMIO8(0x8000f003u) = 0;
}
void pcg_col80(void) {
  pcg_width = 80;
  pcg_cell_height = 8;
  CPT32_MMIO8(0x8000f003u) = 1;
}
void pcg_mode80x16(void) {
  pcg_width = 80;
  pcg_height = 30;
  pcg_cell_height = 16;
  CPT32_MMIO8(0x8000f003u) = 3;
}
static void pcg_setcur(uint32_t x, uint32_t y) {
  CPT32_MMIO8(0x8000f005u) = (uint8_t)x;
  CPT32_MMIO8(0x8000f006u) = (uint8_t)y;
}
static void pcg_update_row(void) {
  pcg_vram_row = ((pcg_scroll_y / pcg_cell_height) + pcg_cur_y) % pcg_height;
}
static void pcg_update_cur(void) { pcg_setcur(pcg_cur_x, pcg_vram_row); }
void pcg_locate(uint32_t x, uint32_t y) {
  pcg_cur_x = x;
  pcg_cur_y = y;
  pcg_update_row();
  pcg_update_cur();
}
void pcg_cls(uint32_t color) { pcg_fill(0x10002000u, 0x1000u, (uint8_t)color); }
void pcg_initPCG(void) {
  CPT32_MMIO8(0x80000001u) = 1;
  CPT32_MMIO8(0x8000f003u) = 0;
  pcg_mode = 0;
  pcg_width = 40;
  pcg_height = 30;
  pcg_cell_height = 8;
  CPT32_MMIO8(0x8000f000u) = 0;
  pcg_scroll_y = 0;
  CPT32_MMIO8(0x8000f00cu) = 0;
  CPT32_MMIO8(0x8000f00du) = 0;
  CPT32_MMIO8(0x8000f00eu) = 0;
  CPT32_MMIO8(0x8000f00fu) = 0;
  pcg_cur_x = pcg_cur_y = 0;
  pcg_update_row();
  CPT32_MMIO8(0x8000f007u) = 1;
  CPT32_MMIO8(0x8000f009u) = 10;
  pcg_update_cur();
}
static void pcg_ln(void) {
  pcg_cur_x = 0;
  pcg_cur_y++;
  pcg_vram_row = (pcg_vram_row + 1u) % pcg_height;
  if (pcg_cur_y >= pcg_height) {
    pcg_scroll_y = (pcg_scroll_y + pcg_cell_height) % (pcg_height * pcg_cell_height);
    CPT32_MMIO8(0x8000f00eu) = (uint8_t)pcg_scroll_y;
    pcg_cur_y = pcg_height - 1u;
    {
      uint32_t p = pcg_vram_base + pcg_vram_row * pcg_width;
      pcg_fill(p, pcg_width, 0);
      pcg_fill(p + 0x1000u, pcg_width, 0x3f);
      pcg_fill(p + 0x2000u, pcg_width, 0);
    }
  }
  pcg_update_cur();
}
void pcg_putcscr(uint32_t ch) {
  uint32_t p = pcg_vram_base + pcg_vram_row * pcg_width + pcg_cur_x;
  CPT32_MMIO8(p) = (uint8_t)ch;
  CPT32_MMIO8(p + 0x1000u) = (uint8_t)pcg_fg_color;
  CPT32_MMIO8(p + 0x2000u) = (uint8_t)pcg_bg_color;
  pcg_cur_x++;
  if (pcg_cur_x >= pcg_width)
    pcg_ln();
  pcg_update_cur();
}
void pcg_prnscr(const char *s) {
  while (*s)
    pcg_putcscr((uint8_t)*s++);
}
void pcg_prnlnscr(const char *s) {
  pcg_prnscr(s);
  pcg_ln();
}
void pcg_colorfg(uint32_t c) { pcg_fg_color = c; }
void pcg_colorbg(uint32_t c) { pcg_bg_color = c; }
