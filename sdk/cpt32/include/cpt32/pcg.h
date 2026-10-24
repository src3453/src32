#ifndef CPT32_PCG_H
#define CPT32_PCG_H
#include <stdint.h>
#define CPT32_PCG_FONT_8X16_VRAM_OFFSET 0x21000u
#define CPT32_PCG_FONT_8X16_SIZE 0x1000u
/* mode 3 の2 byte文字: FG属性 bit 7:6 (WIDE_KIND) と WIDE_FONT_BASE (0xF010..0xF013, VRAM byte offset)。 */
#define CPT32_PCG_WIDE_KIND_SINGLE 0x00u
#define CPT32_PCG_WIDE_KIND_MSB 0x40u
#define CPT32_PCG_WIDE_KIND_LSB 0x80u
#define CPT32_PCG_WIDE_GLYPH_SIZE 32u
/* sol pcg namespace state is public and intentionally mutable by callers. */
extern uint32_t pcg_cur_x, pcg_cur_y, pcg_width, pcg_height, pcg_cell_height;
extern uint32_t pcg_fg_color, pcg_bg_color, pcg_mode, pcg_cursor;
extern uint32_t pcg_vram_base, pcg_scroll_y, pcg_vram_row;
/* 8×8フォントの40列表示へ切り替えます。 */
void pcg_col40(void);
/* 8×8フォントの80列表示へ切り替えます。 */
void pcg_col80(void);
/* 8×16フォントの80列×30行表示へ切り替えます。 */
void pcg_mode80x16(void);
/* 論理文字座標を設定し、移動先のFG/BGを現在色にしてカーソルを移動します。 */
void pcg_locate(uint32_t x, uint32_t y);
/* Clear all 4 KiB of text to glyph zero and both attribute planes to color.
 * Then prepares the cursor cell's FG/BG with the current output colors.
 * Preserves cursor position, scroll and the output color variables. */
/* 文字面と前景・背景属性面を全消去し、指定色で画面を埋めます。 */
void pcg_cls(uint32_t color);
/* PCGを40列×30行に初期化し、スクロールとカーソル位置をリセットします。 */
void pcg_initPCG(void);
/* 現在色で1文字を書き、右端では折り返して必要に応じてスクロールします。 */
void pcg_putcscr(uint32_t character);
/* NUL終端文字列を現在位置から書きます（改行文字も文字コードとして出力）。 */
void pcg_prnscr(const char *string);
/* NUL終端文字列を書き、次の行へ移動します。 */
void pcg_prnlnscr(const char *string);
/* 文字出力と現在のカーソル位置に使う前景色を設定します。 */
void pcg_colorfg(uint32_t color);
/* 文字出力と現在のカーソル位置に使う背景色を設定します。 */
void pcg_colorbg(uint32_t color);
#endif
