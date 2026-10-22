/* WAX v3 from C (tools/llvm-src32 --wax3/--wal) on the Foundation kernel.
 * images.h comes from build.py: test.app in slots 5 and 6 with test.math
 * and test.base, so each process has its own library instances at other
 * bases. Ends with R1 = 42 at a HALT, or 0x100 + the failed checks. */
#include <foundation/kernel.h>

#include "images.h"

#define IMAGE(name) img_##name, img_##name##_size
#define WAIT_TICKS 60u

static const struct fnd_image libs[] = {{IMAGE(math)}, {IMAGE(base)}};
static int32_t pid_app, pid_app2;

static uint32_t exited(int32_t pid) {
    struct fnd_proc *p = fnd_proc_get((uint32_t)pid);
    return p && p->state == FND_PROC_EXITED && p->exit_code == 42;
}

static uint32_t on_tick(uint32_t ticks) {
    if (ticks < WAIT_TICKS)
        return 0;
    uint32_t failures = !exited(pid_app) + !exited(pid_app2);
    return failures ? 0x100u + failures : 42u;
}

int main(void) {
    pid_app = fnd_proc_spawn_wax("app", FND_PRIO_APP, IMAGE(app), libs, 2, 0, 0);
    pid_app2 = fnd_proc_spawn_wax("app2", FND_PRIO_APP, IMAGE(app2), libs, 2, 0, 0);
    if (pid_app < 0 || pid_app2 < 0)
        return 2;
    if (fnd_proc_spawn_wax("spin", FND_PRIO_APP, IMAGE(spin), 0, 0, 0, 0) < 0)
        return 3;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
