/* User-mode tasks of the phase 1 self-test. They may execute only
 * [user_code_begin, user_code_end) and write only their own slot, so they
 * use no globals, string literals or calls. */
#include <foundation/sys.h>

#include "tasks.h"

/* Marks the start of the user code region; never called. Keep it first. */
void user_code_begin(void) {
}

/* Never yields: only the timer tick can switch away from it. */
void task_count(volatile uint32_t *box) {
    for (;;)
        ++box[0];
}

/* Counts a while, then stores into another process's slot: the protection
 * fault must end this process only. */
void task_fault(volatile uint32_t *box) {
    while (box[0] < FAULT_AFTER)
        ++box[0];
    *(volatile uint32_t *)(uintptr_t)SLOT_A_MAILBOX = 0xDEADu;
    for (;;) {
    }
}

/* SERVICE priority: checks system call results, sleeps PROBE_SLEEPS times
 * recording how long each sleep took, then exits. */
void task_probe(volatile uint32_t *box) {
    volatile char message[4];
    box[PROBE_BAD_POINTER] = (uint32_t)fnd_sys_debug_write(0x100u, 4u);
    box[PROBE_BAD_NUMBER] = cpt32_syscall(0x3Eu, 0u, 0u, 0u, 0u, 0u);
    box[PROBE_BAD_CONTEXT] = cpt32_syscall(FND_SYS_YIELD, 1u, 0u, 0u, 0u, 0u);
    box[PROBE_IRQ_ACCESS] = cpt32_syscall(FND_SYS_IRQ_BIND, 0u, 0u, 0u, 0u, 0u);
    box[PROBE_TICK_HZ] = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    /* Length 0 only: src32_testbench has no UART, so writing a byte would
     * be a bus error in the kernel. */
    box[PROBE_WRITTEN] = (uint32_t)fnd_sys_debug_write(message, 0u) + 1u;
    uint32_t cpu[5];
    cpu[4] = 0x12345678u;
    box[PROBE_CPU_SIZE] = (uint32_t)fnd_sys_info(2u, 0, 0);
    box[PROBE_CPU_BAD_POINTER] = (uint32_t)fnd_sys_info(2u, (void *)0x100u, 16u);
    box[PROBE_CPU_ALIGNMENT] = (uint32_t)fnd_sys_info(2u, (void *)((uintptr_t)cpu + 1u), 16u);
    box[PROBE_CPU_BAD_SELECTOR] = (uint32_t)fnd_sys_info(4u, cpu, sizeof cpu);
    int32_t needed = fnd_sys_info(2u, cpu, 16u);
    box[PROBE_CPU_PREFIX] = needed == 48 && cpu[4] == 0x12345678u && cpu[0] > 0 && cpu[1] > 0;
    box[PROBE_CPU_CLOCK] = cpu[3];
    box[PROBE_MIN_SLEEP] = 0xFFFFFFFFu;
    box[PROBE_MAX_SLEEP] = 0;
    while (box[PROBE_ROUNDS] < PROBE_SLEEPS) {
        uint32_t before = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO);
        fnd_sys_sleep(SLEEP_TICKS);
        uint32_t slept = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO) - before;
        if (slept < box[PROBE_MIN_SLEEP])
            box[PROBE_MIN_SLEEP] = slept;
        if (slept > box[PROBE_MAX_SLEEP])
            box[PROBE_MAX_SLEEP] = slept;
        ++box[PROBE_ROUNDS];
    }
    fnd_sys_proc_exit(PROBE_EXIT_BASE + box[PROBE_ROUNDS]);
    for (;;) {
    }
}

/* Marks the end of the user code region; never called. Keep it last. */
void user_code_end(void) {
}
