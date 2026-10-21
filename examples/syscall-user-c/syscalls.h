#ifndef SYSCALL_USER_SYSCALLS_H
#define SYSCALL_USER_SYSCALLS_H
#include <stdint.h>

/* System call numbers shared by the kernel (kernel.c) and the user program
 * (user.c). The numbers are this sample's own ABI; the CPU only carries them
 * in CAUSE bits 7..0. */
#define SYS_EXIT 0u        /* a1 = exit code; does not return */
#define SYS_PUT_HEX 1u     /* a1 = value; context byte = PCG foreground color */
#define SYS_ADD 2u         /* returns a1 + a2 */
#define SYS_CALLER_MODE 3u /* returns 1 when called from user mode */
#define SYS_POKE 4u        /* a1 = address, a2 = value; kernel refuses kernel memory */

#define SYS_ERROR 0xFFFFFFFFu

/* Layout of the user program, programmed into the protection regions. */
#define USER_STACK_BASE 0x00080000u
#define USER_STACK_TOP 0x00090000u

/* user.c: the user entry point and a marker placed right after it. The kernel
 * grants R/X only on [user_main, user_code_end). */
void user_main(void);
void user_code_end(void);
#endif
