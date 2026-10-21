#ifndef FOUNDATION_ERRORS_H
#define FOUNDATION_ERRORS_H

/* Kernel error codes (doc/spec_Foundation.md §4). Results are i32: >= 0 is
 * success, negative is an error. */
#define FND_OK 0
#define FND_E_BAD_CALL (-1)
#define FND_E_BAD_ARG (-2)
#define FND_E_BAD_HANDLE (-3)
#define FND_E_ACCESS (-4)
#define FND_E_NO_MEMORY (-5)
#define FND_E_LIMIT (-6)
#define FND_E_PEER_GONE (-7)
#define FND_E_TIMEOUT (-8)
#define FND_E_WOULD_BLOCK (-9)
#define FND_E_NOT_FOUND (-10)
#define FND_E_EXISTS (-11)
#define FND_E_FORMAT (-12)
#define FND_E_UNSUPPORTED (-13)
#define FND_E_ADDRESS_CONFLICT (-14)
#define FND_E_REGION_LIMIT (-15)
#define FND_E_IO (-16)
#define FND_E_NO_MEDIA (-17)
#define FND_E_READ_ONLY (-18)
#define FND_E_BUSY (-19)
#define FND_E_KILLED (-20)
#define FND_E_DEPENDENCY (-21)

#endif
