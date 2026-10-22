#ifndef FOUNDATION_FND_INTERNAL_H
#define FOUNDATION_FND_INTERNAL_H
#include <foundation/fnd.h>

/* fnd_proc_exec (doc/spec_Foundation_API.md §11.6) passing console as the
 * CONSOLE role (none if 0). init uses it: it has no roles of its own (§12)
 * and gives the shell the console it opened. */
fnd_status fnd__exec(const char *path, const fnd_exec_t *ex, fnd_handle console, fnd_handle *proc);

#endif
