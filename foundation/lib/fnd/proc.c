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
    uint32_t envc = 0;
    while (ex->envp && ex->envp[envc])
        ++envc;
    uint32_t argv_off = off, roles_off = argv_off + 4u * argc;
    uint32_t env_off = roles_off + 8u * role_count;
    off = env_off + 4u * envc;
    if (argc > FND_SPAWN_MAX_HANDLES * 64u || envc > 128u || off > FND_STARTUP_MAX)
        return FND_E_LIMIT;
    char *bytes = (char *)block;
    for (uint32_t i = 0; i < argc; ++i) {
        const char *arg = i == 0 && !((ex->flags & FND_EXEC_KEEP_ARGV0) && ex->argv && ex->argv[0]) ? argv0 : ex->argv[i];
        uint32_t n = str_len(arg) + 1u;
        if (n > FND_STARTUP_MAX - off)
            return FND_E_LIMIT;
        block[argv_off / 4u + i] = off;
        for (uint32_t k = 0; k < n; ++k)
            bytes[off + k] = arg[k];
        off += n;
    }
    for (uint32_t i = 0; i < envc; ++i) {
        uint32_t len = str_len(ex->envp[i]) + 1u;
        if (len > FND_STARTUP_MAX - off)
            return FND_E_LIMIT;
        block[env_off / 4u + i] = off;
        for (uint32_t k = 0; k < len; ++k)
            bytes[off + k] = ex->envp[i][k];
        off += len;
    }
    uint32_t cwd_off = off, n = str_len(cwd) + 1u;
    if (n > FND_STARTUP_MAX - off)
        return FND_E_LIMIT;
    for (uint32_t k = 0; k < n; ++k)
        bytes[off + k] = cwd[k];
    off = (off + n + 7u) & ~7u;
    if (off > FND_STARTUP_MAX)
        return FND_E_LIMIT;
    for (uint32_t i = 0; i < role_count; ++i) {
        block[roles_off / 4u + 2u * i] = i + 1u;     /* role 1..5 */
        block[roles_off / 4u + 2u * i + 1u] = i + 1u; /* child handle 1 + index */
    }
    *(fnd_startup *)block =
        (fnd_startup){FND_STARTUP_MAGIC, FND_STARTUP_VERSION, off, argc, argv_off, cwd_off, role_count, roles_off, envc, env_off};
    return (int32_t)off;
}



/* §11.6 steps 2, 3, 5 and 6 for an image in memory. */
static fnd_status spawn(const void *image, uint32_t size, const char *path, const fnd_exec_t *ex,
                       fnd_handle console, const fnd_spawn_libs *libs, fnd_handle *proc) {
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
                            (uint32_t)(uintptr_t)name, (uint32_t)(uintptr_t)libs};
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

/* Reads the whole file at path into the heap (§11.6 step 1). */
static fnd_status read_image(const char *path, uint8_t **image, uint32_t *size) {
    fnd_handle file;
    fnd_status status = fnd_file_open(path, FND_OPEN_EXEC, 0, &file);
    if (status != FND_OK)
        return status;
    fnd_stat_t st;
    status = fnd_stream_stat(file, &st);
    uint32_t got = 0;
    *image = 0;
    if (status == FND_OK && (st.size_hi || st.size_lo > FND_EXEC_IMAGE_MAX))
        status = FND_E_LIMIT;
    if (status == FND_OK && !(*image = fnd_mem_alloc(st.size_lo ? st.size_lo : 1u)))
        status = FND_E_NO_MEMORY;
    if (status == FND_OK)
        status = fnd_read(file, *image, st.size_lo, &got);
    fnd_close(file);
    if (status == FND_OK && got != st.size_lo)
        status = FND_E_IO;
    if (status != FND_OK) {
        fnd_mem_free(*image);
        *image = 0;
        return status;
    }
    *size = got;
    return FND_OK;
}

/* ---- WALs (§11.6 step 4, doc/spec_Foundation.md §9.5) ---- */

#define EXEC_MAX_IMAGES 8u /* the WAX and its WALs */
#define WAX3_HEADER 160u

struct deps {
    fnd_spawn_libs head; /* lib[] follows it: the list proc_spawn reads */
    fnd_spawn_lib lib[EXEC_MAX_IMAGES - 1u];
    const char *id[EXEC_MAX_IMAGES - 1u];
    uint32_t major[EXEC_MAX_IMAGES - 1u];
};

static uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}

static int same_id(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return *a == *b;
}

/* The NUL-terminated string at off in the string table, or NULL. */
static const char *wax_string(const uint8_t *d, uint32_t size, uint32_t off) {
    uint32_t strings = rd32(d + 88), string_size = rd32(d + 92);
    if (strings > size || string_size > size - strings || off >= string_size)
        return 0;
    for (uint32_t i = off; i < string_size; ++i) {
        if (!d[strings + i])
            return (const char *)d + strings + off;
    }
    return 0;
}

/* "/lib/<id>.<major>.wal" into path (at least 150 bytes); id is at most
 * 127 bytes (WAX v3 §2). */
static void lib_path(char *path, const char *id, uint32_t major) {
    uint32_t n = 0;
    for (const char *p = "/lib/"; *p; ++p)
        path[n++] = *p;
    while (*id)
        path[n++] = *id++;
    path[n++] = '.';
    char digits[10];
    uint32_t nd = 0;
    do
        digits[nd++] = (char)('0' + major % 10u);
    while (major /= 10u);
    while (nd)
        path[n++] = digits[--nd];
    for (const char *p = ".wal"; *p; ++p)
        path[n++] = *p;
    path[n] = 0;
}

/* Reads /lib/<id>.<major>.wal for each dependency of the image d that is
 * not in deps yet, and then their dependencies. Only the tables are
 * followed here: the kernel validates the images and the minor versions. */
static fnd_status add_deps(struct deps *deps, const uint8_t *d, uint32_t size) {
    if (size < WAX3_HEADER || d[0] != 'W' || d[1] != 'A' || d[2] != 'X' || d[3] != 0 || d[4] != 0 ||
        d[5] != 3)
        return FND_OK; /* v2 or not a WAX: no WALs, and the kernel decides */
    uint32_t table = rd32(d + 52), count = rd32(d + 56);
    if (table > size || count > (size - table) / 16u)
        return FND_E_FORMAT;
    for (uint32_t i = 0; i < count; ++i) {
        const char *id = wax_string(d, size, rd32(d + table + 16u * i));
        uint32_t major = rd32(d + table + 16u * i + 4u);
        if (!id || !*id || str_len(id) > 127u || major == 0)
            return FND_E_FORMAT;
        uint32_t k = 0;
        while (k < deps->head.count && !(same_id(deps->id[k], id) && deps->major[k] == major))
            ++k;
        if (k < deps->head.count)
            continue;
        if (deps->head.count == EXEC_MAX_IMAGES - 1u)
            return FND_E_LIMIT;
        char path[160];
        lib_path(path, id, major);
        uint8_t *image;
        uint32_t image_size;
        fnd_status status = read_image(path, &image, &image_size);
        if (status == FND_E_NOT_FOUND)
            return FND_E_DEPENDENCY;
        if (status != FND_OK)
            return status;
        k = deps->head.count++;
        deps->lib[k] = (fnd_spawn_lib){(uint32_t)(uintptr_t)image, image_size};
        deps->id[k] = id;
        deps->major[k] = major;
        if ((status = add_deps(deps, image, image_size)) != FND_OK)
            return status;
    }
    return FND_OK;
}

fnd_status fnd__exec(const char *path, const fnd_exec_t *ex, fnd_handle console, fnd_handle *proc) {
    uint8_t *image;
    uint32_t size;
    fnd_status status = read_image(path, &image, &size);
    if (status != FND_OK)
        return status;
    struct deps *deps = fnd_mem_alloc(sizeof *deps);
    if (!deps) {
        fnd_mem_free(image);
        return FND_E_NO_MEMORY;
    }
    deps->head = (fnd_spawn_libs){0, 0};
    status = add_deps(deps, image, size);
    if (status == FND_OK)
        status = spawn(image, size, path, ex, console, deps->head.count ? &deps->head : 0, proc);
    for (uint32_t i = 0; i < deps->head.count; ++i)
        fnd_mem_free((void *)(uintptr_t)deps->lib[i].image);
    fnd_mem_free(deps);
    fnd_mem_free(image);
    return status;
}

fnd_status fnd_proc_exec(const char *path, const fnd_exec_t *ex, fnd_handle *proc) {
    return fnd__exec(path, ex, fnd_console(), proc);
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

int32_t fnd_proc_fork(void) { return fnd_sys_proc_fork(); }
