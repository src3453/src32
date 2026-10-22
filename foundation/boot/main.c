/* Foundation kernel entry: spawns the FNDBOOT modules (doc/spec_Foundation.md
 * §11) in table order. Today the only module is the shell (/bin/shell.wax,
 * doc/spec_Foundation_shell.md) in slot 4 at APP priority.
 *
 * Disk boot: stage1 enters with R1 = boot drive and R2 = the FNDBOOT image
 * staged at 0x00200000. FOUNDATION_BOOT_EMBEDDED builds a WAX that the host
 * loads directly; build.py then links a module-only FNDBOOT into the image
 * (fndboot_image.h) and R1/R2 are ignored.
 *
 * Provisional until init and the console service exist: the shell drives the
 * console itself (foundation/lib/fnd/con_pcg.c) through the MMIO grants in
 * its module entry, and when the last module (the shell standing in for init)
 * exits, the kernel spawns it again from the image, as init would restart
 * it. So the staging area stays in use instead of being released. */
#include <foundation/kernel.h>

#ifdef FOUNDATION_BOOT_EMBEDDED
#include "fndboot_image.h"
#endif

#define RESPAWN_TICKS 100u

static const void *boot_image;
static uint32_t last_module;
static int32_t last_pid;

static uint32_t on_tick(uint32_t ticks) {
    if (ticks % RESPAWN_TICKS == 0) {
        struct fnd_proc *proc = fnd_proc_get((uint32_t)last_pid);
        if (!proc || proc->state != FND_PROC_LIVE)
            last_pid = fnd_boot_spawn(boot_image, last_module);
    }
#ifdef FOUNDATION_BOOT_STOP_TICKS
    /* tests: stop the machine so src32_testbench can read the screen */
    if (ticks == FOUNDATION_BOOT_STOP_TICKS)
        return 42u;
#endif
    return 0;
}

int main(uint32_t drive, uint32_t fndboot) {
#ifdef FOUNDATION_BOOT_EMBEDDED
    (void)drive;
    (void)fndboot;
    boot_image = img_fndboot;
#else
    fnd_set_boot_drive(drive);
    boot_image = (const void *)(uintptr_t)fndboot;
#endif
    int32_t count = fnd_boot_check(boot_image);
    if (count < 0)
        return count;
    for (uint32_t i = 0; i < (uint32_t)count; ++i) {
        last_pid = fnd_boot_spawn(boot_image, i);
        if (last_pid < 0)
            return last_pid;
    }
    last_module = (uint32_t)count - 1u;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
