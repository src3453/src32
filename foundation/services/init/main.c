/* init (doc/spec_Foundation_API.md §12): opens the console, starts the
 * shell with the console as STDIN, STDOUT, STDERR and CONSOLE and a new fs
 * session as FS, and starts it again whenever it exits.
 *
 * Provisional: the shell image is linked into init (shell_image.h from
 * boot/build.py) until the fs on the boot disk holds /bin/shell.wax; then
 * init uses fnd_proc_exec on that path instead. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "fnd_internal.h"
#include "shell_image.h"

#define RETRY_TICKS 10u
#define RESPAWN_TICKS 100u

static const char *const shell_argv[] = {"/bin/shell.wax"};

/* Waits for a service that the kernel starts before init. */
static fnd_handle lookup(const char *name) {
    for (;;) {
        int32_t h = fnd_sys_svc_lookup(name);
        if (h > 0)
            return (fnd_handle)h;
        fnd_sys_sleep(RETRY_TICKS);
    }
}

int main(void) {
    fnd_init();
    fnd_handle dev = lookup("dev.con");
    uint32_t args[4] = {FND_OPEN_READ | FND_OPEN_WRITE, 0, 0, 0};
    fnd_msg rep;
    if (fnd_call(dev, FND_DEV_OPEN, args, 0, 0, 0, 0, &rep) < 0)
        fnd_exit(FND_E_BAD_HANDLE);
    fnd_handle con = rep.a3;
    fnd_sys_handle_close(dev);
    while (!fnd_fs())
        fnd_sys_sleep(RETRY_TICKS);
    fnd_exec_t ex = {1u, shell_argv, "/", con, con, con, 0};
    for (;;) {
        fnd_handle shell;
        int32_t code;
        if (fnd__spawn_image(img_shell, img_shell_size, shell_argv[0], &ex, con, &shell) == FND_OK)
            fnd_proc_wait(shell, &code);
        fnd_sys_sleep(RESPAWN_TICKS);
    }
}
