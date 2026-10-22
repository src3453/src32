/* free (doc/spec_Foundation_shell.md §10.3): RAM and free RAM in KiB from
 * sys_info sel 0, then the VRAM and PCMRAM capacities from sel 3. */
#include "cmd.h"

static int32_t free_cmd(uint32_t argc) {
    if (argc > 1u)
        return cmd_usage("free");
    uint32_t info[8];
    int32_t size = fnd_sys_info(FND_INFO_SYSTEM, info, FND_SYSINFO_BYTES);
    if (size < 0)
        return cmd_report("free", 0, size);
    cmd_out_dec((int32_t)(info[3] / 1024u), 8);
    cmd_out(" KiB total\n");
    cmd_out_dec((int32_t)(info[4] / 1024u), 8);
    cmd_out(" KiB free\n");
    size = fnd_sys_info(FND_INFO_MEMORY, info, FND_MEMINFO_BYTES);
    if (size < 0)
        return cmd_report("free", 0, size);
    cmd_out_dec((int32_t)(info[1] / 1024u), 8);
    cmd_out(" KiB VRAM\n");
    cmd_out_dec((int32_t)(info[2] / 1024u), 8);
    cmd_out(" KiB PCMRAM\n");
    return FND_OK;
}

CMD_MAIN(free_cmd)
