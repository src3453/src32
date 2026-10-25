#ifndef PKCL_ERRNO_H
#define PKCL_ERRNO_H

/* pkcl の errno（doc/spec_pkcl.md §5）。値は Newlib の sys/errno.h と同じで、
 * 後から Newlib を載せても数値が食い違わない。単一のグローバルで、スレッドごとではない。 */
extern int errno;

#define EPERM 1
#define ENOENT 2
#define EINTR 4
#define EIO 5
#define ENOEXEC 8
#define EBADF 9
#define ECHILD 10
#define EAGAIN 11
#define EWOULDBLOCK EAGAIN
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EBUSY 16
#define EEXIST 17
#define EXDEV 18
#define ENODEV 19
#define ENOTDIR 20
#define EISDIR 21
#define EINVAL 22
#define ENFILE 23
#define EMFILE 24
#define ENOTTY 25
#define EFBIG 27
#define ENOSPC 28
#define ESPIPE 29
#define EROFS 30
#define EPIPE 32
#define EDOM 33
#define ERANGE 34
#define ENOSYS 88
#define ENOTEMPTY 90
#define ENAMETOOLONG 91
#define ELOOP 92
#define EPROTO 71
#define ENOTSUP 134
#define EOVERFLOW 139
#define ETIMEDOUT 116

#endif
