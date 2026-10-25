/* fork・wait・kill・pid（doc/spec_pkcl.md §6.4、§7 G1）。
 * Foundation の proc_fork（Foundation §9.6）の上に作る。VM のカーネルだけが fork でき、slot では
 * FND_E_UNSUPPORTED なので ENOSYS になる。子は PROCESS ハンドルで待つ。ハンドルの番号は親子で同じだが、
 * 子に渡されるのは親が持つ他のハンドルで、親の子のハンドルは渡されない。 */
#define _GNU_SOURCE 1
#include <errno.h>
#include <signal.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include <foundation/fnd.h>
#include <foundation/sys.h>
#include <pkcl/pkcl.h>

#define CHILD_MAX 32

static struct {
    pid_t pid;
    fnd_handle handle;
} children[CHILD_MAX];
static pid_t own_pid, own_parent;

static void load_self(void) {
    uint32_t info[2];
    if (own_pid)
        return;
    if (fnd_sys_info(4, info, sizeof info) == (int32_t)sizeof info) {
        own_pid = (pid_t)info[0];
        own_parent = (pid_t)info[1];
    } else {
        own_pid = 1;
    }
}

pid_t getpid(void) {
    load_self();
    return own_pid;
}
pid_t getppid(void) {
    load_self();
    return own_parent;
}

pid_t fork(void) {
    pkcl_init();
    int slot = -1;
    for (int i = 0; i < CHILD_MAX; ++i) {
        if (!children[i].pid) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        errno = EAGAIN;
        return -1;
    }
    int32_t result = fnd_proc_fork();
    if (result < 0) {
        errno = pkcl_errno(result);
        return -1;
    }
    if (result == 0) { /* 子: 親の子は自分の子ではない */
        memset(children, 0, sizeof children);
        own_pid = 0;
        return 0;
    }
    int32_t pid = fnd_sys_info(5, result, 0);
    if (pid <= 0) {
        fnd_proc_kill((fnd_handle)result);
        fnd_sys_handle_close((fnd_handle)result);
        errno = pkcl_errno(pid);
        return -1;
    }
    children[slot].pid = (pid_t)pid;
    children[slot].handle = (fnd_handle)result;
    return (pid_t)pid;
}

/* vfork は fork と同じ。親は止まらないが、BusyBox が子で行うのは exec か _exit だけ。 */
pid_t vfork(void) { return fork(); }

/* exit_code を wait の status に直す。終了コードは 0〜255、強制終了は SIGKILL、それ以外（フォールト）は SIGSEGV。 */
static int wait_status(int32_t code) {
    if (code >= 0 && code < 256)
        return code << 8;
    return code == FND_E_KILLED ? SIGKILL : SIGSEGV;
}

pid_t waitpid(pid_t pid, int *status, int options) {
    pkcl_init();
    for (;;) {
        int candidates = 0, only = -1;
        for (int i = 0; i < CHILD_MAX; ++i) {
            if (children[i].pid && (pid <= 0 || children[i].pid == pid)) {
                only = candidates++ ? -1 : i;
            }
        }
        for (int i = 0; i < CHILD_MAX; ++i) {
            if (!children[i].pid || (pid > 0 && children[i].pid != pid))
                continue;
            int32_t code = 0;
            /* 待つ相手が 1 つなら、カーネルの中で待つ（polling より速い） */
            int32_t r = fnd_sys_wait(children[i].handle, i == only && !(options & WNOHANG) ? FND_FOREVER : 0, &code);
            if (r == FND_E_WOULD_BLOCK)
                continue;
            fnd_sys_handle_close(children[i].handle);
            pid_t done = children[i].pid;
            children[i].pid = 0;
            if (r < 0) {
                errno = pkcl_errno(r);
                return -1;
            }
            if (status)
                *status = wait_status(code);
            return done;
        }
        if (!candidates) {
            errno = ECHILD;
            return -1;
        }
        if (options & WNOHANG)
            return 0;
        fnd_sys_sleep(1); /* 複数の子のどれかを待つカーネルの手段がないので、短い間隔で見る */
    }
}

pid_t wait(int *status) { return waitpid(-1, status, 0); }

int kill(pid_t pid, int sig) {
    pkcl_init();
    if (sig < 0 || sig >= NSIG) {
        errno = EINVAL;
        return -1;
    }
    if (pid == getpid())
        return sig == 0 ? 0 : (errno = ENOSYS, -1); /* 自分宛のシグナルの配送はない（§6.5） */
    for (int i = 0; i < CHILD_MAX; ++i) {
        if (children[i].pid == pid) {
            if (sig != 0)
                fnd_proc_kill(children[i].handle);
            return 0;
        }
    }
    errno = ESRCH;
    return -1;
}
