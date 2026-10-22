/* fnd_strerror (doc/spec_Foundation_API.md §11.9): the kernel codes as in
 * doc/spec_Foundation_shell.md §11.1, then the API codes. */
#include <foundation/fnd.h>

const char *fnd_strerror(fnd_status code) {
    switch (code) {
    case FND_E_BAD_ARG:
        return "invalid argument";
    case FND_E_BAD_HANDLE:
        return "bad handle";
    case FND_E_ACCESS:
        return "access denied";
    case FND_E_NO_MEMORY:
        return "out of memory";
    case FND_E_LIMIT:
        return "limit exceeded";
    case FND_E_PEER_GONE:
        return "service gone";
    case FND_E_NOT_FOUND:
        return "not found";
    case FND_E_EXISTS:
        return "already exists";
    case FND_E_KILLED:
        return "interrupted";
    case FND_E_FORMAT:
        return "bad format";
    case FND_E_UNSUPPORTED:
        return "not supported";
    case FND_E_ADDRESS_CONFLICT:
        return "slot in use";
    case FND_E_REGION_LIMIT:
        return "too many regions";
    case FND_E_IO:
        return "I/O error";
    case FND_E_NO_MEDIA:
        return "no media";
    case FND_E_READ_ONLY:
        return "read-only";
    case FND_E_BUSY:
        return "busy";
    case FND_E_DEPENDENCY:
        return "missing dependency";
    case FND_E_NOT_DIR:
        return "not a directory";
    case FND_E_IS_DIR:
        return "is a directory";
    case FND_E_NOT_EMPTY:
        return "directory not empty";
    case FND_E_BAD_PATH:
        return "bad path";
    case FND_E_NAME_TOO_LONG:
        return "name too long";
    case FND_E_NO_SPACE:
        return "no space";
    case FND_E_CROSS_DEVICE:
        return "cross-device move";
    case FND_E_INTERRUPTED:
        return "interrupted";
    case FND_E_NEEDS_FSCK:
        return "needs fsck";
    default:
        return "error";
    }
}
