/* Foundation on Extension V (doc/foundation_vm_plan.md) on src32_testbench.
 * The images are those of foundation/tests/wax (build.py there writes
 * images.h):
 * - app is spawned twice at once. Both copies are linked at 0x00800000, which
 *   the slot backend refuses with FND_E_ADDRESS_CONFLICT; here every process
 *   has its own address space, so both run and exit 42;
 * - poke_write_vm / poke_exec_vm fault on a store to RX and a jump into RW,
 *   now as page faults (CAUSE 0x8006_000k, bit 3 = protection). Their blocks
 *   are on pages of their own: a page is as permissive as the most permissive
 *   block on it, so blocks that share a page cannot be told apart;
 * - v2app is a PROTECTED WAX v2 program in C; spin keeps the CPU busy.
 * Rejected spawns, exits and faults must give every frame back: the free
 * memory the kernel reports is the same before and after each round. Ends
 * with R1 = 42 at a HALT, or 0x100 + the number of failed checks. */
#include <foundation/kernel.h>

#include "images.h"
#include "mm.h"

#define IMAGE(name) img_##name, img_##name##_size
#define ROUND_TICKS 40u
#define ROUNDS 3u

#define POKE_WRITE_BASE 0x00600000u
#define POKE_EXEC_DATA 0x00501000u
#define FAULT_WRITE_PROTECTION 0x8006000Au /* a store to a page that is not writable */
#define FAULT_FETCH_PROTECTION 0x8006000Cu /* a fetch from a page that is not executable */

static const struct fnd_image libs[] = {{IMAGE(math_old)}, {IMAGE(math)}, {IMAGE(base)}};
static const struct fnd_image base_only[] = {{IMAGE(base)}};

static int32_t pid_app, pid_twin, pid_v2, pid_write, pid_exec;
static uint32_t failures, rounds, baseline;

static void check(int ok) {
    if (!ok)
        ++failures;
}

static int exited(int32_t pid, int32_t code) {
    struct fnd_proc *p = fnd_proc_get((uint32_t)pid);
    return p && p->state == FND_PROC_EXITED && p->exit_code == code;
}

/* Nothing of the finished processes is left but the spinner. */
static int frames_returned(void) { return mm_vm_free_bytes() == baseline; }

static uint32_t on_tick(uint32_t ticks) {
    if (ticks % ROUND_TICKS)
        return 0;
    check(exited(pid_app, 42));
    check(exited(pid_twin, 42));
    check(frames_returned());
    if (rounds == 0) {
        check(exited(pid_v2, 42));
        struct fnd_proc *w = fnd_proc_get((uint32_t)pid_write);
        struct fnd_proc *x = fnd_proc_get((uint32_t)pid_exec);
        check(exited(pid_write, (int32_t)FAULT_WRITE_PROTECTION) && w->fault_epc == POKE_WRITE_BASE);
        check(exited(pid_exec, (int32_t)FAULT_FETCH_PROTECTION) && x->fault_epc == POKE_EXEC_DATA);
    }
    if (++rounds == ROUNDS)
        return failures == 0u ? 42u : 0x100u + failures;
    /* Spawn and exit again: frames, records and threads are reused. */
    pid_app = fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), libs, 3, 0, 0);
    pid_twin = fnd_proc_spawn_wax("twin", FND_PRIO_APP, IMAGE(app), libs, 3, 0, 0);
    check(pid_app > 0 && pid_twin > 0);
    return 0;
}

int main(void) {
    static const uint8_t garbage[64] = {'W', 'A', 'X', 0, 0, 3};
    if (fnd_proc_spawn_wax("spin", FND_PRIO_APP, IMAGE(spin), 0, 0, 0, 0) < 0)
        return 2;
    if (!mm_vm)
        return 4; /* the CPU has no Extension V, or the kernel was built slot-only */
    baseline = mm_vm_free_bytes();
    /* Rejections are atomic: no frame is kept. */
    check(fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), base_only, 1, 0, 0) == FND_E_DEPENDENCY);
    check(fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app_unprotected), libs, 3, 0, 0) == FND_E_FORMAT);
    check(fnd_proc_spawn_wax("bad", FND_PRIO_APP, garbage, sizeof garbage, 0, 0, 0, 0) == FND_E_FORMAT);
    check(fnd_proc_spawn_wax("app", FND_PRIO_IDLE, IMAGE(app), libs, 3, 0, 0) == FND_E_BAD_ARG);
    check(frames_returned());

    pid_app = fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), libs, 3, 0, 0);
    pid_twin = fnd_proc_spawn_wax("twin", FND_PRIO_APP, IMAGE(app), libs, 3, 0, 0);
    pid_v2 = fnd_proc_spawn_wax("v2app", FND_PRIO_APP, IMAGE(v2app), 0, 0, 0, 0);
    pid_write = fnd_proc_spawn_wax("poke-write", FND_PRIO_APP, IMAGE(poke_write_vm), 0, 0, 0, 0);
    pid_exec = fnd_proc_spawn_wax("poke-exec", FND_PRIO_APP, IMAGE(poke_exec_vm), 0, 0, 0, 0);
    if (pid_app < 0 || pid_twin < 0 || pid_v2 < 0 || pid_write < 0 || pid_exec < 0)
        return 2;
    check(mm_vm_free_bytes() < baseline);
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
