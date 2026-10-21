#ifndef CPT32_NEWLIB_PORT_H
#define CPT32_NEWLIB_PORT_H
#include <stddef.h>
/* Standalone nano-mallocr.c calls _sbrk_r without a reentrancy argument. */
#define _sbrk_r cpt32_newlib_sbrk
void *cpt32_newlib_sbrk(ptrdiff_t increment);
#endif
