/* libfnd startup, roles, streams, console, fs paths and processes
 * (doc/spec_Foundation_API.md §11.2–§11.6, §12). Each operation is one IPC
 * call to the object's service (§2.1). */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#define ARGS_MAX 64u

static uint32_t *block; /* the startup block, on the heap */
static uint32_t argc;
static const char *argv[ARGS_MAX];
static fnd_handle roles[FND_ROLE_CONSOLE + 1u];

static uint32_t str_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

/* A NUL-terminated string at off inside the first size bytes, or NULL. */
static const char *block_string(uint32_t off, uint32_t size) {
    const char *bytes = (const char *)block;
    for (uint32_t i = off; i < size; ++i) {
        if (!bytes[i])
            return bytes + off;
    }
    return 0;
}

/* §9.3 of the kernel ABI. A missing or broken block leaves argc = 0 and no
 * roles (§11.2). */
static const char *read_startup(void) {
    int32_t size = fnd_sys_proc_startup(0, 0);
    if (size < (int32_t)sizeof(fnd_startup) || size > (int32_t)FND_STARTUP_MAX ||
        !(block = fnd_mem_alloc((uint32_t)size)) || fnd_sys_proc_startup(block, (uint32_t)size) != size)
        return 0;
    const fnd_startup *h = (const fnd_startup *)block;
    uint32_t n = (uint32_t)size;
    if (h->magic != FND_STARTUP_MAGIC || h->version != FND_STARTUP_VERSION || h->size != n ||
        h->argc > ARGS_MAX || (h->argv_off & 3u) || h->argv_off > n || h->argc * 4u > n - h->argv_off ||
        (h->roles_off & 3u) || h->roles_off > n || h->role_count * 8u > n - h->roles_off)
        return 0;
    const char *cwd = block_string(h->cwd_off, n);
    for (uint32_t i = 0; i < h->argc; ++i) {
        argv[i] = block_string(block[h->argv_off / 4u + i], n);
        if (!argv[i])
            return 0;
    }
    argc = h->argc;
    for (uint32_t i = 0; i < h->role_count; ++i) {
        uint32_t role = block[h->roles_off / 4u + 2u * i], handle = block[h->roles_off / 4u + 2u * i + 1u];
        if (role >= FND_ROLE_STDIN && role <= FND_ROLE_CONSOLE)
            roles[role] = handle;
    }
    return cwd;
}

fnd_status fnd_init(void) {
    const char *cwd = read_startup();
    /* §11.2: the startup cwd goes to the fs session. On failure it stays /. */
    if (cwd && roles[FND_ROLE_FS] && !(cwd[0] == '/' && cwd[1] == 0))
        fnd_cwd_set(cwd);
    return FND_OK;
}

uint32_t fnd_argc(void) { return argc; }
const char *fnd_argv(uint32_t i) { return i < argc ? argv[i] : 0; }
fnd_handle fnd_std_in(void) { return roles[FND_ROLE_STDIN]; }
fnd_handle fnd_std_out(void) { return roles[FND_ROLE_STDOUT]; }
fnd_handle fnd_std_err(void) { return roles[FND_ROLE_STDERR] ? roles[FND_ROLE_STDERR] : roles[FND_ROLE_STDOUT]; }
fnd_handle fnd_console(void) { return roles[FND_ROLE_CONSOLE]; }

/* §11.2: without an FS role, the first use looks the service up. */
fnd_handle fnd_fs(void) {
    if (!roles[FND_ROLE_FS]) {
        int32_t h = fnd_sys_svc_lookup("fs");
        if (h > 0)
            roles[FND_ROLE_FS] = (fnd_handle)h;
    }
    return roles[FND_ROLE_FS];
}

void fnd_exit(int32_t code) {
    for (;;)
        fnd_sys_proc_exit(code);
}

fnd_status fnd_call(fnd_handle h, uint32_t op, const uint32_t args[4], const void *data, uint32_t len,
                    void *rep_data, uint32_t rep_cap, fnd_msg *reply) {
    fnd_msg req = {op, 0, 0, 0, 0, 0, (uint32_t)(uintptr_t)data, len};
    if (args) {
        req.a0 = args[0], req.a1 = args[1], req.a2 = args[2], req.a3 = args[3];
    }
    fnd_msg rep = {0, 0, 0, 0, 0, 0, (uint32_t)(uintptr_t)rep_data, rep_cap};
    if (!h)
        return FND_E_BAD_HANDLE;
    fnd_status status = fnd_sys_ipc_call(h, &req, &rep);
    if (status != FND_OK)
        return status;
    if (reply)
        *reply = rep;
    return (fnd_status)rep.op;
}

/* ---- streams (§11.3) ---- */

fnd_status fnd_read(fnd_handle h, void *buf, uint32_t len, uint32_t *got) {
    uint32_t done = 0;
    while (done < len) {
        uint32_t chunk = len - done < FND_IPC_MAX_DATA ? len - done : FND_IPC_MAX_DATA;
        uint32_t args[4] = {chunk, 0, 0, 0};
        fnd_msg rep;
        fnd_status status = fnd_call(h, FND_ST_READ, args, 0, 0, (uint8_t *)buf + done, chunk, &rep);
        if (status < 0)
            return status;
        done += rep.len;
        if (rep.len < chunk)
            break; /* EOF, or a device that returned what it had */
    }
    *got = done;
    return FND_OK;
}

fnd_status fnd_write(fnd_handle h, const void *buf, uint32_t len, uint32_t *put) {
    uint32_t done = 0;
    fnd_status status = FND_OK;
    while (done < len) {
        uint32_t chunk = len - done < FND_IPC_MAX_DATA ? len - done : FND_IPC_MAX_DATA;
        fnd_msg rep;
        status = fnd_call(h, FND_ST_WRITE, 0, (const uint8_t *)buf + done, chunk, 0, 0, &rep);
        if (status < 0)
            break;
        done += rep.a0;
        if (rep.a0 < chunk) {
            status = FND_E_IO;
            break;
        }
    }
    if (put)
        *put = done; /* also on failure (§11.3) */
    return status < 0 ? status : FND_OK;
}

fnd_status fnd_stream_stat(fnd_handle h, fnd_stat_t *st) {
    fnd_stat_t got;
    fnd_status status = fnd_call(h, FND_ST_STAT, 0, 0, 0, &got, sizeof got, 0);
    if (status < 0)
        return status;
    *st = got;
    return FND_OK;
}

fnd_status fnd_close(fnd_handle h) { return fnd_sys_handle_close(h); }

/* ---- fs (§11.3, §11.4) ---- */

static fnd_status path_call(uint32_t op, const char *path, const uint32_t args[4], void *rep_data,
                            uint32_t rep_cap, fnd_msg *reply) {
    uint32_t len = str_len(path);
    if (len > FND_IPC_MAX_DATA)
        return FND_E_NAME_TOO_LONG;
    return fnd_call(fnd_fs(), op, args, path, len, rep_data, rep_cap, reply);
}

fnd_status fnd_file_open(const char *path, uint32_t flags, uint32_t mode, fnd_handle *out) {
    uint32_t args[4] = {flags, mode, 0, 0};
    fnd_msg rep;
    fnd_status status = path_call(FND_FS_OPEN, path, args, 0, 0, &rep);
    if (status < 0)
        return status;
    *out = rep.a3;
    return FND_OK;
}

fnd_status fnd_path_stat(const char *path, fnd_stat_t *st) {
    fnd_stat_t got;
    fnd_status status = path_call(FND_FS_STAT, path, 0, &got, sizeof got, 0);
    if (status < 0)
        return status;
    *st = got;
    return FND_OK;
}

fnd_status fnd_cwd_set(const char *path) {
    fnd_status status = path_call(FND_FS_CWD_SET, path, 0, 0, 0, 0);
    return status < 0 ? status : FND_OK;
}

/* The reply needs room for a 4096-byte cwd (§6.3); a smaller buffer gets
 * it through a temporary one, so *len can tell the size it needs. */
fnd_status fnd_cwd_get(char *buf, uint32_t cap, uint32_t *len) {
    fnd_msg rep;
    char *tmp = cap > FND_IPC_MAX_DATA ? buf : fnd_mem_alloc(FND_IPC_MAX_DATA);
    if (!tmp)
        return FND_E_NO_MEMORY;
    fnd_status status = fnd_call(fnd_fs(), FND_FS_CWD_GET, 0, 0, 0, tmp, FND_IPC_MAX_DATA, &rep);
    if (status >= 0) {
        *len = rep.len + 1u;
        status = rep.len + 1u > cap ? FND_E_LIMIT : FND_OK;
        for (uint32_t i = 0; status == FND_OK && i < rep.len; ++i)
            buf[i] = tmp[i];
        if (status == FND_OK)
            buf[rep.len] = 0;
    }
    if (tmp != buf)
        fnd_mem_free(tmp);
    return status < 0 ? status : FND_OK;
}

/* ---- console (§11.5) ---- */

fnd_status fnd_con_write(const char *text) { return fnd_write(fnd_std_out(), text, str_len(text), 0); }

fnd_status fnd_con_getkey(fnd_key_t *key) {
    fnd_msg rep;
    fnd_status status = fnd_call(fnd_console(), FND_CON_READ_KEY, 0, 0, 0, 0, 0, &rep);
    if (status < 0)
        return status;
    key->code = rep.a0;
    key->ch = rep.a1;
    key->mods = rep.a2;
    return FND_OK;
}

fnd_status fnd_con_pollkey(fnd_key_t *key) {
    uint32_t args[4] = {FND_CON_NOWAIT, 0, 0, 0};
    fnd_msg rep;
    fnd_status status = fnd_call(fnd_console(), FND_CON_READ_KEY, args, 0, 0, 0, 0, &rep);
    if (status != FND_OK)
        return status;
    key->code = rep.a0;
    key->ch = rep.a1;
    key->mods = rep.a2;
    return FND_OK;
}

fnd_status fnd_con_set_mode(uint32_t mode) {
    uint32_t args[4] = {mode, 0, 0, 0};
    fnd_status status = fnd_call(fnd_console(), FND_CON_SET_MODE, args, 0, 0, 0, 0, 0);
    return status < 0 ? status : FND_OK;
}

fnd_status fnd_con_clear(void) {
    fnd_status status = fnd_call(fnd_console(), FND_CON_CLEAR, 0, 0, 0, 0, 0, 0);
    return status < 0 ? status : FND_OK;
}

fnd_status fnd_con_size(uint32_t *cols, uint32_t *rows) {
    fnd_msg rep;
    fnd_status status = fnd_call(fnd_console(), FND_CON_GET_SIZE, 0, 0, 0, 0, 0, &rep);
    if (status < 0)
        return status;
    *cols = rep.a0;
    *rows = rep.a1;
    return FND_OK;
}
