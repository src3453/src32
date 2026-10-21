/* Supervisor-mode kernel: sets up user protection, runs user.c in user mode
 * and serves its SYSCALLs. Returns the user's exit code (42 on success). */
#include <cpt32/cpu.h>
#include <cpt32/pcg.h>

#include "syscalls.h"

static void put_hex(uint32_t value) {
    char text[11] = {'0', 'x'};
    for (int digit = 0; digit < 8; ++digit) {
        uint32_t nibble = (value >> (28 - 4 * digit)) & 0xFu;
        text[2 + digit] = (char)(nibble < 10u ? '0' + nibble : 'A' + nibble - 10u);
    }
    text[10] = 0;
    pcg_prnlnscr(text);
}

/* Called by the compiler's SYSCALL entry on the supervisor stack. frame->r[n]
 * holds Rn of the caller; writing frame->r[1] sets the SYSCALL result. */
void syscall_dispatch(cpt32_trap_frame *frame) {
    uint32_t cause = frame->r[0];
    uint32_t a1 = frame->r[1], a2 = frame->r[2];
    switch (CPT32_CAUSE_SYSCALL_NUMBER(cause)) {
    case SYS_EXIT:
        cpt32_return_to_kernel(a1);
    case SYS_PUT_HEX:
        pcg_colorfg(CPT32_CAUSE_SYSCALL_CONTEXT(cause));
        pcg_prnscr("user says ");
        put_hex(a1);
        pcg_colorfg(0x3Fu);
        frame->r[1] = 0;
        break;
    case SYS_ADD:
        frame->r[1] = a1 + a2;
        break;
    case SYS_CALLER_MODE:
        frame->r[1] = (cpt32_mfsr(CPT32_SR_STATUS) & CPT32_STATUS_PU) ? 1u : 0u;
        break;
    case SYS_POKE:
        /* The kernel runs unprotected, so it must check user pointers itself:
         * only the user stack is writable on the user's behalf. */
        if (a1 < USER_STACK_BASE || a1 > USER_STACK_TOP - 4u) {
            frame->r[1] = SYS_ERROR;
        } else {
            *(volatile uint32_t *)a1 = a2;
            frame->r[1] = 0;
        }
        break;
    default:
        frame->r[1] = SYS_ERROR;
        break;
    }
}

int main(void) {
    pcg_initPCG();
    pcg_cls(0);
    pcg_colorfg(0x3Fu);
    pcg_colorbg(0);
    pcg_prnlnscr("kernel: entering user mode");

    uint32_t code_start = (uint32_t)(uintptr_t)user_main;
    uint32_t code_end = (uint32_t)(uintptr_t)user_code_end;
    if (code_end <= code_start)
        return 2; /* user.c must be linked with user_main before user_code_end */

    /* Region 0: user code (R/X). Region 1: user stack (R/W). Everything
     * else, including this kernel and all devices, stays denied. */
    cpt32_mtsr(CPT32_SR_RGN_START(0), code_start);
    cpt32_mtsr(CPT32_SR_RGN_END(0), code_end);
    cpt32_mtsr(CPT32_SR_RGN_FLAGS(0), CPT32_PERM_R | CPT32_PERM_X);
    cpt32_mtsr(CPT32_SR_RGN_START(1), USER_STACK_BASE);
    cpt32_mtsr(CPT32_SR_RGN_END(1), USER_STACK_TOP);
    cpt32_mtsr(CPT32_SR_RGN_FLAGS(1), CPT32_PERM_R | CPT32_PERM_W);

    uint32_t code = cpt32_run_user(user_main, USER_STACK_TOP);

    /* Back in supervisor mode; the user stack pointer is still saved in USP. */
    if (cpt32_mfsr(CPT32_SR_STATUS) & CPT32_STATUS_U)
        return 3;
    pcg_prnscr("kernel: user exited with ");
    put_hex(code);
    return (int)code;
}
