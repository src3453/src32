#ifndef SRC32_STDINT_H
#define SRC32_STDINT_H

typedef __INT8_TYPE__ int8_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __INT16_TYPE__ int16_t;
typedef __UINT16_TYPE__ uint16_t;
typedef __INT32_TYPE__ int32_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __INT64_TYPE__ int64_t;
typedef __UINT64_TYPE__ uint64_t;
typedef __INT_LEAST8_TYPE__ int_least8_t;
typedef __UINT_LEAST8_TYPE__ uint_least8_t;
typedef __INT_LEAST16_TYPE__ int_least16_t;
typedef __UINT_LEAST16_TYPE__ uint_least16_t;
typedef __INT_LEAST32_TYPE__ int_least32_t;
typedef __UINT_LEAST32_TYPE__ uint_least32_t;
typedef __INT_LEAST64_TYPE__ int_least64_t;
typedef __UINT_LEAST64_TYPE__ uint_least64_t;
typedef __INT_FAST8_TYPE__ int_fast8_t;
typedef __UINT_FAST8_TYPE__ uint_fast8_t;
typedef __INT_FAST16_TYPE__ int_fast16_t;
typedef __UINT_FAST16_TYPE__ uint_fast16_t;
typedef __INT_FAST32_TYPE__ int_fast32_t;
typedef __UINT_FAST32_TYPE__ uint_fast32_t;
typedef __INT_FAST64_TYPE__ int_fast64_t;
typedef __UINT_FAST64_TYPE__ uint_fast64_t;
typedef __INTPTR_TYPE__ intptr_t;
typedef __UINTPTR_TYPE__ uintptr_t;
typedef __INTMAX_TYPE__ intmax_t;
typedef __UINTMAX_TYPE__ uintmax_t;

#define INT8_MIN (-127 - 1)
#define INT8_MAX 127
#define UINT8_MAX 255
#define INT16_MIN (-32767 - 1)
#define INT16_MAX 32767
#define UINT16_MAX 65535
#define INT32_MIN (-2147483647 - 1)
#define INT32_MAX 2147483647
#define UINT32_MAX 4294967295U
#define INT64_MIN (-9223372036854775807LL - 1)
#define INT64_MAX 9223372036854775807LL
#define UINT64_MAX 18446744073709551615ULL
#define INTPTR_MIN INT32_MIN
#define INTPTR_MAX INT32_MAX
#define UINTPTR_MAX UINT32_MAX
#define INTMAX_MIN INT64_MIN
#define INTMAX_MAX INT64_MAX
#define UINTMAX_MAX UINT64_MAX
#define SIZE_MAX UINT32_MAX
#define PTRDIFF_MIN INT32_MIN
#define PTRDIFF_MAX INT32_MAX
#define INT_LEAST8_MIN INT8_MIN
#define INT_LEAST8_MAX INT8_MAX
#define UINT_LEAST8_MAX UINT8_MAX
#define INT_LEAST16_MIN INT16_MIN
#define INT_LEAST16_MAX INT16_MAX
#define UINT_LEAST16_MAX UINT16_MAX
#define INT_LEAST32_MIN INT32_MIN
#define INT_LEAST32_MAX INT32_MAX
#define UINT_LEAST32_MAX UINT32_MAX
#define INT_LEAST64_MIN INT64_MIN
#define INT_LEAST64_MAX INT64_MAX
#define UINT_LEAST64_MAX UINT64_MAX
#define INT_FAST8_MIN (-__INT_FAST8_MAX__ - 1)
#define INT_FAST8_MAX __INT_FAST8_MAX__
#define UINT_FAST8_MAX __UINT_FAST8_MAX__
#define INT_FAST16_MIN (-__INT_FAST16_MAX__ - 1)
#define INT_FAST16_MAX __INT_FAST16_MAX__
#define UINT_FAST16_MAX __UINT_FAST16_MAX__
#define INT_FAST32_MIN (-__INT_FAST32_MAX__ - 1)
#define INT_FAST32_MAX __INT_FAST32_MAX__
#define UINT_FAST32_MAX __UINT_FAST32_MAX__
#define INT_FAST64_MIN (-__INT_FAST64_MAX__ - 1)
#define INT_FAST64_MAX __INT_FAST64_MAX__
#define UINT_FAST64_MAX __UINT_FAST64_MAX__
#define SIG_ATOMIC_MIN (-__SIG_ATOMIC_MAX__ - 1)
#define SIG_ATOMIC_MAX __SIG_ATOMIC_MAX__
#ifdef __WCHAR_UNSIGNED__
#define WCHAR_MIN 0
#else
#define WCHAR_MIN (-__WCHAR_MAX__ - 1)
#endif
#define WCHAR_MAX __WCHAR_MAX__
#ifdef __WINT_UNSIGNED__
#define WINT_MIN 0
#else
#define WINT_MIN (-__WINT_MAX__ - 1)
#endif
#define WINT_MAX __WINT_MAX__
#define INT8_C(x) x
#define UINT8_C(x) x
#define INT16_C(x) x
#define UINT16_C(x) x
#define INT32_C(x) x
#define UINT32_C(x) x##U
#define INT64_C(x) x##LL
#define UINT64_C(x) x##ULL
#define INTMAX_C(x) INT64_C(x)
#define UINTMAX_C(x) UINT64_C(x)

#endif
