/* libfnd startup, roles and streams (doc/spec_Foundation_API.md §11.2,
 * §11.3, §11.5). Provisional, see <foundation/fnd.h>: proc_startup is not
 * implemented yet, so fnd_init continues with argc = 0 as §11.2 says for a
 * missing startup block, and the console roles are the linked-in console. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "con_pcg.h"

static fnd_handle console_role;

fnd_status fnd_init(void) {
    fnd_status status = con_pcg_init();
    if (status == FND_OK)
        console_role = FND_LOCAL_CONSOLE;
    return status;
}

uint32_t fnd_argc(void) { return 0; }
const char *fnd_argv(uint32_t i) {
    (void)i;
    return 0;
}
fnd_handle fnd_std_in(void) { return console_role; }
fnd_handle fnd_std_out(void) { return console_role; }
fnd_handle fnd_std_err(void) { return console_role; }
fnd_handle fnd_console(void) { return console_role; }
fnd_handle fnd_fs(void) { return 0; }

void fnd_exit(int32_t code) {
    for (;;)
        fnd_sys_proc_exit(code);
}

fnd_status fnd_write(fnd_handle h, const void *buf, uint32_t len, uint32_t *put) {
    if (h == 0 || h != console_role)
        return FND_E_BAD_HANDLE;
    con_pcg_write((const uint8_t *)buf, len);
    if (put)
        *put = len;
    return FND_OK;
}

fnd_status fnd_con_write(const char *text) {
    uint32_t len = 0;
    while (text[len])
        ++len;
    return fnd_write(fnd_std_out(), text, len, 0);
}

fnd_status fnd_con_getkey(fnd_key_t *key) {
    if (!fnd_console())
        return FND_E_BAD_HANDLE;
    con_pcg_read_key(key);
    return FND_OK;
}

fnd_status fnd_con_clear(void) {
    if (!fnd_console())
        return FND_E_BAD_HANDLE;
    con_pcg_clear();
    return FND_OK;
}

fnd_status fnd_con_size(uint32_t *cols, uint32_t *rows) {
    if (!fnd_console())
        return FND_E_BAD_HANDLE;
    *cols = CON_PCG_COLS;
    *rows = CON_PCG_ROWS;
    return FND_OK;
}
