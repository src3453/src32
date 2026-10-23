/* compiler-rt compatible floating-point routines for SRC32, on Berkeley SoftFloat.
 *
 * The backend carries float and double as their bit patterns and turns every
 * floating-point instruction into a call of one of these routines (fp.py lists
 * which). They are therefore declared on uint32_t / uint64_t, the very
 * words the call ABI passes. SoftFloat runs with its defaults: round to nearest
 * even, and exception flags are only recorded (softfloat_exceptionFlags).
 *
 * The driver omits each definition the user's modules already provide
 * (SRC32_HAVE_<name>), as for string.c. */
#include <stdint.h>
#include "softfloat.h"

static inline float32_t f32(uint32_t v) { float32_t r; r.v = v; return r; }
static inline float64_t f64(uint64_t v) { float64_t r; r.v = v; return r; }

#define SF2(name, expr) uint32_t name(uint32_t a, uint32_t b) { float32_t x = f32(a), y = f32(b); (void)x; (void)y; return expr; }
#define DF2(name, expr) uint64_t name(uint64_t a, uint64_t b) { float64_t x = f64(a), y = f64(b); (void)x; (void)y; return expr; }

#ifndef SRC32_HAVE___addsf3
SF2(__addsf3, f32_add(x, y).v)
#endif
#ifndef SRC32_HAVE___subsf3
SF2(__subsf3, f32_sub(x, y).v)
#endif
#ifndef SRC32_HAVE___mulsf3
SF2(__mulsf3, f32_mul(x, y).v)
#endif
#ifndef SRC32_HAVE___divsf3
SF2(__divsf3, f32_div(x, y).v)
#endif
#ifndef SRC32_HAVE___adddf3
DF2(__adddf3, f64_add(x, y).v)
#endif
#ifndef SRC32_HAVE___subdf3
DF2(__subdf3, f64_sub(x, y).v)
#endif
#ifndef SRC32_HAVE___muldf3
DF2(__muldf3, f64_mul(x, y).v)
#endif
#ifndef SRC32_HAVE___divdf3
DF2(__divdf3, f64_div(x, y).v)
#endif

/* Comparisons return like compiler-rt: __eq/__ne: 0 iff equal / not equal
 * (unordered: nonzero); __lt/__le: < 0 / <= 0 iff true (unordered: 1);
 * __gt/__ge: > 0 / >= 0 iff true (unordered: -1); __unord: nonzero iff NaN. */
static inline int isnan32(float32_t x) { return (x.v & 0x7FFFFFFFu) > 0x7F800000u; }
static inline int isnan64(float64_t x) { return (x.v & 0x7FFFFFFFFFFFFFFFull) > 0x7FF0000000000000ull; }

#ifndef SRC32_HAVE___eqsf2
int __eqsf2(uint32_t a, uint32_t b) { return !f32_eq(f32(a), f32(b)); }
#endif
#ifndef SRC32_HAVE___nesf2
int __nesf2(uint32_t a, uint32_t b) { return !f32_eq(f32(a), f32(b)); }
#endif
#ifndef SRC32_HAVE___ltsf2
int __ltsf2(uint32_t a, uint32_t b) {
    if (isnan32(f32(a)) || isnan32(f32(b))) return 1;
    return f32_lt_quiet(f32(a), f32(b)) ? -1 : f32_lt_quiet(f32(b), f32(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___lesf2
int __lesf2(uint32_t a, uint32_t b) {
    if (isnan32(f32(a)) || isnan32(f32(b))) return 1;
    return f32_lt_quiet(f32(a), f32(b)) ? -1 : f32_lt_quiet(f32(b), f32(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___gtsf2
int __gtsf2(uint32_t a, uint32_t b) {
    if (isnan32(f32(a)) || isnan32(f32(b))) return -1;
    return f32_lt_quiet(f32(a), f32(b)) ? -1 : f32_lt_quiet(f32(b), f32(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___gesf2
int __gesf2(uint32_t a, uint32_t b) {
    if (isnan32(f32(a)) || isnan32(f32(b))) return -1;
    return f32_lt_quiet(f32(a), f32(b)) ? -1 : f32_lt_quiet(f32(b), f32(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___unordsf2
int __unordsf2(uint32_t a, uint32_t b) { return isnan32(f32(a)) || isnan32(f32(b)); }
#endif
#ifndef SRC32_HAVE___eqdf2
int __eqdf2(uint64_t a, uint64_t b) { return !f64_eq(f64(a), f64(b)); }
#endif
#ifndef SRC32_HAVE___nedf2
int __nedf2(uint64_t a, uint64_t b) { return !f64_eq(f64(a), f64(b)); }
#endif
#ifndef SRC32_HAVE___ltdf2
int __ltdf2(uint64_t a, uint64_t b) {
    if (isnan64(f64(a)) || isnan64(f64(b))) return 1;
    return f64_lt_quiet(f64(a), f64(b)) ? -1 : f64_lt_quiet(f64(b), f64(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___ledf2
int __ledf2(uint64_t a, uint64_t b) {
    if (isnan64(f64(a)) || isnan64(f64(b))) return 1;
    return f64_lt_quiet(f64(a), f64(b)) ? -1 : f64_lt_quiet(f64(b), f64(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___gtdf2
int __gtdf2(uint64_t a, uint64_t b) {
    if (isnan64(f64(a)) || isnan64(f64(b))) return -1;
    return f64_lt_quiet(f64(a), f64(b)) ? -1 : f64_lt_quiet(f64(b), f64(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___gedf2
int __gedf2(uint64_t a, uint64_t b) {
    if (isnan64(f64(a)) || isnan64(f64(b))) return -1;
    return f64_lt_quiet(f64(a), f64(b)) ? -1 : f64_lt_quiet(f64(b), f64(a)) ? 1 : 0;
}
#endif
#ifndef SRC32_HAVE___unorddf2
int __unorddf2(uint64_t a, uint64_t b) { return isnan64(f64(a)) || isnan64(f64(b)); }
#endif

/* Conversions. Float to integer truncates toward zero (C semantics);
 * integer to float rounds to nearest even. */
#ifndef SRC32_HAVE___extendsfdf2
uint64_t __extendsfdf2(uint32_t a) { return f32_to_f64(f32(a)).v; }
#endif
#ifndef SRC32_HAVE___truncdfsf2
uint32_t __truncdfsf2(uint64_t a) { return f64_to_f32(f64(a)).v; }
#endif
#ifndef SRC32_HAVE___fixsfsi
int32_t __fixsfsi(uint32_t a) { return f32_to_i32_r_minMag(f32(a), 0); }
#endif
#ifndef SRC32_HAVE___fixunssfsi
uint32_t __fixunssfsi(uint32_t a) { return f32_to_ui32_r_minMag(f32(a), 0); }
#endif
#ifndef SRC32_HAVE___fixdfsi
int32_t __fixdfsi(uint64_t a) { return f64_to_i32_r_minMag(f64(a), 0); }
#endif
#ifndef SRC32_HAVE___fixunsdfsi
uint32_t __fixunsdfsi(uint64_t a) { return f64_to_ui32_r_minMag(f64(a), 0); }
#endif
#ifndef SRC32_HAVE___fixsfdi
int64_t __fixsfdi(uint32_t a) { return f32_to_i64_r_minMag(f32(a), 0); }
#endif
#ifndef SRC32_HAVE___fixunssfdi
uint64_t __fixunssfdi(uint32_t a) { return f32_to_ui64_r_minMag(f32(a), 0); }
#endif
#ifndef SRC32_HAVE___fixdfdi
int64_t __fixdfdi(uint64_t a) { return f64_to_i64_r_minMag(f64(a), 0); }
#endif
#ifndef SRC32_HAVE___fixunsdfdi
uint64_t __fixunsdfdi(uint64_t a) { return f64_to_ui64_r_minMag(f64(a), 0); }
#endif
#ifndef SRC32_HAVE___floatsisf
uint32_t __floatsisf(int32_t a) { return i32_to_f32(a).v; }
#endif
#ifndef SRC32_HAVE___floatunsisf
uint32_t __floatunsisf(uint32_t a) { return ui32_to_f32(a).v; }
#endif
#ifndef SRC32_HAVE___floatsidf
uint64_t __floatsidf(int32_t a) { return i32_to_f64(a).v; }
#endif
#ifndef SRC32_HAVE___floatunsidf
uint64_t __floatunsidf(uint32_t a) { return ui32_to_f64(a).v; }
#endif
#ifndef SRC32_HAVE___floatdisf
uint32_t __floatdisf(int64_t a) { return i64_to_f32(a).v; }
#endif
#ifndef SRC32_HAVE___floatundisf
uint32_t __floatundisf(uint64_t a) { return ui64_to_f32(a).v; }
#endif
#ifndef SRC32_HAVE___floatdidf
uint64_t __floatdidf(int64_t a) { return i64_to_f64(a).v; }
#endif
#ifndef SRC32_HAVE___floatundidf
uint64_t __floatundidf(uint64_t a) { return ui64_to_f64(a).v; }
#endif

/* libm routines the compiler lowers intrinsics to (and C code may call). */
#ifndef SRC32_HAVE_sqrtf
uint32_t sqrtf(uint32_t a) { return f32_sqrt(f32(a)).v; }
#endif
#ifndef SRC32_HAVE_sqrt
uint64_t sqrt(uint64_t a) { return f64_sqrt(f64(a)).v; }
#endif
#ifndef SRC32_HAVE_fmaf
uint32_t fmaf(uint32_t a, uint32_t b, uint32_t c) { return f32_mulAdd(f32(a), f32(b), f32(c)).v; }
#endif
#ifndef SRC32_HAVE_fma
uint64_t fma(uint64_t a, uint64_t b, uint64_t c) { return f64_mulAdd(f64(a), f64(b), f64(c)).v; }
#endif

/* fmod: exact remainder by shift-and-subtract on the bit patterns, with the
 * sign of x. The result is NaN for an infinite x, a zero or NaN y, or a NaN x. */
#ifndef SRC32_HAVE_fmodf
uint32_t fmodf(uint32_t x, uint32_t y) {
    uint32_t ux = x, uy = y, sign = x & 0x80000000u;
    int ex = (int)(ux >> 23 & 0xFF), ey = (int)(uy >> 23 & 0xFF), i;
    if ((uy << 1) == 0 || isnan32(f32(y)) || ex == 0xFF) {
        float32_t product = f32_mul(f32(x), f32(y));
        return f32_div(product, product).v;
    }
    if ((ux << 1) <= (uy << 1)) return (ux << 1) == (uy << 1) ? sign : x;
    if (!ex) {
        for (i = (int)(ux << 9); i >= 0; ex--, i = (int)((uint32_t)i << 1)) {}
        ux <<= -ex + 1;
    } else {
        ux = (ux & (0xFFFFFFFFu >> 9)) | (1u << 23);
    }
    if (!ey) {
        for (i = (int)(uy << 9); i >= 0; ey--, i = (int)((uint32_t)i << 1)) {}
        uy <<= -ey + 1;
    } else {
        uy = (uy & (0xFFFFFFFFu >> 9)) | (1u << 23);
    }
    for (; ex > ey; ex--) {
        uint32_t d = ux - uy;
        if (!(d >> 31)) {
            if (d == 0) return sign;
            ux = d;
        }
        ux <<= 1;
    }
    {
        uint32_t d = ux - uy;
        if (!(d >> 31)) {
            if (d == 0) return sign;
            ux = d;
        }
    }
    for (; ux >> 23 == 0; ux <<= 1, ex--) {}
    if (ex > 0) ux = (ux - (1u << 23)) | ((uint32_t)ex << 23);
    else ux >>= -ex + 1;
    return ux | sign;
}
#endif

#ifndef SRC32_HAVE_fmod
uint64_t fmod(uint64_t x, uint64_t y) {
    uint64_t ux = x, uy = y, sign = x >> 63 << 63;
    int ex = (int)(ux >> 52 & 0x7FF), ey = (int)(uy >> 52 & 0x7FF);
    int64_t i;
    if ((uy << 1) == 0 || isnan64(f64(y)) || ex == 0x7FF) {
        float64_t product = f64_mul(f64(x), f64(y));
        return f64_div(product, product).v;
    }
    if ((ux << 1) <= (uy << 1)) return (ux << 1) == (uy << 1) ? sign : x;
    if (!ex) {
        for (i = (int64_t)(ux << 12); i >= 0; ex--, i = (int64_t)((uint64_t)i << 1)) {}
        ux <<= -ex + 1;
    } else {
        ux = (ux & (0xFFFFFFFFFFFFFFFFull >> 12)) | (1ull << 52);
    }
    if (!ey) {
        for (i = (int64_t)(uy << 12); i >= 0; ey--, i = (int64_t)((uint64_t)i << 1)) {}
        uy <<= -ey + 1;
    } else {
        uy = (uy & (0xFFFFFFFFFFFFFFFFull >> 12)) | (1ull << 52);
    }
    for (; ex > ey; ex--) {
        uint64_t d = ux - uy;
        if (!(d >> 63)) {
            if (d == 0) return sign;
            ux = d;
        }
        ux <<= 1;
    }
    {
        uint64_t d = ux - uy;
        if (!(d >> 63)) {
            if (d == 0) return sign;
            ux = d;
        }
    }
    for (; ux >> 52 == 0; ux <<= 1, ex--) {}
    if (ex > 0) ux = (ux - (1ull << 52)) | ((uint64_t)ex << 52);
    else ux >>= -ex + 1;
    return ux | sign;
}
#endif
