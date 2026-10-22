/* libfnd memory (doc/spec_Foundation_API.md §11.8): first fit over a heap
 * that mem_grow extends. Every block has an 8-byte header; free neighbours
 * merge when a block is freed. The heap never shrinks (v1). */
#include <foundation/fnd.h>
#include <foundation/sys.h>

struct block {
    uint32_t size; /* bytes after the header, a multiple of 8 */
    uint32_t used;
};

static struct block *first;
static uint8_t *heap_end;

static struct block *next_of(struct block *b) { return (struct block *)((uint8_t *)(b + 1) + b->size); }

void *fnd_mem_alloc(uint32_t size) {
    if (size == 0 || size > 0x7FFFFFF0u)
        return 0;
    size = (size + 7u) & ~7u;
    struct block *last = 0;
    for (struct block *b = first; b && (uint8_t *)b < heap_end; b = next_of(b)) {
        if (!b->used && b->size >= size) {
            if (b->size >= size + 2u * sizeof(struct block)) {
                struct block *rest = (struct block *)((uint8_t *)(b + 1) + size);
                rest->size = b->size - size - sizeof(struct block);
                rest->used = 0;
                b->size = size;
            }
            b->used = 1;
            return b + 1;
        }
        last = b;
    }
    uint32_t need = size + sizeof(struct block);
    if (last && !last->used)
        need -= last->size + sizeof(struct block); /* grow the free tail */
    int32_t old = fnd_sys_mem_grow(need);
    if (old < 0)
        return 0;
    if (!first)
        first = (struct block *)(uintptr_t)(uint32_t)old;
    heap_end = (uint8_t *)(uintptr_t)(uint32_t)old + need;
    struct block *b = last && !last->used ? last : (struct block *)(uintptr_t)(uint32_t)old;
    b->size = size;
    b->used = 1;
    return b + 1;
}

void fnd_mem_free(void *p) {
    if (!p)
        return;
    struct block *b = (struct block *)p - 1;
    b->used = 0;
    for (struct block *c = first; c && (uint8_t *)c < heap_end; c = next_of(c)) {
        while (!c->used && (uint8_t *)next_of(c) < heap_end && !next_of(c)->used)
            c->size += next_of(c)->size + sizeof(struct block);
    }
}
