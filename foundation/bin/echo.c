/* echo [word...] (doc/spec_Foundation_shell.md §10.3). */
#include "cmd.h"

static int32_t echo(uint32_t argc) {
    for (uint32_t i = 1; i < argc; ++i) {
        if (i > 1u)
            cmd_out(" ");
        cmd_out(fnd_argv(i));
    }
    cmd_out("\n");
    return FND_OK;
}

CMD_MAIN(echo)
