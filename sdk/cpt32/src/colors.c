#include <cpt32/colors.h>
/* Preserve sol's arithmetic packing (including its four-bit red shift). */
uint32_t colors_tocolor(uint32_t r, uint32_t g, uint32_t b) {
  return b + (g << 2) + (r << 4);
}
