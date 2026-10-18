#ifndef SRC32_STDDEF_H
#define SRC32_STDDEF_H

typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;
typedef __WCHAR_TYPE__ wchar_t;
typedef struct { long long _integer; long double _floating; } max_align_t;
#define NULL ((void *)0)
#define offsetof(type, member) __builtin_offsetof(type, member)

#endif
