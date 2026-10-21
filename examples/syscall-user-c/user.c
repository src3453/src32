/* User-mode program. It runs with Extension P protection: it may execute only
 * [user_main, user_code_end) and touch only its stack, so it uses no globals,
 * no string literals and no library calls; everything else goes through
 * SYSCALL. */
#include <cpt32/cpu.h>

#include "syscalls.h"

void user_main(void) {
    uint32_t ok = 1;
    /* The kernel sees STATUS.PU = 1 for a SYSCALL made from user mode. */
    if (CPT32_SYSCALL0(SYS_CALLER_MODE) != 1u)
        ok = 0;
    uint32_t sum = CPT32_SYSCALL2(SYS_ADD, 40u, 2u);
    /* The context byte (0x0C) selects the text color for this call. */
    cpt32_syscall(SYS_PUT_HEX, 0x0Cu, sum, 0u, 0u, 0u);
    /* Asking the kernel to write kernel memory for us must be refused. */
    if (CPT32_SYSCALL2(SYS_POKE, 0x00000100u, 0u) != SYS_ERROR)
        ok = 0;
    CPT32_SYSCALL1(SYS_EXIT, ok ? sum : 1u);
    for (;;) {
    }
}

/* Marks the end of the user code region; never called. Keep it last. */
void user_code_end(void) {
}
