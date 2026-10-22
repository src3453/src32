#include <foundation/kernel.h>
#include "images.h"

static const struct fnd_image libraries[] = {{img_library, img_library_size}};
static int32_t first, second;

static uint32_t on_tick(uint32_t ticks) {
    struct fnd_proc *a = fnd_proc_get((uint32_t)first);
    struct fnd_proc *b = fnd_proc_get((uint32_t)second);
    if (a && b && a->state == FND_PROC_EXITED && b->state == FND_PROC_EXITED)
        return a->exit_code != 42 ? 0x10000u | (uint32_t)a->exit_code :
               b->exit_code != 42 ? 0x20000u | (uint32_t)b->exit_code : 42;
    /* SDK table setup is CPU work; bound the complete two-process run. */
    return ticks > 15000 ? 0x101u : 0;
}

int main(void) {
    first = fnd_proc_spawn_wax("lzma-test", FND_PRIO_APP, img_test, img_test_size, libraries, 1, 0, 0);
    second = fnd_proc_spawn_wax("lzma-demo", FND_PRIO_APP, img_demo, img_demo_size, libraries, 1, 0, 0);
    if (first < 0 || second < 0)
        return 2;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
