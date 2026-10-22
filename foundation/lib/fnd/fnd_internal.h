#ifndef FOUNDATION_FND_INTERNAL_H
#define FOUNDATION_FND_INTERNAL_H
#include <foundation/fnd.h>

/* Starts an image that is already in memory, as fnd_proc_exec does after
 * reading path (doc/spec_Foundation_API.md §11.6 steps 2-5), passing
 * console as the CONSOLE role (none if 0). init uses it: it has no roles of
 * its own (§12) and starts a shell linked into its image until the fs on
 * the boot disk holds /bin/shell.wax. */
fnd_status fnd__spawn_image(const void *image, uint32_t size, const char *path, const fnd_exec_t *ex,
                            fnd_handle console, fnd_handle *proc);

#endif
