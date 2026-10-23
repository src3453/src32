/* rmdir path... (doc/spec_Foundation_shell.md §10.3). */
#include "cmd.h"

static int32_t rmdir(uint32_t argc) {
    if (argc < 2u)
        return cmd_usage("rmdir path...");
    int32_t result = FND_OK;
    for (uint32_t i = 1; i < argc; ++i) {
        fnd_status status = fnd_dir_remove(fnd_argv(i));
        if (status != FND_OK) {
            int32_t code = cmd_report("rmdir", fnd_argv(i), status);
            if (result == FND_OK)
                result = code;
        }
    }
    return result;
}

CMD_MAIN(rmdir)
