/* Foundation kernel entry: spawns the FNDBOOT modules (doc/spec_Foundation.md
 * §11) in table order, init last. build.py packs:
 *
 *   console  slot 1, SERVICE, SVC, MMIO grants for the PCG and PeC input
 *   disk     slot 2, SERVICE, DISK | SVC (blk_io for fs)
 *   fs       slot 3, SERVICE, SVC (FS2 on the boot drive, /dd, /dev, /tmp)
 *   init     slot 0, SERVICE, starts /bin/shell.wax and restarts it
 *
 * Disk boot: stage1 enters with R1 = boot drive and R2 = the FNDBOOT image
 * staged at 0x00200000. FOUNDATION_BOOT_EMBEDDED builds a WAX that the host
 * loads directly; build.py then links a module-only FNDBOOT into the image
 * (fndboot_image.h), R1/R2 are ignored and the boot drive is 0.
 *
 * Provisional: the staging area is not released after the modules are
 * spawned; nothing allocates that memory yet. */
#include <foundation/kernel.h>

#ifdef FOUNDATION_BOOT_EMBEDDED
#include "fndboot_image.h"
#endif

#ifdef FOUNDATION_BOOT_STOP_TICKS
/* tests: stop the machine so src32_testbench can read the screen */
static uint32_t on_tick(uint32_t ticks) { return ticks == FOUNDATION_BOOT_STOP_TICKS ? 42u : 0u; }
#endif

int main(uint32_t drive, uint32_t fndboot) {
#ifdef FOUNDATION_BOOT_EMBEDDED
    (void)drive;
    (void)fndboot;
    const void *boot_image = img_fndboot;
#else
    fnd_set_boot_drive(drive);
    const void *boot_image = (const void *)(uintptr_t)fndboot;
#endif
    int32_t count = fnd_boot_check(boot_image);
    if (count < 0)
        return count;
    for (uint32_t i = 0; i < (uint32_t)count; ++i) {
        int32_t pid = fnd_boot_spawn(boot_image, i);
        if (pid < 0)
            return pid;
    }
#ifdef FOUNDATION_BOOT_STOP_TICKS
    fnd_set_tick_hook(on_tick);
#endif
    fnd_kernel_run();
}
