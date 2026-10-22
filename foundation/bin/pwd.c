/* pwd (doc/spec_Foundation_shell.md §10.3): the cwd of the startup block,
 * which fnd_init gave to this process's fs session. */
#include "cmd.h"

static int32_t pwd(uint32_t argc) {
    if (argc > 1u)
        return cmd_usage("pwd");
    uint32_t len;
    char *cwd = fnd_mem_alloc(FND_IPC_MAX_DATA + 1u);
    if (!cwd)
        return cmd_report("pwd", 0, FND_E_NO_MEMORY);
    fnd_status status = fnd_cwd_get(cwd, FND_IPC_MAX_DATA + 1u, &len);
    if (status != FND_OK)
        return cmd_report("pwd", 0, status);
    cmd_out(cwd);
    cmd_out("\n");
    return FND_OK;
}

CMD_MAIN(pwd)
