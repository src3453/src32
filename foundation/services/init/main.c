/* init (doc/spec_Foundation_API.md §12): opens the console, starts
 * /bin/shell.wax with the console as STDIN, STDOUT, STDERR and CONSOLE and
 * a new fs session as FS, and starts it again whenever it exits. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "fnd_internal.h"

#define RETRY_TICKS 1u    /* 10 ms at 100 Hz */
#define RESPAWN_TICKS 10u /* 100 ms at 100 Hz */

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

static uint32_t str_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

/* "init: <what>: <error> (<code>)" on the console. */
static void say(fnd_handle con, const char *what, fnd_status status) {
    char num[12];
    uint32_t n = sizeof num, v = status < 0 ? 0u - (uint32_t)status : (uint32_t)status;
    num[--n] = 0;
    do
        num[--n] = (char)('0' + v % 10u);
    while (v /= 10u);
    if (status < 0)
        num[--n] = '-';
    const char *parts[] = {"init: ", what, ": ", fnd_strerror(status), " (", num + n, ")\n"};
    for (uint32_t i = 0; i < sizeof parts / sizeof parts[0]; ++i)
        fnd_write(con, parts[i], str_len(parts[i]), 0);
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
        fnd_status status = fnd__exec(shell_argv[0], &ex, con, &shell);
        if (status == FND_OK)
            fnd_proc_wait(shell, &code);
        else
            say(con, shell_argv[0], status);
        fnd_sys_sleep(RESPAWN_TICKS);
    }
}
