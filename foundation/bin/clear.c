/* clear (doc/spec_Foundation_shell.md §10.3): through the CONSOLE role, so
 * the screen clears even when STDOUT is redirected. */
#include "cmd.h"

static int32_t clear(uint32_t argc) {
    if (argc > 1u)
        return cmd_usage("clear");
    if (!fnd_console())
        return cmd_report("clear", 0, FND_E_BAD_HANDLE);
    fnd_status status = fnd_con_clear();
    return status == FND_OK ? FND_OK : cmd_report("clear", 0, status);
}

CMD_MAIN(clear)
