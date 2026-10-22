/* PROTECTED WAX v2 program for the loader test: globals in its RW block,
 * constants in its RX block. Foundation's exit is proc_exit, not the
 * toolchain's SYSCALL 0 (doc/spec_Foundation.md §12). */
#include <foundation/sys.h>

static const uint32_t weights[4] = {1, 2, 3, 4};
static uint32_t squares[16];
static uint32_t zeroed[8];

int main(void) {
    uint32_t sum = 0;
    for (uint32_t i = 0; i < 16u; ++i)
        squares[i] = i * i;
    for (uint32_t i = 0; i < 16u; ++i)
        sum += squares[i] * weights[i & 3u];
    for (uint32_t i = 0; i < 8u; ++i)
        sum += zeroed[i];
    /* sum = 3400 */
    fnd_sys_proc_exit(sum == 3400u ? 42 : 1);
    for (;;) {
    }
}
