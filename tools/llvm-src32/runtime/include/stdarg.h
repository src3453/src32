#ifndef SRC32_STDARG_H
#define SRC32_STDARG_H
/* Clang lowers these builtins for the MIPS o32 va_list (a char pointer) to
 * llvm.va_start / llvm.va_copy and inline va_arg code; the SRC32 backend
 * implements the intrinsics (extra arguments follow the named ones on the
 * stack, 8-byte types aligned to 8). */
typedef __builtin_va_list va_list;
#define va_start(ap, last) __builtin_va_start(ap, last)
#define va_end(ap) __builtin_va_end(ap)
#define va_arg(ap, type) __builtin_va_arg(ap, type)
#define va_copy(dest, src) __builtin_va_copy(dest, src)
#endif
