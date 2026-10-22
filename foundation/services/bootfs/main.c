/* Provisional fs service: a read-only tree linked into this image.
 *
 * Until the real fs service (FS2 on the boot disk) exists, this one
 * registers as "fs" and serves the files that boot/build.py put into
 * bootfs_files.h, so the shell can find /bin/<name>.wax and fnd_proc_exec
 * can read it. It speaks the read-only part of the session and stream
 * protocols of doc/spec_Foundation_API.md §5 and §6: FS_OPEN (READ, EXEC),
 * FS_STAT, FS_CWD_SET, FS_CWD_GET, FS_SESSION_NEW, ST_READ, ST_SEEK,
 * ST_STAT. Writes are FND_E_READ_ONLY. Directories are the parents of the
 * files. Names compare ASCII case-insensitively, a stand-in for the FS2
 * folding (§11.7) that is enough for ASCII file names. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "bootfs_files.h"

#define SESSIONS 16u
#define STREAMS 16u
#define PATH_MAX 255u
#define LOOKUP_BADGE 0x80000000u
#define MODE_FILE 0x003Du /* FS2 default directory mode: R, W, X for the owner */
#define MODE_DIR 0x003Du

struct session {
    uint32_t badge; /* 0: free */
    char cwd[PATH_MAX + 1u];
};
struct stream {
    uint32_t badge;
    uint32_t file;
    uint32_t pos;
};

static struct session sessions[SESSIONS];
static struct stream streams[STREAMS];
static uint32_t next_badge;
static uint8_t data[FND_IPC_MAX_DATA];
static uint8_t out[sizeof(fnd_stat_t)]; /* stat replies */
static char path[PATH_MAX + 1u];

static uint32_t str_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

static uint32_t lower(uint32_t c) { return c >= 'A' && c <= 'Z' ? c + 32u : c; }

static void reply(uint32_t token, int32_t result, uint32_t flags, uint32_t a0, uint32_t a3, const void *buf,
                  uint32_t len) {
    fnd_msg msg = {(uint32_t)result, flags, a0, 0, 0, a3, (uint32_t)(uintptr_t)buf, len};
    fnd_sys_ipc_reply(token, &msg);
}

static uint32_t new_badge(void) {
    next_badge = next_badge >= 0x7FFFFFFFu ? 1u : next_badge + 1u;
    return next_badge;
}

static struct session *find_session(uint32_t badge) {
    for (uint32_t i = 0; i < SESSIONS; ++i) {
        if (sessions[i].badge == badge)
            return &sessions[i];
    }
    return 0;
}

static struct session *add_session(uint32_t badge, const char *cwd) {
    struct session *s = find_session(0);
    if (!s)
        return 0;
    s->badge = badge;
    uint32_t n = str_len(cwd);
    for (uint32_t i = 0; i <= n; ++i)
        s->cwd[i] = cwd[i];
    return s;
}

/* §6.2: a relative path starts at the session's cwd. Builds the absolute
 * path with "." and ".." resolved; FND_E_NAME_TOO_LONG past PATH_MAX. */
static int32_t resolve(const struct session *s, const uint8_t *p, uint32_t len) {
    if (len == 0)
        return FND_E_BAD_PATH;
    uint32_t n = 0;
    if (p[0] != '/') {
        n = str_len(s->cwd);
        for (uint32_t i = 0; i < n; ++i)
            path[i] = s->cwd[i];
        if (n == 1u)
            n = 0; /* the cwd is "/" */
    }
    for (uint32_t i = 0; i < len;) {
        while (i < len && p[i] == '/')
            ++i;
        uint32_t start = i;
        while (i < len && p[i] != '/') {
            if (p[i] == 0)
                return FND_E_BAD_PATH;
            ++i;
        }
        uint32_t c = i - start;
        if (c == 0 || (c == 1 && p[start] == '.'))
            continue;
        if (c == 2 && p[start] == '.' && p[start + 1u] == '.') {
            while (n && path[n - 1u] != '/')
                --n;
            if (n)
                --n;
            continue;
        }
        if (n + 1u + c > PATH_MAX)
            return FND_E_NAME_TOO_LONG;
        path[n++] = '/';
        for (uint32_t k = 0; k < c; ++k)
            path[n++] = (char)p[start + k];
    }
    if (n == 0)
        path[n++] = '/';
    path[n] = 0;
    return FND_OK;
}

/* Whether name (n bytes) equals the first n bytes of entry, ignoring ASCII
 * case. */
static int same(const char *entry, const char *name, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i) {
        if (!entry[i] || lower((uint8_t)entry[i]) != lower((uint8_t)name[i]))
            return 0;
    }
    return 1;
}

/* What path names: a file index (>= 0), DIR, or an error. */
#define IS_DIR 0x7FFFFFFF
static int32_t lookup(void) {
    uint32_t n = str_len(path);
    if (n == 1u)
        return IS_DIR;
    for (uint32_t i = 0; i < BOOTFS_FILES; ++i) {
        const char *f = bootfs_files[i].path;
        if (!same(f, path, n))
            continue;
        if (f[n] == 0)
            return (int32_t)i;
        if (f[n] == '/')
            return IS_DIR;
    }
    /* A file in the middle of the path makes it FND_E_NOT_DIR. */
    for (uint32_t i = 0; i < BOOTFS_FILES; ++i) {
        uint32_t len = str_len(bootfs_files[i].path);
        if (len < n && path[len] == '/' && same(bootfs_files[i].path, path, len))
            return FND_E_NOT_DIR;
    }
    return FND_E_NOT_FOUND;
}

static uint32_t fill_stat(int32_t what) {
    fnd_stat_t st = {0};
    if (what == IS_DIR) {
        st.type = FND_TYPE_DIR;
        st.mode = MODE_DIR;
        st.flags = FND_STAT_READ_ONLY;
    } else {
        st.type = FND_TYPE_FILE;
        st.mode = MODE_FILE;
        st.flags = FND_STAT_READ_ONLY | FND_STAT_SEEKABLE;
        st.size_lo = bootfs_files[what].size;
    }
    const uint8_t *bytes = (const uint8_t *)&st;
    for (uint32_t i = 0; i < sizeof st; ++i)
        out[i] = bytes[i];
    return sizeof st;
}

/* Sends a new channel of port with badge to the caller (XFER_HANDLE). */
static int32_t give_channel(fnd_handle port, uint32_t token, uint32_t badge, uint32_t a0) {
    int32_t ch = fnd_sys_chan_create(port, badge);
    if (ch < 0)
        return ch;
    fnd_msg msg = {FND_OK, FND_MSG_XFER_HANDLE, a0, 0, 0, (uint32_t)ch, 0, 0};
    int32_t status = fnd_sys_ipc_reply(token, &msg);
    if (status != FND_OK)
        fnd_sys_handle_close((uint32_t)ch);
    return status;
}

/* Session operations (§6.3). Returns 1 when it has replied. */
static int session_op(fnd_handle port, const fnd_recv *r, struct session *s, int32_t *result) {
    const fnd_msg *m = &r->msg;
    int32_t what;
    switch (m->op) {
    case FND_FS_OPEN: {
        if ((*result = resolve(s, data, m->len)) != FND_OK)
            return 0;
        uint32_t flags = m->a0;
        if (flags & (FND_OPEN_WRITE | FND_OPEN_CREATE | FND_OPEN_TRUNC | FND_OPEN_APPEND)) {
            *result = FND_E_READ_ONLY;
            return 0;
        }
        if (!(flags & (FND_OPEN_READ | FND_OPEN_EXEC)) || (flags & ~(FND_OPEN_READ | FND_OPEN_EXEC))) {
            *result = FND_E_BAD_ARG;
            return 0;
        }
        what = lookup();
        if (what < 0 || what == IS_DIR) {
            *result = what < 0 ? what : FND_E_IS_DIR;
            return 0;
        }
        struct stream *st = 0;
        for (uint32_t i = 0; i < STREAMS && !st; ++i) {
            if (!streams[i].badge)
                st = &streams[i];
        }
        if (!st) {
            *result = FND_E_LIMIT;
            return 0;
        }
        *st = (struct stream){new_badge(), (uint32_t)what, 0};
        if (give_channel(port, r->token, st->badge, FND_TYPE_FILE) != FND_OK)
            st->badge = 0;
        return 1;
    }
    case FND_FS_STAT:
        if ((*result = resolve(s, data, m->len)) != FND_OK)
            return 0;
        what = lookup();
        if (what < 0) {
            *result = what;
            return 0;
        }
        reply(r->token, FND_OK, 0, 0, 0, out, fill_stat(what));
        return 1;
    case FND_FS_CWD_SET:
        if ((*result = resolve(s, data, m->len)) != FND_OK)
            return 0;
        what = lookup();
        if (what != IS_DIR) {
            *result = what < 0 ? what : FND_E_NOT_DIR;
            return 0;
        }
        for (uint32_t i = 0; i <= str_len(path); ++i)
            s->cwd[i] = path[i];
        return 0;
    case FND_FS_CWD_GET: {
        uint32_t n = str_len(s->cwd);
        if (n > r->reply_cap) {
            *result = FND_E_LIMIT;
            return 0;
        }
        reply(r->token, FND_OK, 0, 0, 0, s->cwd, n);
        return 1;
    }
    case FND_FS_SESSION_NEW: {
        uint32_t badge = new_badge();
        struct session *child = add_session(badge, s->cwd);
        if (!child) {
            *result = FND_E_LIMIT;
            return 0;
        }
        if (give_channel(port, r->token, badge, 0) != FND_OK)
            child->badge = 0;
        return 1;
    }
    case FND_FS_MKDIR: case FND_FS_UNLINK: case FND_FS_RMDIR: case FND_FS_RENAME:
    case FND_FS_CHMOD: case FND_FS_CHOWN:
        *result = FND_E_READ_ONLY;
        return 0;
    case FND_FS_SYNC:
        return 0;
    default:
        *result = FND_E_UNSUPPORTED; /* FS_DIR_OPEN, FS_VOLUME_INFO: not yet */
        return 0;
    }
}

/* Stream operations (§5) on an open file. */
static int stream_op(const fnd_recv *r, struct stream *st, int32_t *result, uint32_t *a0, uint32_t *a1) {
    const fnd_msg *m = &r->msg;
    uint32_t size = bootfs_files[st->file].size;
    switch (m->op) {
    case FND_ST_READ: {
        if (m->a0 == 0 || m->a0 > FND_IPC_MAX_DATA) {
            *result = FND_E_BAD_ARG;
            return 0;
        }
        uint32_t n = m->a0 < r->reply_cap ? m->a0 : r->reply_cap;
        if (n > size - st->pos)
            n = size - st->pos;
        /* straight from the image: the kernel copies it by words */
        const uint8_t *file = (const uint8_t *)bootfs_files[st->file].data;
        reply(r->token, FND_OK, 0, 0, 0, file + st->pos, n);
        st->pos += n;
        return 1;
    }
    case FND_ST_SEEK: {
        int32_t base = m->a0 == 0 ? 0 : m->a0 == 1 ? (int32_t)st->pos : m->a0 == 2 ? (int32_t)size : -1;
        int32_t to = base + (int32_t)m->a2;
        if (m->a0 > 2u || m->a1 != (m->a2 & 0x80000000u ? 0xFFFFFFFFu : 0) || to < 0) {
            *result = FND_E_BAD_ARG;
            return 0;
        }
        st->pos = (uint32_t)to < size ? (uint32_t)to : size;
        *a0 = 0;
        *a1 = st->pos;
        return 0;
    }
    case FND_ST_STAT:
        reply(r->token, FND_OK, 0, 0, 0, out, fill_stat((int32_t)st->file));
        return 1;
    case FND_ST_SYNC:
        return 0;
    case FND_ST_WRITE: case FND_ST_TRUNCATE:
        *result = FND_E_ACCESS; /* opened for reading only */
        return 0;
    default:
        *result = FND_E_UNSUPPORTED;
        return 0;
    }
}

static void client_gone(uint32_t badge) {
    struct session *s = find_session(badge);
    if (s)
        s->badge = 0;
    for (uint32_t i = 0; i < STREAMS; ++i) {
        if (streams[i].badge == badge)
            streams[i].badge = 0;
    }
}

int main(void) {
    fnd_init();
    int32_t port = fnd_sys_port_create();
    if (port < 0 || fnd_sys_svc_register("fs", (uint32_t)port) != FND_OK)
        fnd_exit(FND_E_BAD_HANDLE);
    for (;;) {
        fnd_recv r = {{0}, (uint32_t)(uintptr_t)data, sizeof data, 0, 0, 0, 0, FND_FOREVER, 0};
        if (fnd_sys_ipc_recv((uint32_t)port, &r) != FND_OK)
            continue;
        if (r.token == 0) {
            if (r.msg.op == FND_NOTIFY_CLIENT_GONE)
                client_gone(r.badge);
            continue;
        }
        int32_t result = FND_OK;
        uint32_t a0 = 0, a1 = 0;
        int answered = 0;
        struct session *s = find_session(r.badge);
        struct stream *st = 0;
        for (uint32_t i = 0; i < STREAMS && !s && !st; ++i) {
            if (streams[i].badge == r.badge)
                st = &streams[i];
        }
        /* §6.1: a channel from svc_lookup is a new session at /. */
        if (!s && !st && (r.badge & LOOKUP_BADGE))
            s = add_session(r.badge, "/");
        if (r.msg.len > r.total || r.total > sizeof data)
            result = FND_E_NAME_TOO_LONG;
        else if (s)
            answered = session_op((fnd_handle)port, &r, s, &result);
        else if (st)
            answered = stream_op(&r, st, &result, &a0, &a1);
        else
            result = (r.badge & LOOKUP_BADGE) ? FND_E_LIMIT : FND_E_BAD_HANDLE;
        if (!answered) {
            fnd_msg msg = {(uint32_t)result, 0, a0, a1, 0, 0, 0, 0};
            fnd_sys_ipc_reply(r.token, &msg);
        }
    }
}
