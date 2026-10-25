/* ファイル記述子表と、ストリームに対する POSIX 呼び出し（doc/spec_pkcl.md §6.1）。
 * 1 つの記述子は 1 つの Foundation ハンドル（開いたファイル 1 つ = 1 CHANNEL）を指し、
 * 読み書き位置はサービス側が持つ。dup はハンドルの複製なので、位置を共有する（POSIX どおり）。 */
#define _GNU_SOURCE 1 /* libc のヘッダに O_CLOEXEC などを出させる */
#include <errno.h>
#include <fcntl.h>
#include <foundation/api.h>
#include <foundation/sys.h>
#include <pkcl/pkcl.h>
#include <stdarg.h>
#include <sys/stat.h>
#include <unistd.h>

struct fd_entry {
    fnd_handle handle; /* 0 = 未使用 */
    uint32_t status;   /* F_GETFL: アクセスモードと O_APPEND、O_NONBLOCK */
    uint32_t cloexec;  /* FD_CLOEXEC。exec 経路ができるまで記録するだけ */
};

static struct fd_entry fds[PKCL_OPEN_MAX];
static int initialized;
static mode_t current_umask = 022;

void pkcl_init(void) {
    if (initialized)
        return;
    initialized = 1;
    fnd_handle std[3] = {fnd_std_in(), fnd_std_out(), fnd_std_err()};
    static const uint32_t access[3] = {O_RDONLY, O_WRONLY, O_WRONLY};
    for (int i = 0; i < 3; ++i) {
        fnd_handle h = std[i];
        if (!h)
            continue;
        /* fnd_std_err() は役割がなければ STDOUT と同じハンドルを返す。
         * 同じハンドルを 2 つの記述子で持つと、片方の close でもう片方が壊れる。 */
        for (int j = 0; j < i; ++j) {
            if (fds[j].handle == h) {
                int32_t copy = fnd_sys_handle_dup(h);
                h = copy > 0 ? (fnd_handle)copy : 0;
                break;
            }
        }
        fds[i].handle = h;
        fds[i].status = access[i];
    }
}

fnd_handle pkcl_handle(int fd) {
    pkcl_init();
    if (fd < 0 || fd >= PKCL_OPEN_MAX || !fds[fd].handle) {
        errno = EBADF;
        return 0;
    }
    return fds[fd].handle;
}

/* from 以上で最小の空き記述子。なければ -1（EMFILE）。 */
static int fd_alloc(int from) {
    for (int fd = from; fd < PKCL_OPEN_MAX; ++fd) {
        if (!fds[fd].handle)
            return fd;
    }
    errno = EMFILE;
    return -1;
}

int pkcl_fd_install(fnd_handle h, uint32_t status) {
    pkcl_init();
    int fd = fd_alloc(0);
    if (fd < 0)
        return -1;
    fds[fd] = (struct fd_entry){h, status, 0};
    return fd;
}

uint32_t pkcl_mode_to_fnd(uint32_t m) {
    /* FS2: bit 0〜2 が other の X/W/R、bit 3〜5 が owner の X/W/R。 */
    return ((m >> 6) & 7u) << 3 | (m & 7u);
}

uint32_t pkcl_mode_from_fnd(uint32_t type, uint32_t f) {
    uint32_t owner = (f >> 3) & 7u, other = f & 7u;
    uint32_t kind = type == FND_TYPE_DIR ? S_IFDIR : type == FND_TYPE_FILE ? S_IFREG
                    : type == FND_TYPE_PIPE ? S_IFIFO : S_IFCHR;
    /* group は持たないので other と同じに見せる。 */
    return kind | owner << 6 | other << 3 | other;
}

int open(const char *path, int flags, ...) {
    pkcl_init();
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = va_arg(ap, mode_t);
        va_end(ap);
    }
    uint32_t access = (uint32_t)flags & O_ACCMODE, fl = 0;
    if (access == O_RDONLY)
        fl = FND_OPEN_READ;
    else if (access == O_WRONLY)
        fl = FND_OPEN_WRITE;
    else if (access == O_RDWR)
        fl = FND_OPEN_READ | FND_OPEN_WRITE;
    else {
        errno = EINVAL;
        return -1;
    }
    if (flags & O_CREAT)
        fl |= FND_OPEN_CREATE;
    if ((flags & O_EXCL) && (flags & O_CREAT))
        fl |= FND_OPEN_EXCL;
    /* TRUNC は WRITE を要する（API §6.4）。O_RDONLY|O_TRUNC は POSIX では未規定なので無視する。 */
    if ((flags & O_TRUNC) && (fl & FND_OPEN_WRITE))
        fl |= FND_OPEN_TRUNC;
    if ((flags & O_APPEND) && (fl & FND_OPEN_WRITE))
        fl |= FND_OPEN_APPEND;
    int fd = fd_alloc(0);
    if (fd < 0)
        return -1;
    uint32_t fnd_mode = FND_MODE_DEFAULT;
    if (flags & O_CREAT)
        fnd_mode = pkcl_mode_to_fnd(mode & ~current_umask);
    fnd_handle h;
    if (pkcl_fail(fnd_file_open(path, fl, fnd_mode, &h)))
        return -1;
    fds[fd].handle = h;
    fds[fd].status = (uint32_t)flags & (O_ACCMODE | O_APPEND | O_NONBLOCK);
    fds[fd].cloexec = (flags & O_CLOEXEC) ? FD_CLOEXEC : 0;
    return fd;
}

int creat(const char *path, mode_t mode) { return open(path, O_WRONLY | O_CREAT | O_TRUNC, mode); }

int close(int fd) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    fds[fd].handle = 0;
    return pkcl_fail(fnd_close(h));
}

ssize_t read(int fd, void *buf, size_t len) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    if ((fds[fd].status & O_ACCMODE) == O_WRONLY) {
        errno = EBADF;
        return -1;
    }
    if (len > 0x7FFFFFFFu)
        len = 0x7FFFFFFFu;
    /* fnd_read は満杯の塊のあと次の塊を待つ。pipe は届いた分だけ返す（POSIX の短い read）。 */
    if ((fds[fd].status & PKCL_FD_PIPE) && len > FND_IPC_MAX_DATA)
        len = FND_IPC_MAX_DATA;
    uint32_t got = 0;
    if (pkcl_fail(fnd_read(h, buf, (uint32_t)len, &got)))
        return -1;
    return (ssize_t)got;
}

ssize_t write(int fd, const void *buf, size_t len) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    if ((fds[fd].status & O_ACCMODE) == O_RDONLY) {
        errno = EBADF;
        return -1;
    }
    if (len > 0x7FFFFFFFu)
        len = 0x7FFFFFFFu;
    uint32_t put = 0;
    fnd_status status = fnd_write(h, buf, (uint32_t)len, &put);
    /* 一部だけ書けていれば、POSIX どおりその byte 数を成功として返す。 */
    if (status < 0 && put == 0)
        return pkcl_fail(status);
    return (ssize_t)put;
}

off_t lseek(int fd, off_t offset, int whence) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    if (whence < SEEK_SET || whence > SEEK_END) {
        errno = EINVAL;
        return -1;
    }
    int64_t delta = offset;
    uint32_t args[4] = {(uint32_t)whence, (uint32_t)((uint64_t)delta >> 32), (uint32_t)delta, 0};
    fnd_msg rep;
    fnd_status status = fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, &rep);
    if (status < 0) {
        /* サービスは UNSUPPORTED を返す（コンソール、シーク不可のデバイス）。POSIX では ESPIPE。 */
        errno = status == FND_E_UNSUPPORTED ? ESPIPE : pkcl_errno(status);
        return -1;
    }
    if (rep.a0 != 0 || rep.a1 > 0x7FFFFFFFu) {
        errno = EOVERFLOW;
        return -1;
    }
    return (off_t)rep.a1;
}

int dup(int fd) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    int nfd = fd_alloc(0);
    if (nfd < 0)
        return -1;
    int32_t copy = fnd_sys_handle_dup(h);
    if (copy <= 0)
        return pkcl_fail(copy);
    fds[nfd] = (struct fd_entry){(fnd_handle)copy, fds[fd].status, 0};
    return nfd;
}

int dup2(int fd, int newfd) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    if (newfd < 0 || newfd >= PKCL_OPEN_MAX) {
        errno = EBADF;
        return -1;
    }
    if (fd == newfd)
        return newfd;
    int32_t copy = fnd_sys_handle_dup(h);
    if (copy <= 0)
        return pkcl_fail(copy);
    if (fds[newfd].handle)
        fnd_close(fds[newfd].handle);
    fds[newfd] = (struct fd_entry){(fnd_handle)copy, fds[fd].status, 0};
    return newfd;
}

int fcntl(int fd, int cmd, ...) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    va_list ap;
    va_start(ap, cmd);
    int arg = va_arg(ap, int);
    va_end(ap);
    switch (cmd) {
    case F_DUPFD: {
        int nfd = fd_alloc(arg < 0 ? 0 : arg);
        if (nfd < 0)
            return -1;
        int32_t copy = fnd_sys_handle_dup(h);
        if (copy <= 0)
            return pkcl_fail(copy);
        fds[nfd] = (struct fd_entry){(fnd_handle)copy, fds[fd].status, 0};
        return nfd;
    }
    case F_GETFD:
        return (int)fds[fd].cloexec;
    case F_SETFD:
        fds[fd].cloexec = (uint32_t)arg & FD_CLOEXEC;
        return 0;
    case F_GETFL:
        return (int)(fds[fd].status & ~PKCL_FD_PIPE);
    case F_SETFL:
        /* アクセスモードは変えられない。O_APPEND は開く時のストリームに固定なので、
         * 後から変えても効かない（記録も更新しない）。O_NONBLOCK は記録だけする。 */
        fds[fd].status = (fds[fd].status & ~(uint32_t)O_NONBLOCK) | ((uint32_t)arg & O_NONBLOCK);
        return 0;
    default:
        errno = EINVAL;
        return -1;
    }
}

int isatty(int fd) {
    fnd_handle h = pkcl_handle(fd);
    fnd_stat_t st;
    if (!h)
        return 0;
    if (fnd_stream_stat(h, &st) == FND_OK && st.type == FND_TYPE_CONSOLE)
        return 1;
    errno = ENOTTY;
    return 0;
}

int fsync(int fd) {
    fnd_handle h = pkcl_handle(fd);
    return h ? pkcl_fail(fnd_sync(h)) : -1;
}

int ftruncate(int fd, off_t length) {
    fnd_handle h = pkcl_handle(fd);
    if (!h)
        return -1;
    if (length < 0) {
        errno = EINVAL;
        return -1;
    }
    uint32_t args[4] = {0, 0, (uint32_t)length, 0};
    fnd_status status = fnd_call(h, FND_ST_TRUNCATE, args, 0, 0, 0, 0, 0);
    if (status == FND_E_UNSUPPORTED) {
        errno = EINVAL; /* POSIX: 切り詰められない種類のファイル */
        return -1;
    }
    return pkcl_fail(status);
}

int fstat(int fd, struct stat *st) {
    fnd_handle h = pkcl_handle(fd);
    fnd_stat_t fs;
    if (!h)
        return -1;
    if (pkcl_fail(fnd_stream_stat(h, &fs)))
        return -1;
    return pkcl_stat_convert(&fs, st);
}

mode_t umask(mode_t mask) {
    mode_t old = current_umask;
    current_umask = mask & 0777;
    return old;
}

mode_t pkcl_current_umask(void) { return current_umask; }

void _exit(int code) { fnd_exit(code); }

uid_t getuid(void) { return 0; }
uid_t geteuid(void) { return 0; }
gid_t getgid(void) { return 0; }
gid_t getegid(void) { return 0; }
