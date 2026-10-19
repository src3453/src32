#include <cpt32/colors.h>
#include <cpt32/mmio.h>
#include <cpt32/pcg.h>
#include <cpt32/pec_rng.h>
#include <cpt32/vpu.h>

int main(void) {
  uint32_t rng;
  pcg_initPCG();
  pcg_colorfg(0x3fu);
  pcg_colorbg(0u);
  pcg_prnlnscr("PCG ring scroll smoke");
  /* Force more than one complete text screen of scrolling. */
  for (uint32_t i = 0; i < 35u; ++i)
    pcg_prnlnscr("scroll row");
  rng = pec_rng_randT();
  pcg_colorfg(colors_tocolor(rng & 7u, (rng >> 3) & 7u, (rng >> 6) & 3u));
  pcg_prnlnscr("RNG sampled");
  vpu_clear(1u, 0x204060ffu, 0x3f800000u);
  vpu_wait_idle();
  if ((CPT32_MMIO32(VPU_STATUS) & 0x38u) != 0)
    return 101;
  if (pcg_scroll_y != 64 || pcg_cur_y != 29 || pcg_vram_row != 7)
    return 102;
  if (CPT32_MMIO8(0x8000F006u) != 7)
    return 103;
  if (CPT32_MMIO8(0x10000000u + 6u * 40u) != 'R')
    return 104;
  if (CPT32_MMIO8(0x10000000u + 7u * 40u) != 0)
    return 105;
  if (colors_tocolor(7, 7, 3) != 143)
    return 106;
  /* CLS must erase all ring rows and attributes, including hidden cells when
   * switching column modes. Equal FG/BG also blanks a nonempty glyph zero. */
  for (uint32_t mode = 0; mode < 3u; ++mode) {
    if (mode == 0)
      pcg_col40();
    else if (mode == 1)
      pcg_col80();
    else
      pcg_mode80x16();
    pcg_locate(3u, 4u);
    pcg_colorfg(0x12u);
    pcg_colorbg(0x23u);
    for (uint32_t i = 0; i < 0x1000u; ++i) {
      CPT32_MMIO8(pcg_vram_base + i) = 'X';
      CPT32_MMIO8(pcg_vram_base + 0x1000u + i) = 0x12u;
      CPT32_MMIO8(pcg_vram_base + 0x2000u + i) = 0x23u;
    }
    pcg_cls(0x15u);
    for (uint32_t i = 0; i < 0x1000u; ++i) {
      if (CPT32_MMIO8(pcg_vram_base + i) != 0)
        return 107;
      if (CPT32_MMIO8(pcg_vram_base + 0x1000u + i) != 0x15u)
        return 108;
      if (CPT32_MMIO8(pcg_vram_base + 0x2000u + i) != 0x15u)
        return 109;
    }
    if (pcg_cur_x != 3u || pcg_cur_y != 4u || pcg_scroll_y != 64u ||
        pcg_fg_color != 0x12u || pcg_bg_color != 0x23u)
      return 110;
    pcg_putcscr('Y');
    uint32_t p = pcg_vram_base + pcg_vram_row * pcg_width + 3u;
    if (CPT32_MMIO8(p) != 'Y' || CPT32_MMIO8(p + 0x1000u) != 0x12u ||
        CPT32_MMIO8(p + 0x2000u) != 0x23u)
      return 111;
  }
  return 42;
}
