/* Relocatable executable (driver --pie, doc/spec_WAX_v3.md §3.1) for the
 * loader test. Static data that holds addresses (a pointer to a global, a
 * table of function pointers) only works if the loader relocated it, and the
 * code builds the addresses of those objects the same way. It runs several
 * times at once, each copy in an arena of its own.
 *
 * Exit code: 42 when everything checks out and the heap grew past the
 * kernel's default reserve in place; 43 when only the growth was refused
 * because another arena sits right above (every copy but the last, which
 * the test relies on being so: it asks before any copy can finish).
 * Anything else is a failure. */
#include <foundation/sys.h>

static uint32_t counter = 7;
static uint32_t *const where = &counter;

static uint32_t twice(uint32_t x) { return 2u * x; }
static uint32_t thrice(uint32_t x) { return 3u * x; }
static uint32_t (*const ops[2])(uint32_t) = {twice, thrice};
static const char *const names[2] = {"twice", "thrice"};

int main(void) {
    int32_t status = 1;
    if (where != &counter || *where != 7u || ops[0] != twice || ops[1](ops[0](5u)) != 30u ||
        names[1][0] != 't' || names[1][1] != 'h')
        fnd_sys_proc_exit(status);
    /* The heap, well beyond the reserve, asked for first thing: while the
     * copies are still starting, the next arena is there for all but the
     * last. */
    int32_t first = fnd_sys_mem_grow(0x30000u);
    if (first == FND_E_NO_MEMORY)
        fnd_sys_proc_exit(43);
    if (first <= 0)
        fnd_sys_proc_exit(2);
    uint32_t *heap = (uint32_t *)(uintptr_t)first;
    for (uint32_t i = 0; i < 0x2000u; ++i)
        heap[i] = i ^ (uint32_t)first;
    for (uint32_t i = 0; i < 0x2000u; ++i) {
        if (heap[i] != (i ^ (uint32_t)first))
            fnd_sys_proc_exit(3);
    }
    uint32_t *top = (uint32_t *)(uintptr_t)(first + 0x30000 - 4);
    *top = 0xC0FFEEu;
    fnd_sys_proc_exit(*top == 0xC0FFEEu ? 42 : 4);
    for (;;) {
    }
}
