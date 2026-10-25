/* pkcl の errno と変換表（doc/spec_pkcl.md §5）。 */
#include <errno.h>
#include <foundation/api.h>
#include <pkcl/pkcl.h>

int errno;

int pkcl_errno(fnd_status status) {
    switch (status) {
    case FND_E_BAD_CALL:
    case FND_E_UNSUPPORTED:
        return ENOSYS;
    case FND_E_BAD_ARG:
    case FND_E_BAD_PATH:
        return EINVAL;
    case FND_E_BAD_HANDLE:
        return EBADF;
    case FND_E_ACCESS:
        return EACCES;
    case FND_E_NO_MEMORY:
    case FND_E_ADDRESS_CONFLICT:
    case FND_E_REGION_LIMIT:
        return ENOMEM;
    case FND_E_LIMIT:
        return EMFILE;
    case FND_E_TIMEOUT:
        return ETIMEDOUT;
    case FND_E_WOULD_BLOCK:
        return EAGAIN;
    case FND_E_NOT_FOUND:
    case FND_E_DEPENDENCY:
        return ENOENT;
    case FND_E_EXISTS:
        return EEXIST;
    case FND_E_FORMAT:
        return ENOEXEC;
    case FND_E_NO_MEDIA:
        return ENODEV;
    case FND_E_READ_ONLY:
        return EROFS;
    case FND_E_BUSY:
        return EBUSY;
    case FND_E_KILLED:
    case FND_E_INTERRUPTED:
        return EINTR;
    case FND_E_NOT_DIR:
        return ENOTDIR;
    case FND_E_IS_DIR:
        return EISDIR;
    case FND_E_NOT_EMPTY:
        return ENOTEMPTY;
    case FND_E_NAME_TOO_LONG:
        return ENAMETOOLONG;
    case FND_E_NO_SPACE:
        return ENOSPC;
    case FND_E_CROSS_DEVICE:
        return EXDEV;
    default:
        return status >= 0 ? 0 : EIO; /* PEER_GONE, IO, NEEDS_FSCK と未知のコード */
    }
}

int pkcl_fail(fnd_status status) {
    if (status >= 0)
        return 0;
    errno = pkcl_errno(status);
    return -1;
}
