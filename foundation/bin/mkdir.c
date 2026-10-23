/* mkdir path... (doc/spec_Foundation_shell.md §10.3). */
#include "cmd.h"

static int32_t mkdir(uint32_t argc) {
    if (argc < 2u)
        return cmd_usage("mkdir path...");
    int32_t result = FND_OK;
    for (uint32_t i = 1; i < argc; ++i) {
        fnd_status status = fnd_dir_make(fnd_argv(i), FND_MODE_DEFAULT);
        if (status != FND_OK) {
            int32_t code = cmd_report("mkdir", fnd_argv(i), status);
            if (result == FND_OK)
                result = code;
        }
    }
    return result;
}

CMD_MAIN(mkdir)
