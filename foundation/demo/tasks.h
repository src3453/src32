#ifndef FOUNDATION_DEMO_TASKS_H
#define FOUNDATION_DEMO_TASKS_H
#include <stdint.h>

/* Each process gets TASK_RAM bytes at the top of its slot; the argument of
 * its task points at the bottom, where word 0 is a counter the kernel shows. */
#define TASK_RAM 0x4000u
#define CRASH_AFTER 1000000u
#define TICKER_SLEEP 250u /* ticks */

/* user.c: code granted R/X lies in [user_code_begin, user_code_end). */
void user_code_begin(void);
void task_count(volatile uint32_t *box);
void task_crash(volatile uint32_t *box);
void task_ticker(volatile uint32_t *box);
void user_code_end(void);
#endif
