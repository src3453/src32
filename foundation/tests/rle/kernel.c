#include <foundation/kernel.h>
#include "images.h"

static const struct fnd_image libraries[] = {{img_rle, img_rle_size}, {img_lzma, img_lzma_size}};
static int32_t pid;

static uint32_t on_tick(uint32_t ticks) {
    struct fnd_proc *p = fnd_proc_get((uint32_t)pid);
    if (p && p->state == FND_PROC_EXITED)
        return p->exit_code == 42 ? 42 : 0x10000u | (uint32_t)p->exit_code;
    return ticks > 15000 ? 0x101u : 0;
}

int main(void) {
    pid = fnd_proc_spawn_wax("rle-test", FND_PRIO_APP, img_test, img_test_size, libraries, 2, 0, 0);
    if (pid < 0)
        return 2;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
