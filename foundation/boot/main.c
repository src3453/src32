/* Foundation boot image: the kernel starts the shell (/bin/shell.wax,
 * doc/spec_Foundation_shell.md) in slot 4 at APP priority.
 *
 * Provisional until init, the console service and FNDBOOT exist: the shell
 * image is linked into the kernel image (shell_image.h from build.py), and
 * the shell drives the console itself (foundation/lib/fnd/con_pcg.c), so it
 * gets MMIO grants for the PCG and the PeC input. When the shell exits, the
 * kernel starts it again, as init would. */
#include <foundation/kernel.h>

#include "con_pcg.h"
#include "shell_image.h"

#define RESPAWN_TICKS 100u
#define RW (CPT32_PERM_R | CPT32_PERM_W)

static const struct fnd_grant console_grants[] = {
    {CON_PCG_VDP_START, CON_PCG_VDP_END, RW},
    {CON_PCG_TEXT_START, CON_PCG_TEXT_END, RW},
    {CON_PCG_INPUT_START, CON_PCG_INPUT_END, RW},
};
#define GRANTS (sizeof console_grants / sizeof console_grants[0])

static int32_t shell_pid;

static int32_t spawn_shell(void) {
    return fnd_proc_spawn_wax("shell", FND_PRIO_APP, img_shell, img_shell_size, 0, 0, console_grants,
                              GRANTS);
}

static uint32_t on_tick(uint32_t ticks) {
    if (ticks % RESPAWN_TICKS == 0) {
        struct fnd_proc *proc = fnd_proc_get((uint32_t)shell_pid);
        if (!proc || proc->state != FND_PROC_LIVE)
            shell_pid = spawn_shell();
    }
#ifdef FOUNDATION_BOOT_STOP_TICKS
    /* tests/shell: stop the machine so src32_testbench can read the screen */
    if (ticks == FOUNDATION_BOOT_STOP_TICKS)
        return 42u;
#endif
    return 0;
}

int main(void) {
    shell_pid = spawn_shell();
    if (shell_pid < 0)
        return 1;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
