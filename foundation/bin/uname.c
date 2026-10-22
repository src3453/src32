/* uname (doc/spec_Foundation_shell.md §10.3): the OS version from sys_info
 * sel 0. The shell shows its own version at start-up and in help. */
#include "cmd.h"

static int32_t uname(uint32_t argc) {
    if (argc > 1u)
        return cmd_usage("uname");
    uint32_t info[8];
    int32_t size = fnd_sys_info(FND_INFO_SYSTEM, info, FND_SYSINFO_BYTES);
    if (size < 0)
        return cmd_report("uname", 0, size);
    cmd_out("Foundation OS (kernel ABI ");
    cmd_out_dec((int32_t)info[0], 0);
    cmd_out(".");
    cmd_out_dec((int32_t)info[1], 0);
    cmd_out(")\n");
    return FND_OK;
}

CMD_MAIN(uname)
