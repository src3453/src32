#ifndef FOUNDATION_PHASE1_TASKS_H
#define FOUNDATION_PHASE1_TASKS_H
#include <stdint.h>

/* Each process gets TASK_RAM bytes at the top of its slot; the task's
 * argument points at the bottom of it, where it keeps a mailbox of words
 * that the test kernel reads. */
#define TASK_RAM 0x4000u
#define SLOT_PROBE 3u
#define SLOT_FAULT 4u
#define SLOT_A 5u
#define SLOT_B 6u
#define SLOT_A_MAILBOX (0x00C00000u - TASK_RAM) /* written by task_fault */

#define FAULT_AFTER 1000u
#define SLEEP_TICKS 7u
#define PROBE_SLEEPS 5u /* then exits with PROBE_EXIT_BASE + rounds */
#define PROBE_EXIT_BASE 0x1200u

/* Probe mailbox words. */
enum {
    PROBE_ROUNDS, PROBE_MIN_SLEEP, PROBE_MAX_SLEEP, PROBE_BAD_POINTER,
    PROBE_BAD_NUMBER, PROBE_BAD_CONTEXT, PROBE_TICK_HZ, PROBE_WRITTEN,
    PROBE_CPU_SIZE, PROBE_CPU_BAD_POINTER, PROBE_CPU_ALIGNMENT,
    PROBE_CPU_PREFIX, PROBE_CPU_CLOCK, PROBE_CPU_BAD_SELECTOR, PROBE_IRQ_ACCESS
};

/* user.c: code granted R/X lies in [user_code_begin, user_code_end). */
void user_code_begin(void);
void task_count(volatile uint32_t *box);
void task_fault(volatile uint32_t *box);
void task_probe(volatile uint32_t *box);
void user_code_end(void);
#endif
