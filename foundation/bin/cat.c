/* cat path... (doc/spec_Foundation_shell.md §10.3). */
#include "cmd.h"

static int32_t cat_file(const char *path, uint8_t *buf) {
    fnd_stat_t st;
    fnd_status status = fnd_path_stat(path, &st);
    if (status != FND_OK)
        return status;
    if (st.type == FND_TYPE_DIR)
        return FND_E_IS_DIR;
    fnd_handle h;
    status = fnd_file_open(path, FND_OPEN_READ, FND_MODE_DEFAULT, &h);
    if (status != FND_OK)
        return status;
    for (;;) {
        uint32_t got = 0;
        status = fnd_read(h, buf, FND_IPC_MAX_DATA, &got);
        if (status != FND_OK || got == 0)
            break;
        status = fnd_write(fnd_std_out(), buf, got, 0);
        if (status != FND_OK)
            break;
    }
    fnd_close(h);
    return status;
}

static int32_t cat(uint32_t argc) {
    if (argc < 2u)
        return cmd_usage("cat path...");
    uint8_t *buf = fnd_mem_alloc(FND_IPC_MAX_DATA);
    if (!buf)
        return cmd_report("cat", 0, FND_E_NO_MEMORY);
    int32_t result = FND_OK;
    for (uint32_t i = 1; i < argc; ++i) {
        fnd_status status = cat_file(fnd_argv(i), buf);
        if (status != FND_OK) {
            int32_t code = cmd_report("cat", fnd_argv(i), status);
            if (result == FND_OK)
                result = code;
        }
    }
    fnd_mem_free(buf);
    return result;
}

CMD_MAIN(cat)
