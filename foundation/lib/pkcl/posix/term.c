/* 端末・ioctl・poll・pipe（doc/spec_pkcl.md §6.6）。端末は console の LINE / RAW モードに対応づける。 */
#define _GNU_SOURCE 1
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/termios.h>
#include <unistd.h>

#include <foundation/api.h>
#include <foundation/fnd.h>
#include <foundation/sys.h>
#include <pkcl/pkcl.h>

static int is_console(int fd) {
    fnd_handle h = pkcl_handle(fd);
    fnd_stat_t st;
    if (h && fnd_stream_stat(h, &st) == FND_OK && st.type == FND_TYPE_CONSOLE)
        return 1;
    if (h)
        errno = ENOTTY;
    return 0;
}

/* LINE: 行編集と echo をコンソールが行う。RAW: 1 キーずつ返す。 */
static int raw_mode;

int tcgetattr(int fd, struct termios *t) {
    if (!is_console(fd))
        return -1;
    memset(t, 0, sizeof *t);
    t->c_iflag = ICRNL | IXON;
    t->c_oflag = OPOST | ONLCR;
    t->c_cflag = CS8 | CREAD | CLOCAL | B38400;
    t->c_lflag = ISIG | IEXTEN | (raw_mode ? 0 : (ICANON | ECHO | ECHOE | ECHOK));
    t->c_cc[VINTR] = 3;
    t->c_cc[VQUIT] = 28;
    t->c_cc[VERASE] = 127;
    t->c_cc[VKILL] = 21;
    t->c_cc[VEOF] = 4;
    t->c_cc[VMIN] = 1;
    t->c_cc[VSTART] = 17;
    t->c_cc[VSTOP] = 19;
    t->c_cc[VSUSP] = 26;
    t->c_ispeed = t->c_ospeed = B38400;
    return 0;
}

int tcsetattr(int fd, int actions, const struct termios *t) {
    (void)actions;
    if (!is_console(fd))
        return -1;
    int raw = !(t->c_lflag & ICANON);
    if (raw != raw_mode) {
        if (pkcl_fail(fnd_con_set_mode(raw ? FND_CON_RAW : FND_CON_LINE)))
            return -1;
        raw_mode = raw;
    }
    return 0;
}

int tcflush(int fd, int queue) {
    (void)queue;
    return is_console(fd) ? 0 : -1;
}
int tcdrain(int fd) { return is_console(fd) ? 0 : -1; }
speed_t cfgetispeed(const struct termios *t) { return t->c_ispeed; }
speed_t cfgetospeed(const struct termios *t) { return t->c_ospeed; }
int cfsetispeed(struct termios *t, speed_t speed) { t->c_ispeed = speed; return 0; }
int cfsetospeed(struct termios *t, speed_t speed) { t->c_ospeed = speed; return 0; }
pid_t tcgetpgrp(int fd) { return is_console(fd) ? 1 : -1; }
int tcsetpgrp(int fd, pid_t pgrp) { (void)pgrp; return is_console(fd) ? 0 : -1; }

int ttyname_r(int fd, char *buf, size_t size) {
    if (!is_console(fd))
        return ENOTTY;
    if (size < sizeof "/dev/console")
        return ERANGE;
    strcpy(buf, "/dev/console");
    return 0;
}

int ioctl(int fd, unsigned long request, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap, request);
    void *arg = __builtin_va_arg(ap, void *);
    __builtin_va_end(ap);
    if (request == TIOCGWINSZ) {
        uint32_t cols, rows;
        if (!is_console(fd))
            return -1;
        if (pkcl_fail(fnd_con_size(&cols, &rows)))
            return -1;
        struct winsize *ws = arg;
        ws->ws_col = (unsigned short)cols;
        ws->ws_row = (unsigned short)rows;
        ws->ws_xpixel = ws->ws_ypixel = 0;
        return 0;
    }
    errno = is_console(fd) || !pkcl_handle(fd) ? EINVAL : ENOTTY;
    return -1;
}

/* 待てる fd は read が返ることを保証できないので、要求された入出力はすべて準備済みとして返す（§7 G9）。 */
int poll(struct pollfd *fds, nfds_t count, int timeout) {
    (void)timeout;
    int ready = 0;
    for (nfds_t i = 0; i < count; ++i) {
        if (fds[i].fd < 0 || !pkcl_handle(fds[i].fd)) {
            fds[i].revents = fds[i].fd < 0 ? 0 : POLLNVAL;
        } else {
            fds[i].revents = fds[i].events & (POLLIN | POLLOUT);
        }
        ready += fds[i].revents != 0;
    }
    return ready;
}

/* 2 つの端は pipe サービスの CHANNEL 1 本ずつ。読み端を開いて id を受け取り、その id で書き端を付ける。 */
static int pipe_open(uint32_t mode, uint32_t id, uint32_t *out_id) {
    int32_t svc = fnd_sys_svc_lookup("pipe");
    if (svc <= 0) {
        errno = ENOSYS; /* VM でない構成には pipe サービスがない */
        return -1;
    }
    uint32_t args[4] = {mode, id, 0, 0};
    fnd_msg rep;
    fnd_status status = fnd_call((fnd_handle)svc, FND_PIPE_OPEN, args, 0, 0, 0, 0, &rep);
    fnd_close((fnd_handle)svc);
    if (pkcl_fail(status))
        return -1;
    if (!(rep.flags & FND_MSG_XFER_HANDLE) || !rep.a3) {
        errno = EIO;
        return -1;
    }
    if (out_id)
        *out_id = rep.a0;
    return (int)rep.a3;
}

int pipe2(int fds[2], int flags) {
    pkcl_init();
    uint32_t id = 0;
    int32_t r = pipe_open(0, 0, &id);
    if (r < 0)
        return -1;
    int32_t w = pipe_open(1, id, 0);
    if (w < 0) {
        fnd_close((fnd_handle)r);
        return -1;
    }
    uint32_t nb = (uint32_t)flags & O_NONBLOCK;
    int rfd = pkcl_fd_install((fnd_handle)r, O_RDONLY | PKCL_FD_PIPE | nb);
    int wfd = rfd < 0 ? -1 : pkcl_fd_install((fnd_handle)w, O_WRONLY | PKCL_FD_PIPE | nb);
    if (wfd < 0) {
        fnd_close((fnd_handle)r);
        fnd_close((fnd_handle)w);
        if (rfd >= 0)
            close(rfd);
        return -1;
    }
    fds[0] = rfd;
    fds[1] = wfd;
    return 0;
}

int pipe(int fds[2]) { return pipe2(fds, 0); }
