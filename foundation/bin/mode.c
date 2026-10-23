/* Console display selection; LINE/RAW input mode is independent. */
#include "cmd.h"

static int equal(const char *a, const char *b) {
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}

static int32_t mode(uint32_t argc) {
    uint32_t selected = FND_CON_DISPLAY_80;
    if (argc > 2)
        return cmd_usage("mode [40|80|8x16]");
    if (argc == 2) {
        const char *arg = fnd_argv(1);
        if (equal(arg, "40")) selected = FND_CON_DISPLAY_40;
        else if (equal(arg, "80")) selected = FND_CON_DISPLAY_80;
        else if (equal(arg, "8x16")) selected = FND_CON_DISPLAY_8X16;
        else return cmd_usage("mode [40|80|8x16]");
    }
    fnd_handle console = fnd_console();
    if (!console)
        return cmd_report("mode", 0, FND_E_BAD_HANDLE);
    uint32_t args[4] = {selected, 0, 0, 0};
    fnd_msg reply;
    fnd_status status = fnd_call(console, argc == 2 ? FND_CON_SET_DISPLAY : FND_CON_GET_DISPLAY,
                                args, 0, 0, 0, 0, &reply);
    if (status != FND_OK)
        return cmd_report("mode", 0, status);
    if (argc == 1) {
        if (reply.a0 == FND_CON_DISPLAY_40) cmd_out("40x30, 8x8\n");
        else if (reply.a0 == FND_CON_DISPLAY_80) cmd_out("80x30, 8x8\n");
        else if (reply.a0 == FND_CON_DISPLAY_8X16) cmd_out("80x30, 8x16\n");
        else return cmd_report("mode", 0, FND_E_UNSUPPORTED);
    }
    return FND_OK;
}

CMD_MAIN(mode)
