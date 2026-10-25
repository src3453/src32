/* The reentrant allocator entry points Newlib's own code calls (the posix
 * profile), on top of the standalone nano allocator. Single context: no locks. */
#include <stddef.h>

void *malloc(size_t size);
void free(void *p);
void *realloc(void *p, size_t size);
void *calloc(size_t count, size_t size);

struct _reent;

void *_malloc_r(struct _reent *reent, size_t size) { (void)reent; return malloc(size); }
void _free_r(struct _reent *reent, void *p) { (void)reent; free(p); }
void *_realloc_r(struct _reent *reent, void *p, size_t size) { (void)reent; return realloc(p, size); }
void *_calloc_r(struct _reent *reent, size_t count, size_t size) { (void)reent; return calloc(count, size); }
void __malloc_lock(struct _reent *reent) { (void)reent; }
void __malloc_unlock(struct _reent *reent) { (void)reent; }
