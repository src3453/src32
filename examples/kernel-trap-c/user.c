/* User-mode tasks. They may execute only [user_code_begin, user_code_end) and
 * touch only their stacks, so they use no globals, string literals or calls. */
#include <cpt32/cpu.h>

#include "trap_demo.h"

/* Marks the start of the user code region; never called. Keep it first. */
void user_code_begin(void) {
}

/* Cooperative task: keeps three live values across every SYSCALL, so a
 * context switch that loses a register shows up as a wrong value. */
void task_mix(uint32_t seed) {
    uint32_t x = seed, y = seed ^ 0x5A5Au, z = 7u;
    for (uint32_t i = 0; i < 4u; ++i) {
        x = x * 3u + i;
        y = (y << 1) ^ x;
        z += y;
        CPT32_SYSCALL3(SYS_YIELD, x, y, z);
    }
    CPT32_SYSCALL1(SYS_EXIT, x + y + z);
    for (;;) {
    }
}

/* Never yields: only the timer IRQ can switch away from it. */
void task_spin(void) {
    uint32_t n = 0;
    for (;;) {
        ++n;
        if ((n & SPIN_REPORT_MASK) == 0u)
            CPT32_SYSCALL1(SYS_REPORT, n);
    }
}

/* Marks the end of the user code region; never called. Keep it last. */
void user_code_end(void) {
}
