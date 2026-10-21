#ifndef KERNEL_TRAP_DEMO_H
#define KERNEL_TRAP_DEMO_H
#include <stdint.h>

/* System call numbers of this sample (not the Foundation ABI). */
#define SYS_YIELD 1u  /* a1..a3 = task state; switch to the other task */
#define SYS_EXIT 2u   /* a1 = result; the task never runs again */
#define SYS_REPORT 3u /* a1 = spin counter; returns to the same task */

/* User stacks: four 32 KiB stacks at 0x80000..0xA0000. */
#define USER_STACKS_BASE 0x00080000u
#define USER_STACK_SIZE 0x00008000u
#define USER_STACK_TOP(task) (USER_STACKS_BASE + USER_STACK_SIZE * ((task) + 1u))
#define USER_STACKS_END USER_STACK_TOP(3u)

/* SPIN_REPORT_MASK + 1 iterations between two SYS_REPORT calls. */
#define SPIN_REPORT_MASK 0x3FFu

/* user.c: code granted R/X lies in [user_code_begin, user_code_end). */
void user_code_begin(void);
void task_mix(uint32_t seed);
void task_spin(void);
void user_code_end(void);
#endif
