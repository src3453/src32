/* User-mode tasks of the phase 1 demo. They may execute only
 * [user_code_begin, user_code_end) and write only their own memory, so they
 * use no globals, string literals or calls. */
#include <foundation/sys.h>

#include "tasks.h"

/* Marks the start of the user code region; never called. Keep it first. */
void user_code_begin(void) {
}

/* Counts forever without yielding: the timer slices it with the others. */
void task_count(volatile uint32_t *box) {
    for (;;)
        ++box[0];
}

/* Counts for a while, then writes to kernel memory: the protection fault
 * ends this process and the others keep running. */
void task_crash(volatile uint32_t *box) {
    while (box[0] < CRASH_AFTER)
        ++box[0];
    *(volatile uint32_t *)(uintptr_t)0x00010000u = 0;
    for (;;) {
    }
}

/* SERVICE priority: wakes every TICKER_SLEEP ticks, preempting the counters. */
void task_ticker(volatile uint32_t *box) {
    for (;;) {
        fnd_sys_sleep(TICKER_SLEEP);
        ++box[0];
    }
}

/* Marks the end of the user code region; never called. Keep it last. */
void user_code_end(void) {
}
