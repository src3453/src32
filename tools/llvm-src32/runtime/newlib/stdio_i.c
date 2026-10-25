/* The posix profile (doc/spec_pkcl.md §9) runs one formatter. Newlib keeps a second copy for strings
 * (_svfprintf_r, 63 KB) and a third for integers only (_vfiprintf_r and _svfiprintf_r, 32 KB each).
 * A string FILE has no file descriptor, so the full formatter prints into it directly. */
#include <stdarg.h>
#include <stdio.h>
#include <reent.h>

int _vfiprintf_r(struct _reent *r, FILE *fp, const char *fmt, va_list ap) { return _vfprintf_r(r, fp, fmt, ap); }
int vfiprintf(FILE *fp, const char *fmt, va_list ap) { return vfprintf(fp, fmt, ap); }
int _svfiprintf_r(struct _reent *r, FILE *fp, const char *fmt, va_list ap) { return _svfprintf_r(r, fp, fmt, ap); }
int _svfprintf_r(struct _reent *r, FILE *fp, const char *fmt, va_list ap) { return _vfprintf_r(r, fp, fmt, ap); }
