#ifndef CPT32_COLORS_H
#define CPT32_COLORS_H
#include <stdint.h>
/* RGB成分を b + (g << 2) + (r << 4) の色番号に変換します。 */
uint32_t colors_tocolor(uint32_t red, uint32_t green, uint32_t blue);
#endif
