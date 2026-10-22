/* ps (doc/spec_Foundation_shell.md §10.3): the process list from sys_info
 * sel 1. */
#include "cmd.h"

#define PS_MAX 16u
static uint32_t buf[PS_MAX * FND_PROCINFO_BYTES / 4u];

static int32_t ps(uint32_t argc) {
    if (argc > 1u)
        return cmd_usage("ps");
    int32_t size = fnd_sys_info(FND_INFO_PROCESSES, buf, sizeof buf);
    if (size < 0)
        return cmd_report("ps", 0, size);
    if ((uint32_t)size > sizeof buf)
        size = (int32_t)sizeof buf;
    static const char *const states[3] = {"RUN  ", "BLOCK", "EXIT "};
    cmd_out("  PID PARENT STATE PRIO     MEM NAME\n");
    for (uint32_t off = 0; off + FND_PROCINFO_BYTES <= (uint32_t)size; off += FND_PROCINFO_BYTES) {
        const uint32_t *p = buf + off / 4u;
        char name[17];
        for (uint32_t i = 0; i < 16u; ++i)
            name[i] = (char)(p[8u + i / 4u] >> (24u - 8u * (i % 4u)));
        name[16] = 0;
        cmd_out_dec((int32_t)p[0], 5);
        cmd_out_dec((int32_t)p[1], 7);
        cmd_out(" ");
        cmd_out(p[2] < 3u ? states[p[2]] : "?    ");
        cmd_out_dec((int32_t)p[3], 5);
        cmd_out_dec((int32_t)(p[5] / 1024u), 7);
        cmd_out("K ");
        cmd_out(name);
        cmd_out("\n");
    }
    return FND_OK;
}

CMD_MAIN(ps)
