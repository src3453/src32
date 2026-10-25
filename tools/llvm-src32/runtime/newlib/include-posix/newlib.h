/* CPT32 configuration for the Newlib "posix" profile (doc/spec_pkcl.md §9).
 * Newlib normally generates this file with configure; the values here are the
 * ones a single-threaded, 32-bit big-endian target without a hardware FPU
 * (double == long double) needs. */
#ifndef __NEWLIB_H__
#define __NEWLIB_H__ 1
#include <_newlib_version.h>
#define __SINGLE_THREAD__ 1
#define _WANT_USE_LONG_TIME_T 1
#define _LDBL_EQ_DBL 1
#define _ATEXIT_DYNAMIC_ALLOC 1
#define _FVWRITE_IN_STREAMIO 1
#define _FSEEK_OPTIMIZATION 1
#define _UNBUF_STREAM_OPT 1
#define _WANT_IO_C99_FORMATS 1
#define _WANT_IO_LONG_LONG 1
#define _WANT_USE_GDTOA 1
#define _MB_CAPABLE 1
#define _MB_LEN_MAX 8
#define _REENT_CHECK_VERIFY 1
#endif
