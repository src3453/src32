#ifndef PKCL_ENDIAN_H
#define PKCL_ENDIAN_H
#include <machine/endian.h>

/* Newlib spells these with one underscore; glibc-style code (BusyBox) tests the double one. */
#ifndef __BYTE_ORDER
#define __BYTE_ORDER _BYTE_ORDER
#define __BIG_ENDIAN _BIG_ENDIAN
#define __LITTLE_ENDIAN _LITTLE_ENDIAN
#endif

#endif
