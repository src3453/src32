/* libfnd processes (doc/spec_Foundation_API.md §11.6). */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "fnd_internal.h"

static uint32_t str_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

/* Lays out the §9.3 startup block in block; returns its size or
 * FND_E_LIMIT if it would pass FND_STARTUP_MAX. */
static int32_t build_startup(uint32_t *block, const fnd_exec_t *ex, const char *argv0, const char *cwd,
                             uint32_t role_count) {
    uint32_t argc = ex->argc ? ex->argc : 1u;
    uint32_t off = sizeof(fnd_startup);
    uint32_t argv_off = off, roles_off = argv_off + 4u * argc;
    off = roles_off + 8u * role_count;
    if (argc > FND_SPAWN_MAX_HANDLES * 64u || off > FND_STARTUP_MAX)
        return FND_E_LIMIT;
    char *bytes = (char *)block;
    for (uint32_t i = 0; i < argc; ++i) {
        const char *arg = i == 0 ? argv0 : ex->argv[i];
        uint32_t n = str_len(arg) + 1u;
        if (n > FND_STARTUP_MAX - off)
            return FND_E_LIMIT;
        block[argv_off / 4u + i] = off;
        for (uint32_t k = 0; k < n; ++k)
            bytes[off + k] = arg[k];
        off += n;
    }
    uint32_t cwd_off = off, n = str_len(cwd) + 1u;
    if (n > FND_STARTUP_MAX - off)
        return FND_E_LIMIT;
    for (uint32_t k = 0; k < n; ++k)
        bytes[off + k] = cwd[k];
    off = (off + n + 3u) & ~3u;
    if (off > FND_STARTUP_MAX)
        return FND_E_LIMIT;
    for (uint32_t i = 0; i < role_count; ++i) {
        block[roles_off / 4u + 2u * i] = i + 1u;     /* role 1..5 */
        block[roles_off / 4u + 2u * i + 1u] = i + 1u; /* child handle 1 + index */
    }
    *(fnd_startup *)block =
        (fnd_startup){FND_STARTUP_MAGIC, FND_STARTUP_VERSION, off, argc, argv_off, cwd_off, role_count, roles_off};
    return (int32_t)off;
}


fnd_status fnd__spawn_image(const void *image, uint32_t size, const char *path, const fnd_exec_t *ex,
                            fnd_handle console, fnd_handle *proc) {
    char name[16];
    const char *base = path;
    for (const char *p = path; *p; ++p) {
        if (*p == '/')
            base = p + 1;
    }
    uint32_t n = str_len(base);
    if (n >= 4u && base[n - 4u] == '.' && (base[n - 3u] | 32) == 'w' && (base[n - 2u] | 32) == 'a' &&
        (base[n - 1u] | 32) == 'x')
        n -= 4u;
    if (n > 15u)
        n = 15u;
    for (uint32_t i = 0; i < n; ++i)
        name[i] = base[i];
    name[n] = 0;

    uint32_t *startup = fnd_mem_alloc(FND_STARTUP_MAX);
    char *cwd_now = fnd_mem_alloc(FND_IPC_MAX_DATA + 1u);
    if (!startup || !cwd_now) {
        fnd_mem_free(startup);
        fnd_mem_free(cwd_now);
        return FND_E_NO_MEMORY;
    }
    const char *cwd = ex->cwd;
    if (!cwd) {
        uint32_t len;
        if (fnd_cwd_get(cwd_now, FND_IPC_MAX_DATA + 1u, &len) != FND_OK) {
            cwd_now[0] = '/';
            cwd_now[1] = 0;
        }
        cwd = cwd_now;
    }
    /* §11.6 step 2: a new fs session for the child. */
    fnd_msg rep;
    fnd_handle session = 0;
    if (fnd_call(fnd_fs(), FND_FS_SESSION_NEW, 0, 0, 0, 0, 0, &rep) >= 0)
        session = rep.a3;
    fnd_handle handles[5] = {ex->std_in ? ex->std_in : fnd_std_in(), ex->std_out ? ex->std_out : fnd_std_out(),
                             ex->std_err ? ex->std_err : fnd_std_err(), session, console};
    uint32_t count = handles[4] ? 5u : 4u;
    fnd_status status = FND_E_BAD_HANDLE;
    if (session && handles[0] && handles[1] && handles[2]) {
        int32_t len = build_startup(startup, ex, path, cwd, count);
        status = len;
        if (len >= 0) {
            fnd_spawn sp = {(uint32_t)(uintptr_t)image, size, (uint32_t)(uintptr_t)startup, (uint32_t)len,
                            (uint32_t)(uintptr_t)handles, count, FND_PRIO_APP, 0, 0, 0,
                            (uint32_t)(uintptr_t)name, 0};
            status = fnd_sys_proc_spawn(&sp);
            if (status > 0) {
                *proc = (fnd_handle)status;
                status = FND_OK;
            }
        }
    }
    if (session)
        fnd_sys_handle_close(session); /* the child holds its own copy */
    fnd_mem_free(startup);
    fnd_mem_free(cwd_now);
    return status;
}

fnd_status fnd_proc_exec(const char *path, const fnd_exec_t *ex, fnd_handle *proc) {
    fnd_handle file;
    fnd_status status = fnd_file_open(path, FND_OPEN_EXEC, 0, &file);
    if (status != FND_OK)
        return status;
    fnd_stat_t st;
    status = fnd_stream_stat(file, &st);
    uint8_t *image = 0;
    uint32_t got = 0;
    if (status == FND_OK && (st.size_hi || st.size_lo > FND_EXEC_IMAGE_MAX))
        status = FND_E_LIMIT;
    if (status == FND_OK && !(image = fnd_mem_alloc(st.size_lo ? st.size_lo : 1u)))
        status = FND_E_NO_MEMORY;
    if (status == FND_OK)
        status = fnd_read(file, image, st.size_lo, &got);
    fnd_close(file);
    if (status == FND_OK && got != st.size_lo)
        status = FND_E_IO;
    if (status == FND_OK)
        status = fnd__spawn_image(image, got, path, ex, fnd_console(), proc);
    fnd_mem_free(image);
    return status;
}

fnd_status fnd_proc_wait(fnd_handle proc, int32_t *code) {
    int32_t result;
    fnd_status status = fnd_sys_wait(proc, FND_FOREVER, &result);
    if (status != FND_OK)
        return status;
    fnd_sys_handle_close(proc);
    *code = result;
    return FND_OK;
}

fnd_status fnd_proc_kill(fnd_handle proc) { return fnd_sys_proc_kill(proc); }
