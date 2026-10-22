/* mv old new (doc/spec_Foundation_shell.md §10.5): the fs's rename, so only
 * within one mount; an existing new is FND_E_EXISTS. */
#include "cmd.h"

static int32_t mv(uint32_t argc) {
    if (argc != 3u)
        return cmd_usage("mv old new");
    fnd_status status = fnd_file_rename(fnd_argv(1), fnd_argv(2));
    return status == FND_OK ? FND_OK : cmd_report("mv", fnd_argv(1), status);
}

CMD_MAIN(mv)
