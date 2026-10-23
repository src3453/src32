/* Foundation phase 1 self-test (doc/foundation_os_plan.md §8): two U-mode
 * counting tasks that never yield advance on the timer tick, a third one
 * dies on a protection fault while the others keep running, and a SERVICE
 * task checks sleep, priority preemption and system call errors, then exits.
 * The probe is gone before CHECK_TICK, so between CHECK_TICK and END_TICK
 * only the 1-tick (10 ms) time slice can move the CPU between the counters. Ends with
 * R1 = 42 at a HALT, or 0x100 + the number of failed checks. */
#include <foundation/kernel.h>

#include "tasks.h"

#define CHECK_TICK 50u  /* probe finishes its five 7-tick sleeps first */
#define END_TICK 100u   /* one second at 100 Hz */

static int32_t pid_a, pid_b, pid_fault, pid_probe;
static uint32_t failures, a_mid, b_mid;
static uint32_t a_last, b_last, last_ran, clock_mid;
/* Copy of the probe mailbox, refreshed every tick: its exit zeroes the slot. */
static uint32_t probe[PROBE_UNSUPPORTED + 1];

static volatile uint32_t *box(uint32_t slot) {
    return (volatile uint32_t *)(uintptr_t)(fnd_slot_end(slot) - TASK_RAM);
}

static void check(int ok) {
    if (!ok)
        ++failures;
}

static uint32_t on_tick(uint32_t ticks) {
    struct fnd_proc *p = fnd_proc_get((uint32_t)pid_probe);
    if (p && p->state == FND_PROC_LIVE) {
        for (unsigned i = 0; i <= PROBE_UNSUPPORTED; ++i)
            probe[i] = box(SLOT_PROBE)[i];
    }
    if (ticks == CHECK_TICK) {
        struct fnd_proc *fault = fnd_proc_get((uint32_t)pid_fault);
        uint32_t code_start = (uint32_t)(uintptr_t)user_code_begin;
        uint32_t code_end = (uint32_t)(uintptr_t)user_code_end;
        check(fault && fault->state == FND_PROC_EXITED);
        check(fault && fault->exit_code == (int32_t)0x80050002u); /* store fault */
        check(fault && code_start <= fault->fault_epc && fault->fault_epc < code_end);
        check(box(SLOT_FAULT)[0] == 0u); /* its slot was zeroed */
        a_mid = box(SLOT_A)[0];
        b_mid = box(SLOT_B)[0];
        a_last = a_mid;
        b_last = b_mid;
        clock_mid = *(volatile uint32_t *)(uintptr_t)0x80040448u; /* PeC ch1, 48 MHz */
        check(a_mid != 0u && b_mid != 0u);
        check(a_mid != 0xDEADu); /* the faulting store never landed */
        check(probe[PROBE_BAD_POINTER] == (uint32_t)FND_E_ACCESS);
        check(probe[PROBE_BAD_NUMBER] == (uint32_t)FND_E_BAD_CALL);
        check(probe[PROBE_BAD_CONTEXT] == (uint32_t)FND_E_BAD_CALL);
        check(probe[PROBE_UNSUPPORTED] == (uint32_t)FND_E_UNSUPPORTED);
        check(probe[PROBE_TICK_HZ] == FND_TICK_HZ);
        check(probe[PROBE_TICK_HZ] == 100u);
        check(probe[PROBE_WRITTEN] == 1u);
        check(probe[PROBE_CPU_SIZE] == 48u); /* header + four PID/cycle pairs */
        check(probe[PROBE_CPU_BAD_POINTER] == (uint32_t)FND_E_ACCESS);
        check(probe[PROBE_CPU_ALIGNMENT] == (uint32_t)FND_E_BAD_ARG);
        check(probe[PROBE_CPU_BAD_SELECTOR] == (uint32_t)FND_E_BAD_ARG);
        check(probe[PROBE_CPU_PREFIX] == 1u);
        check(probe[PROBE_CPU_CLOCK] == 48000000u);
        check(fault && fault->cpu_cycles > 0u); /* retained after exit */
        /* The SERVICE task preempted the APP tasks as soon as each sleep ended. */
        check(box(SLOT_PROBE)[PROBE_ROUNDS] == 0u); /* zeroed on exit */
        check(probe[PROBE_MIN_SLEEP] >= SLEEP_TICKS && probe[PROBE_MAX_SLEEP] <= SLEEP_TICKS + 1u);
        check(p && p->state == FND_PROC_EXITED && p->exit_code == (int32_t)(PROBE_EXIT_BASE + PROBE_SLEEPS));
        check(p && p->fault_epc == 0u);
    }
    if (ticks > CHECK_TICK) {
        uint32_t a = box(SLOT_A)[0], b = box(SLOT_B)[0];
        /* After the probe exits, exactly one counter runs per 10 ms tick. */
        check((a != a_last) != (b != b_last));
        uint32_t ran = a != a_last ? 1u : 2u;
        check(last_ran == 0u || ran != last_ran);
        last_ran = ran;
        a_last = a;
        b_last = b;
    }
    if (ticks < END_TICK)
        return 0;
    /* Check the real timer period independently of the tick constants.
     * Allow two ticks for trap latency; 1 ms and 100 ms both fail. */
    uint32_t elapsed = *(volatile uint32_t *)(uintptr_t)0x80040448u - clock_mid;
    check(elapsed >= (END_TICK - CHECK_TICK - 2u) * 480000u);
    check(elapsed <= (END_TICK - CHECK_TICK + 2u) * 480000u);
    /* Both counters kept going after the fault. */
    check(box(SLOT_A)[0] > a_mid && box(SLOT_B)[0] > b_mid);
    struct fnd_proc *a = fnd_proc_get((uint32_t)pid_a), *b = fnd_proc_get((uint32_t)pid_b);
    check(a && a->state == FND_PROC_LIVE && b && b->state == FND_PROC_LIVE);
    check(a && a->cpu_cycles > 0u && b && b->cpu_cycles > 0u);
    /* One-tick slices alternate A and B. */
    check(fnd_context_switches() >= END_TICK / FND_SLICE_TICKS);
    return failures == 0u ? 42u : 0x100u + failures;
}

static int32_t spawn(const char *name, uint32_t slot, uint32_t priority, void (*entry)(volatile uint32_t *)) {
    return fnd_proc_create_builtin(name, slot, priority, TASK_RAM,
                                   (uint32_t)(uintptr_t)user_code_begin,
                                   (uint32_t)(uintptr_t)user_code_end, (uint32_t)(uintptr_t)entry,
                                   (uint32_t)(uintptr_t)box(slot));
}

int main(void) {
    pid_a = spawn("count-a", SLOT_A, FND_PRIO_APP, task_count);
    pid_b = spawn("count-b", SLOT_B, FND_PRIO_APP, task_count);
    pid_fault = spawn("fault", SLOT_FAULT, FND_PRIO_APP, task_fault);
    pid_probe = spawn("probe", SLOT_PROBE, FND_PRIO_SERVICE, task_probe);
    if (pid_a < 0 || pid_b < 0 || pid_fault < 0 || pid_probe < 0)
        return 2;
    /* Slot conflicts and bad arguments are refused. */
    if (spawn("dup", SLOT_A, FND_PRIO_APP, task_count) != FND_E_ADDRESS_CONFLICT)
        return 3;
    if (spawn("bad", FND_SLOTS, FND_PRIO_APP, task_count) != FND_E_BAD_ARG)
        return 4;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
