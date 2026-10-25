#ifndef PKCL_BYTESWAP_H
#define PKCL_BYTESWAP_H
#include <stdint.h>

#define bswap_16(x) ((uint16_t)__builtin_bswap16((uint16_t)(x)))
#define bswap_32(x) ((uint32_t)__builtin_bswap32((uint32_t)(x)))
#define bswap_64(x) ((uint64_t)__builtin_bswap64((uint64_t)(x)))

#endif
