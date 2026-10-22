/* init (doc/spec_Foundation_API.md §12): opens the console, starts
 * /bin/shell.wax from the boot disk with the console as STDIN, STDOUT,
 * STDERR and CONSOLE and a new fs session as FS, and starts it again
 * whenever it exits (or retries while it cannot be started). */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "fnd_internal.h"


#define RETRY_TICKS 10u
#define RESPAWN_TICKS 100u

static const char *const shell_argv[] = {"/bin/shell.wax"};

/* fnd_proc_exec (§11.6) with con as the CONSOLE role, which init itself
 * does not have. */
static fnd_status exec_shell(const fnd_exec_t *ex, fnd_handle con, fnd_handle *proc) {
    fnd_handle file;
    fnd_status status = fnd_file_open(shell_argv[0], FND_OPEN_EXEC, 0, &file);
    if (status != FND_OK)
        return status;
    fnd_stat_t st;
    status = fnd_stream_stat(file, &st);
    uint8_t *image = 0;
    uint32_t got = 0;
    if (status == FND_OK && (st.size_hi || st.size_lo > FND_EXEC_IMAGE_MAX))
        status = FND_E_LIMIT;
    if (status == FND_OK && !(image = fnd_mem_alloc(st.size_lo ? st.size_lo : 1u)))
        status = FND_E_NO_MEMORY;
    if (status == FND_OK)
        status = fnd_read(file, image, st.size_lo, &got);
    fnd_close(file);
    if (status == FND_OK && got != st.size_lo)
        status = FND_E_IO;
    if (status == FND_OK)
        status = fnd__spawn_image(image, got, shell_argv[0], ex, con, proc);
    fnd_mem_free(image);
    return status;
}

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
        if (exec_shell(&ex, con, &shell) == FND_OK)
            fnd_proc_wait(shell, &code);
        fnd_sys_sleep(RESPAWN_TICKS);
    }
}
