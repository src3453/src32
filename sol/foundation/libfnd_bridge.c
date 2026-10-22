/* Private sol bridge. This dispatch table is linked with libfnd into one
 * PROTECTED WAX by build_foundation.py; it is not a public syscall ABI. */
#include <foundation/fnd.h>
#define PTR(type, value) ((type *)(uintptr_t)(value))

int32_t sol_fnd_dispatch(uint32_t op, uint32_t a, uint32_t b, uint32_t c, uint32_t d) {
    switch (op) {
    case 0: return fnd_init();
    case 1: return fnd_argc();
    case 2: return (uintptr_t)fnd_argv(a);
    case 3: return fnd_std_in();
    case 4: return fnd_std_out();
    case 5: return fnd_std_err();
    case 6: return fnd_console();
    case 7: return fnd_fs();
    case 8: return fnd_file_open(PTR(const char, a), b, c, PTR(fnd_handle, d));
    case 9: return fnd_read(a, PTR(void, b), c, PTR(uint32_t, d));
    case 10: return fnd_write(a, PTR(const void, b), c, PTR(uint32_t, d));
    case 11: return fnd_close(a);
    case 12: return fnd_stream_stat(a, PTR(fnd_stat_t, b));
    case 13: return fnd_path_stat(PTR(const char, a), PTR(fnd_stat_t, b));
    case 14: return fnd_cwd_set(PTR(const char, a));
    case 15: return fnd_cwd_get(PTR(char, a), b, PTR(uint32_t, c));
    case 16: return fnd_con_write(PTR(const char, a));
    case 17: return fnd_con_getkey(PTR(fnd_key_t, a));
    case 18: return fnd_con_pollkey(PTR(fnd_key_t, a));
    case 19: return fnd_con_set_mode(a);
    case 20: return fnd_con_clear();
    case 21: return fnd_con_size(PTR(uint32_t, a), PTR(uint32_t, b));
    case 22: return fnd_proc_exec(PTR(const char, a), PTR(const fnd_exec_t, b), PTR(fnd_handle, c));
    case 23: return fnd_proc_wait(a, PTR(int32_t, b));
    case 24: return fnd_proc_kill(a);
    case 25: return (uintptr_t)fnd_mem_alloc(a);
    case 26: fnd_mem_free(PTR(void, a)); return FND_OK;
    case 27: return (uintptr_t)fnd_strerror((int32_t)a);
    case 28: {
        const char *path = PTR(const char, a);
        uint32_t n = 0;
        while (path[n] && n <= FND_IPC_MAX_DATA) ++n;
        if (n > FND_IPC_MAX_DATA) return FND_E_NAME_TOO_LONG;
        fnd_msg rep;
        fnd_status status = fnd_call(fnd_fs(), FND_FS_DIR_OPEN, 0, path, n, 0, 0, &rep);
        if (status == FND_OK) *PTR(fnd_handle, b) = rep.a3;
        return status;
    }
    case 29: {
        if (c > FND_IPC_MAX_DATA) return FND_E_LIMIT;
        fnd_msg rep;
        fnd_status status = fnd_call(a, FND_DIR_NEXT, 0, 0, 0, PTR(void, b), c, &rep);
        if (status == FND_OK) *PTR(uint32_t, d) = rep.len;
        return status;
    }
    case 30: return fnd_call(a, FND_DIR_REWIND, 0, 0, 0, 0, 0, 0);
    default: return FND_E_BAD_ARG;
    }
}
