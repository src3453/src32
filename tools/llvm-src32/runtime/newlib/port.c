/* Single-threaded, bounded storage for the optional Newlib nano allocator. */
#include <errno.h>
#include <stddef.h>
#include <stdint.h>

static _Alignas(8) unsigned char heap[CPT32_NEWLIB_HEAP_SIZE];
static size_t used;

#ifndef CPT32_NEWLIB_NO_ERRNO
static int error_number;

/* 単一実行コンテキストの errno の格納先を返す。 */
int *__errno(void) { return &error_number; }
#endif

/* 静的に予約したヒープ内で break を移動し、範囲外では ENOMEM を返す。 */
void *cpt32_newlib_sbrk(ptrdiff_t increment)
{
    size_t next;
    if (increment >= 0) {
        if ((size_t)increment > sizeof(heap) - used) goto exhausted;
        next = used + (size_t)increment;
    } else {
        size_t amount = 0u - (size_t)increment;
        if (amount > used) goto exhausted;
        next = used - amount;
    }
    void *previous = heap + used;
    used = next;
    return previous;
exhausted:
    errno = ENOMEM;
    return (void *)(intptr_t)-1;
}
