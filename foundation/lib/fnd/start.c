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

/* CHANNEL は1..63。キャッシュは next の初回に確保し、close で解放する。
 * raw handle_close はこの状態を解放しないので、列挙には fnd_close を使う。 */
struct dir_cache {
    uint32_t used, pos, end;
    uint8_t data[FND_IPC_MAX_DATA];
};
static struct dir_cache *dirs[64];

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

fnd_status fnd_close(fnd_handle h) {
    fnd_status st = fnd_sys_handle_close(h);
    if (st == FND_OK && h < 64u) {
        fnd_mem_free(dirs[h]);
        dirs[h] = 0;
    }
    return st;
}

fnd_status fnd_sync(fnd_handle h) { return fnd_call(h, FND_ST_SYNC, 0, 0, 0, 0, 0, 0); }

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

fnd_status fnd_file_delete(const char *path) { return path_call(FND_FS_UNLINK, path, 0, 0, 0, 0); }
fnd_status fnd_dir_remove(const char *path) { return path_call(FND_FS_RMDIR, path, 0, 0, 0, 0); }
fnd_status fnd_dir_make(const char *path, uint32_t mode) {
    uint32_t args[4] = {0, mode, 0, 0};
    return path_call(FND_FS_MKDIR, path, args, 0, 0, 0);
}

fnd_status fnd_dir_open(const char *path, fnd_handle *out) {
    fnd_msg rep;
    fnd_status st = path_call(FND_FS_DIR_OPEN, path, 0, 0, 0, &rep);
    if (st == FND_OK) *out = rep.a3;
    return st;
}

static uint32_t dir_u32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

static uint32_t dir_u16(const uint8_t *p) { return ((uint32_t)p[0] << 8) | p[1]; }

static fnd_status dir_error(fnd_handle h, struct dir_cache *c, fnd_status st) {
    /* 初回の失敗ではキャッシュを残さない。再試行用の既存状態は保持する。 */
    if (dirs[h] != c) fnd_mem_free(c);
    return st;
}

fnd_status fnd_dir_next(fnd_handle h, fnd_dirent_t *ent) {
    if (h == 0 || h >= 64u) return FND_E_BAD_HANDLE;
    struct dir_cache *c = dirs[h];
    if (!c) {
        c = fnd_mem_alloc(sizeof *c);
        if (!c) return FND_E_NO_MEMORY;
        c->used = c->pos = c->end = 0;
    }
    if (c->end) return FND_END;
    if (c->pos == c->used) {
        fnd_msg rep;
        fnd_status st = fnd_call(h, FND_DIR_NEXT, 0, 0, 0, c->data, sizeof c->data, &rep);
        if (st != FND_OK) return dir_error(h, c, st);
        /* 応答全体を検証してから1件でも返す。エラー時は ent を変更しない。 */
        if (rep.len > sizeof c->data) return dir_error(h, c, FND_E_FORMAT);
        uint32_t off = 0, count = 0;
        while (off < rep.len) {
            const uint8_t *r = c->data + off;
            if (rep.len - off < FND_DIRENT_HEAD) return dir_error(h, c, FND_E_FORMAT);
            uint32_t n = dir_u32(r), bytes = dir_u16(r + 68), chars = dir_u16(r + 70);
            if (bytes == 0 || bytes > 1024u || chars == 0 || chars > bytes ||
                n != ((FND_DIRENT_HEAD + bytes + 3u) & ~3u) || n > rep.len - off)
                return dir_error(h, c, FND_E_FORMAT);
            for (uint32_t i = 0; i < bytes; ++i)
                if (r[FND_DIRENT_HEAD + i] == 0) return dir_error(h, c, FND_E_FORMAT);
            off += n;
            ++count;
        }
        if (count != rep.a0) return dir_error(h, c, FND_E_FORMAT);
        dirs[h] = c;
        c->pos = 0;
        c->used = rep.len;
        if (!count) {
            c->end = 1;
            return FND_END;
        }
    }
    const uint8_t *r = c->data + c->pos;
    /* C の stat は wire と同じ64-byte配置。 */
    const uint8_t *stat = r + 4;
    for (uint32_t i = 0; i < sizeof ent->stat; ++i)
        ((uint8_t *)&ent->stat)[i] = stat[i];
    ent->name_bytes = dir_u16(r + 68);
    ent->name_chars = dir_u16(r + 70);
    for (uint32_t i = 0; i < ent->name_bytes; ++i) ent->name[i] = (char)r[FND_DIRENT_HEAD + i];
    ent->name[ent->name_bytes] = 0;
    c->pos += dir_u32(r);
    return FND_OK;
}

fnd_status fnd_dir_rewind(fnd_handle h) {
    fnd_status st = fnd_call(h, FND_DIR_REWIND, 0, 0, 0, 0, 0, 0);
    if (st == FND_OK && h < 64u && dirs[h])
        dirs[h]->used = dirs[h]->pos = dirs[h]->end = 0;
    return st;
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
