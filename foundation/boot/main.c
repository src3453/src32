/* Foundation kernel entry: spawns the FNDBOOT modules (doc/spec_Foundation.md
 * §11) in table order, init last. build.py packs
 *
 *   console  SERVICE, SVC, MMIO grants for the PCG and PeC input
 *   disk     SERVICE, DISK | SVC (blk_io for fs)
 *   fs       SERVICE, SVC (FS2 on the boot drive, /dd, /dev, /tmp)
 *   init     SERVICE, starts /bin/shell.wax and restarts it
 *
 * Disk boot: stage1 enters with R1 = boot drive and R2 = the FNDBOOT image
 * staged at 0x00200000. FOUNDATION_BOOT_EMBEDDED builds a WAX that the host
 * loads directly; build.py then adds a module-only FNDBOOT as a block at
 * 0x00200000, where stage1 would have staged it, so the kernel keeps the §5.1
 * layout (linking the modules into the kernel image overran the kernel stack
 * once they passed 900 KiB). R1/R2 are ignored and the boot drive is 0.
 *
 * The modules are PROTECTED WAX v2 with libfnd linked in, so they get no
 * WAL list. Once they are spawned the staged FNDBOOT is no longer needed:
 * proc_spawn reuses the area for the images it stages (§9.2). */
#include <foundation/kernel.h>

/* Where stage1 stages FNDBOOT (§11); the embedded WAX carries it there too. */
#define FNDBOOT_STAGING 0x00200000u
#ifndef FND_TANG_ROOT_DRIVE
#define FND_TANG_ROOT_DRIVE 1u
#endif

#ifdef FOUNDATION_BOOT_STOP_TICKS
/* tests: stop the machine so src32_testbench can read the screen */
static uint32_t on_tick(uint32_t ticks) { return ticks == FOUNDATION_BOOT_STOP_TICKS ? 42u : 0u; }
#endif

int main(uint32_t drive, uint32_t fndboot) {
    const void *boot_image;
#if defined(FOUNDATION_BOOT_EMBEDDED) || defined(FND_PLATFORM_TANG)
    (void)drive;
    (void)fndboot;
#ifdef FND_PLATFORM_TANG
    fnd_set_boot_drive(FND_TANG_ROOT_DRIVE); /* MicroSD dd0, SDRAM disk dd1. */
#endif
    boot_image = (const void *)(uintptr_t)FNDBOOT_STAGING;
#else
    fnd_set_boot_drive(drive);
    boot_image = (const void *)(uintptr_t)fndboot;
#endif
    int32_t count = fnd_boot_check(boot_image);
    if (count < 0)
        return count;
    for (uint32_t i = 0; i < (uint32_t)count; ++i) {
        int32_t pid = fnd_boot_spawn(boot_image, i, 0, 0);
        if (pid < 0)
            return pid;
    }
#ifdef FOUNDATION_BOOT_STOP_TICKS
    fnd_set_tick_hook(on_tick);
#endif
    fnd_kernel_run();
}
