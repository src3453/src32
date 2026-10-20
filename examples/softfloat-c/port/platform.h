#ifndef CPT32_SOFTFLOAT_PLATFORM_H
#define CPT32_SOFTFLOAT_PLATFORM_H
/* SRC32 is big endian: upstream tests whether LITTLEENDIAN is defined.
 * Leave it undefined. Use portable out-of-line primitives (INLINE_LEVEL=0),
 * without host intrinsics, TLS, or native floating-point instructions. */
#define INLINE static inline
#endif
