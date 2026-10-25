/* Newlib の OS 層（libgloss が普通に提供する `_open` や `_sbrk` など）を、pkcl の POSIX 関数と
 * Foundation のカーネルに結びつける（doc/spec_pkcl.md §9）。Newlib のヘッダを使うときだけ
 * リンクする。Newlib の reent 層（`_open_r` など）は Newlib 自身が持ち、ここの `_open` を呼ぶ。 */
#define _GNU_SOURCE 1 /* libc のヘッダに O_CLOEXEC などを出させる */
#include <errno.h>
#include <fcntl.h>
#include <foundation/fnd.h>
#include <foundation/sys.h>
#include <pkcl/pkcl.h>
#include <stddef.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/times.h>
#include <unistd.h>

int _open(const char *path, int flags, int mode) { return open(path, flags, mode); }
int _close(int fd) { return close(fd); }
int _read(int fd, void *buf, size_t len) { return (int)read(fd, buf, len); }
int _write(int fd, const void *buf, size_t len) { return (int)write(fd, buf, len); }
off_t _lseek(int fd, off_t offset, int whence) { return lseek(fd, offset, whence); }
int _fstat(int fd, struct stat *st) { return fstat(fd, st); }
int _stat(const char *path, struct stat *st) { return stat(path, st); }
int _isatty(int fd) { return isatty(fd); }
int _unlink(const char *path) { return unlink(path); }
int _mkdir(const char *path, mode_t mode) { return mkdir(path, mode); }
int _fcntl(int fd, int cmd, int arg) { return fcntl(fd, cmd, arg); }

/* FS2 にリンクはなく、pkcl はシグナルとプロセスの複製を持たない（§6.4、§6.5）。 */
int _link(const char *old_path, const char *new_path) {
    (void)old_path, (void)new_path;
    errno = ENOSYS;
    return -1;
}
int _kill(int pid, int sig) {
    (void)pid, (void)sig;
    errno = ENOSYS;
    return -1;
}
int _fork(void) {
    errno = ENOSYS;
    return -1;
}
int _wait(int *status) {
    (void)status;
    errno = ECHILD;
    return -1;
}
int _execve(const char *path, char *const argv[], char *const envp[]) {
    (void)path, (void)argv, (void)envp;
    errno = ENOSYS;
    return -1;
}
int _getpid(void) { return 1; } /* §7 G1: 自分の pid を知る手段がまだない */

/* カーネルの tick（10 ms）と RTC から。 */
int _gettimeofday(struct timeval *tv, void *tz) {
    (void)tz;
    if (tv) {
        uint32_t ticks = (uint32_t)fnd_sys_time_get(0), hz = (uint32_t)fnd_sys_time_get(2);
        tv->tv_sec = (time_t)(uint32_t)fnd_sys_time_get(3);
        tv->tv_usec = hz ? (long)((ticks % hz) * (1000000u / hz)) : 0;
    }
    return 0;
}
clock_t _times(struct tms *buf) {
    uint32_t ticks = (uint32_t)fnd_sys_time_get(0);
    if (buf)
        buf->tms_utime = buf->tms_stime = buf->tms_cutime = buf->tms_cstime = 0;
    return (clock_t)ticks;
}

/* ヒープは mem_grow で伸ばす（8 の倍数。縮める手段はない）。nano allocator はこれを呼ぶ。 */
void *cpt32_newlib_sbrk(ptrdiff_t increment) {
    if (increment < 0) {
        errno = ENOMEM;
        return (void *)-1;
    }
    uint32_t delta = ((uint32_t)increment + 7u) & ~7u;
    int32_t old = fnd_sys_mem_grow(delta);
    if (old < 0) {
        errno = pkcl_errno(old);
        return (void *)-1;
    }
    return (void *)(uintptr_t)(uint32_t)old;
}
