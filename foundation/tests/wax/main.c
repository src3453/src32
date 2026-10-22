/* Foundation WAX loader test on src32_testbench (doc/spec_WAX_v3.md §8,
 * doc/spec_Foundation.md §9.2). images.h comes from build.py:
 * - app / app2: v3 executables in slots 5 and 6 that call test.math and
 *   test.base, each process with its own instances at different bases;
 * - poke_write / poke_exec: a store to RX and a jump into RW, both faults;
 * - v2app: a PROTECTED WAX v2 program in C;
 * - spin: keeps the CPU busy, as src32_testbench treats the idle HALT as
 *   the end of the program.
 * Rejected spawns change nothing. After the first round app is spawned again
 * into its freed slot. Ends with R1 = 42 at a HALT, or 0x100 + the number of
 * failed checks. */
#include <foundation/kernel.h>

#include "images.h"

#define IMAGE(name) img_##name, img_##name##_size
#define ROUND_TICKS 40u
#define ROUNDS 3u

#define SLOT_APP 5u
#define SLOT_APP2 6u
#define POKE_WRITE_BASE 0x00600000u
#define POKE_EXEC_DATA 0x00500100u

static const struct fnd_image libs[] = {{IMAGE(math_old)}, {IMAGE(math)}, {IMAGE(base)}};
static const struct fnd_image base_only[] = {{IMAGE(base)}};
static const struct fnd_image old_math[] = {{IMAGE(math_old)}, {IMAGE(base)}};

static int32_t pid_app, pid_app2, pid_v2, pid_write, pid_exec;
static uint32_t failures, rounds;

static void check(int ok) {
    if (!ok)
        ++failures;
}

static int exited(int32_t pid, int32_t code) {
    struct fnd_proc *p = fnd_proc_get((uint32_t)pid);
    return p && p->state == FND_PROC_EXITED && p->exit_code == code;
}

static int zero(uint32_t start, uint32_t end) {
    for (uint32_t a = start; a < end; a += 4u) {
        if (*(volatile uint32_t *)(uintptr_t)a)
            return 0;
    }
    return 1;
}

/* The executable, its libraries (placed right after it) and the stack are
 * zero: nothing loaded yet, or zeroed on exit. */
static int slot_clear(uint32_t slot) {
    return zero(fnd_slot_base(slot), fnd_slot_base(slot) + 0x4000u) &&
           zero(fnd_slot_end(slot) - 0x1000u, fnd_slot_end(slot));
}

static uint32_t on_tick(uint32_t ticks) {
    if (ticks % ROUND_TICKS)
        return 0;
    check(exited(pid_app, 42));
    check(slot_clear(SLOT_APP));
    if (rounds == 0) {
        check(exited(pid_app2, 42));
        check(exited(pid_v2, 42));
        struct fnd_proc *w = fnd_proc_get((uint32_t)pid_write);
        struct fnd_proc *x = fnd_proc_get((uint32_t)pid_exec);
        check(exited(pid_write, (int32_t)0x80050002u) && w->fault_epc == POKE_WRITE_BASE);
        check(exited(pid_exec, (int32_t)0x80050004u) && x->fault_epc == POKE_EXEC_DATA);
        check(slot_clear(SLOT_APP2));
    }
    if (++rounds == ROUNDS)
        return failures == 0u ? 42u : 0x100u + failures;
    /* Spawn and exit again: the slot, records and threads are reused. */
    pid_app = fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), libs, 3);
    check(pid_app > 0);
    return 0;
}

int main(void) {
    static const uint8_t garbage[64] = {'W', 'A', 'X', 0, 0, 3};
    /* Rejections are atomic: nothing is allocated or written. */
    check(fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), base_only, 1) == FND_E_DEPENDENCY);
    check(fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), old_math, 2) == FND_E_DEPENDENCY);
    check(fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app_unprotected), libs, 3) == FND_E_FORMAT);
    check(fnd_proc_spawn_wax("bad", FND_PRIO_APP, garbage, sizeof garbage, 0, 0) == FND_E_FORMAT);
    check(fnd_proc_spawn_wax("app", FND_PRIO_IDLE, IMAGE(app), libs, 3) == FND_E_BAD_ARG);
    check(slot_clear(SLOT_APP));

    pid_app = fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), libs, 3);
    pid_app2 = fnd_proc_spawn_wax("app2", FND_PRIO_APP, IMAGE(app2), libs, 3);
    pid_v2 = fnd_proc_spawn_wax("v2app", FND_PRIO_APP, IMAGE(v2app), 0, 0);
    pid_write = fnd_proc_spawn_wax("poke-write", FND_PRIO_APP, IMAGE(poke_write), 0, 0);
    pid_exec = fnd_proc_spawn_wax("poke-exec", FND_PRIO_APP, IMAGE(poke_exec), 0, 0);
    if (fnd_proc_spawn_wax("spin", FND_PRIO_APP, IMAGE(spin), 0, 0) < 0)
        return 2;
    if (pid_app < 0 || pid_app2 < 0 || pid_v2 < 0 || pid_write < 0 || pid_exec < 0)
        return 2;
    /* One process per slot. */
    if (fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), libs, 3) != FND_E_ADDRESS_CONFLICT)
        return 3;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
