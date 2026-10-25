/* プロセス・時間・シグナル・資源制限（doc/spec_pkcl.md §6.3〜§6.5, §7）。
 * Foundation には fork も signal もないので、必要な範囲で縮退させる。 */
#define _GNU_SOURCE 1
#include <errno.h>
#include <signal.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/time.h>
#include <sys/utsname.h>
#include <time.h>
#include <unistd.h>

#include <foundation/fnd.h>
#include <foundation/sys.h>
#include <pkcl/pkcl.h>

extern char **environ;

/* ---- 実行 ----
 * exec 族は「子を起動して終了を待ち、その終了コードで自分も終わる」で置き換える。
 * 呼び出し元から見ると戻らない点が exec と同じで、違うのは pid とメモリが残ることだけ。 */
int execve(const char *path, char *const argv[], char *const envp[]) {
    pkcl_init();
    uint32_t argc = 0;
    while (argv && argv[argc])
        ++argc;
    fnd_exec_t ex = {0};
    ex.argc = argc;
    ex.argv = (const char *const *)argv;
    ex.envp = (const char *const *)envp;
    ex.flags = FND_EXEC_KEEP_ARGV0; /* BusyBox picks the applet from argv[0] */
    ex.std_in = pkcl_handle(0);
    ex.std_out = pkcl_handle(1);
    ex.std_err = pkcl_handle(2);
    fnd_handle proc;
    if (pkcl_fail(fnd_proc_exec(path, &ex, &proc)))
        return -1;
    int32_t code = 0;
    fnd_proc_wait(proc, &code);
    fnd_close(proc);
    exit(code);
}

int execv(const char *path, char *const argv[]) { return execve(path, argv, environ); }

int execvp(const char *file, char *const argv[]) {
    if (strchr(file, '/'))
        return execv(file, argv);
    const char *path = getenv("PATH");
    if (!path)
        path = "/bin";
    char buf[PKCL_PATH_MAX + 1];
    while (*path) {
        const char *end = strchr(path, ':');
        size_t length = end ? (size_t)(end - path) : strlen(path);
        if (length + 1 + strlen(file) < sizeof buf) {
            memcpy(buf, path, length);
            buf[length] = '/';
            strcpy(buf + length + 1, file);
            fnd_stat_t st;
            if (fnd_path_stat(buf, &st) == FND_OK && st.type == FND_TYPE_FILE)
                return execv(buf, argv);
        }
        path = end ? end + 1 : path + length;
    }
    errno = ENOENT;
    return -1;
}

pid_t setsid(void) { return getpid(); }
int setuid(uid_t uid) { return uid == 0 ? 0 : (errno = EPERM, -1); }
int setgid(gid_t gid) { return gid == 0 ? 0 : (errno = EPERM, -1); }
int seteuid(uid_t uid) { return setuid(uid); }
int setegid(gid_t gid) { return setgid(gid); }
int getgroups(int size, gid_t *list) {
    (void)size, (void)list;
    return 0;
}
int initgroups(const char *user, gid_t group) {
    (void)user, (void)group;
    return 0;
}

/* ---- 時間 ---- */
int gettimeofday(struct timeval *tv, void *tz) {
    extern int _gettimeofday(struct timeval *, void *);
    return _gettimeofday(tv, tz);
}
int settimeofday(const struct timeval *tv, const struct timezone *tz) {
    (void)tv, (void)tz;
    errno = EPERM;
    return -1;
}

static void sleep_us(unsigned long long us) {
    uint32_t hz = (uint32_t)fnd_sys_time_get(2);
    if (!hz)
        hz = 100;
    unsigned long long ticks = (us * hz + 999999u) / 1000000u;
    if (ticks > 0x7FFFFFFFu)
        ticks = 0x7FFFFFFFu;
    fnd_sys_sleep((uint32_t)ticks);
}
unsigned sleep(unsigned seconds) {
    sleep_us((unsigned long long)seconds * 1000000u);
    return 0;
}
int usleep(useconds_t us) {
    sleep_us(us);
    return 0;
}
int nanosleep(const struct timespec *req, struct timespec *rem) {
    if (req->tv_sec < 0 || req->tv_nsec < 0 || req->tv_nsec >= 1000000000L) {
        errno = EINVAL;
        return -1;
    }
    sleep_us((unsigned long long)req->tv_sec * 1000000u + (unsigned long long)req->tv_nsec / 1000u);
    if (rem)
        rem->tv_sec = rem->tv_nsec = 0;
    return 0;
}
unsigned alarm(unsigned seconds) {
    (void)seconds; /* SIGALRM を届ける手段がない（§7 G-signal） */
    return 0;
}

/* ---- シグナル: ハンドラは登録できるが、配送はされない ---- */
int sigaction(int sig, const struct sigaction *act, struct sigaction *old) {
    if (sig <= 0 || sig >= NSIG) {
        errno = EINVAL;
        return -1;
    }
    if (old)
        memset(old, 0, sizeof *old);
    (void)act;
    return 0;
}
int sigprocmask(int how, const sigset_t *set, sigset_t *old) {
    (void)how, (void)set;
    if (old)
        *old = 0;
    return 0;
}
int sigisemptyset(const sigset_t *set) { return *set == 0; }
int sigsuspend(const sigset_t *mask) {
    (void)mask;
    errno = ENOSYS;
    return -1;
}

/* ---- 資源・システム情報 ---- */
int getrlimit(int resource, struct rlimit *rlim) {
    (void)resource;
    rlim->rlim_cur = rlim->rlim_max = RLIM_INFINITY;
    return 0;
}
int setrlimit(int resource, const struct rlimit *rlim) {
    (void)resource, (void)rlim;
    return 0;
}
int getpagesize(void) { return 4096; }
long sysconf(int name) {
    switch (name) {
    case _SC_PAGESIZE: return 4096;
    case _SC_CLK_TCK: return 100;
    case _SC_OPEN_MAX: return PKCL_OPEN_MAX;
    case _SC_ARG_MAX: return 32768;
    case _SC_NPROCESSORS_ONLN:
    case _SC_NPROCESSORS_CONF: return 1;
    default: errno = EINVAL; return -1;
    }
}
int uname(struct utsname *name) {
    memset(name, 0, sizeof *name);
    strcpy(name->sysname, "Foundation");
    strcpy(name->nodename, "cpt32");
    strcpy(name->release, "0");
    strcpy(name->version, "0");
    strcpy(name->machine, "src32");
    return 0;
}
