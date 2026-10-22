/* Foundation boot image: the kernel starts the bundled modules in the order
 * of doc/spec_Foundation.md §11, init last:
 *
 *   console  slot 1, SERVICE, SVC, MMIO grants for the PCG and PeC input
 *   fs       slot 3, SERVICE, SVC (provisional read-only fs with /bin)
 *   init     slot 0, SERVICE, starts the shell and restarts it
 *
 * Provisional until FNDBOOT exists: the module images are linked into the
 * kernel image (modules_image.h from build.py). */
#include <foundation/kernel.h>

#include "con_pcg.h"
#include "modules_image.h"

#define RW (CPT32_PERM_R | CPT32_PERM_W)

static const struct fnd_grant console_grants[] = {
    {CON_PCG_VDP_START, CON_PCG_VDP_END, RW},
    {CON_PCG_TEXT_START, CON_PCG_TEXT_END, RW},
    {CON_PCG_INPUT_START, CON_PCG_INPUT_END, RW},
};
#define GRANTS (sizeof console_grants / sizeof console_grants[0])

#ifdef FOUNDATION_BOOT_STOP_TICKS
/* tests/shell: stop the machine so src32_testbench can read the screen */
static uint32_t on_tick(uint32_t ticks) { return ticks == FOUNDATION_BOOT_STOP_TICKS ? 42u : 0u; }
#endif

int main(void) {
    if (fnd_proc_spawn_module("console", FND_PRIO_SERVICE, FND_CAP_SVC, img_console, img_console_size,
                              console_grants, GRANTS) < 0 ||
        fnd_proc_spawn_module("fs", FND_PRIO_SERVICE, FND_CAP_SVC, img_fs, img_fs_size, 0, 0) < 0 ||
        fnd_proc_spawn_module("init", FND_PRIO_SERVICE, 0, img_init, img_init_size, 0, 0) < 0)
        return 1;
#ifdef FOUNDATION_BOOT_STOP_TICKS
    fnd_set_tick_hook(on_tick);
#endif
    fnd_kernel_run();
}
