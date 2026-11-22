/* netfs: the Foundation client of the CNET Filing Protocol
 * (doc/spec_NET_FP.md §11), a boot module with FND_CAP_SVC like fs and pipe
 * (registering a service needs the capability, which no program started
 * from the shell has). The command /bin/netfs asks it to mount and unmount
 * (FND_NET_MOUNT, FND_NET_UMOUNT, FND_NET_LIST).
 *
 * The service keeps one L3P transaction association (lib/l3p, linked in)
 * and CFP session per mount, with the export's root at fid 1. fs forwards
 * every path operation below /net/<name> to it (FND_NET_FS + the low byte
 * of the FS_* operation, the path below /net as data), and it tells fs which
 * names are mounted (FND_NET_SET). Open files and directories are CHANNELs
 * to the service's own PORT, so their reads, writes and listings come here
 * directly, like a device service's streams (API §9).
 *
 * One thread serves everything in order, so a slow server delays the other
 * mounts too; fs itself keeps serving local paths (its workers wait for
 * us). When an association breaks, the open objects of that session give
 * FND_E_STALE and the next path operation opens a new session (§11.2). The
 * association state (about 9 KiB) is allocated only while mounted. */
#include <foundation/l3p.h>
#include <foundation/sys.h>

#define CFP_SOCKET 9u
#define CFP_VERSION 1u
#define MOUNTS 4u
#define OBJECTS 32u
#define IOUNIT 960u
#define BODY_MAX 980u
#define CFP_NAME_MAX 900u
#define WALK_MAX 16u
#define COMPS_MAX 256u
#define RENAME_NAMES_MAX 968u
#define ROOT_FID 1u
#define POLL_TICKS 10u /* L3P timers and keep-alive while a mount is up */
#define KERNEL_BADGE 0x80000000u

/* CFP message types (§3.3) and fields. */
#define T_HELLO 0x01u
#define T_EXPORTS 0x02u
#define T_ATTACH 0x10u
#define T_WALK 0x11u
#define T_CLUNK 0x12u
#define T_OPEN 0x13u
#define T_CREATE 0x14u
#define T_READ 0x15u
#define T_WRITE 0x16u
#define T_READDIR 0x17u
#define T_STAT 0x18u
#define T_SETATTR 0x19u
#define T_REMOVE 0x1Au
#define T_RENAME 0x1Bu
#define T_SYNC 0x1Cu
#define T_STATFS 0x1Du
#define R_ERROR 0xFFu
#define QID_DIR 2u
#define KIND_FILE 1u
#define KIND_DIR 2u
#define SET_MODE 1u
#define SET_UID 2u
#define SET_SIZE 4u

static uint32_t rd16(const uint8_t *p) { return (uint32_t)p[0] << 8 | p[1]; }
static uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}
static uint64_t rd64(const uint8_t *p) { return (uint64_t)rd32(p) << 32 | rd32(p + 4); }
static void wr16(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 8);
    p[1] = (uint8_t)v;
}
static void wr32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >> 8);
    p[3] = (uint8_t)v;
}
static void wr64(uint8_t *p, uint64_t v) {
    wr32(p, (uint32_t)(v >> 32));
    wr32(p + 4, (uint32_t)v);
}
static void copy(void *dst, const void *src, uint32_t n) {
    uint8_t *d = (uint8_t *)dst;
    const uint8_t *s = (const uint8_t *)src;
    while (n--) *d++ = *s++;
}
static void zero(void *dst, uint32_t n) {
    uint8_t *d = (uint8_t *)dst;
    while (n--) *d++ = 0;
}
static uint32_t str_len(const char *s) {
    uint32_t n = 0;
    while (s[n]) ++n;
    return n;
}

/* Mount and export names: ASCII lower case, digits, '_' and '-' (§5). */
static int good_name(const uint8_t *s, uint32_t n) {
    if (n == 0 || n > FND_NET_NAME_MAX) return 0;
    for (uint32_t i = 0; i < n; ++i) {
        uint8_t c = s[i];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-')) return 0;
    }
    return 1;
}

typedef struct {
    uint32_t used, up;
    uint32_t addr, flags; /* FND_NET_MOUNT_* */
    uint32_t gen;         /* increases with every new session */
    uint32_t next_fid, iounit, remote_ro;
    char name[FND_NET_NAME_MAX + 1], export_name[FND_NET_NAME_MAX + 1];
    l3p_assoc *assoc; /* allocated while mounted */
} mount_t;

#define OBJ_FREE 0u
#define OBJ_FILE 1u
#define OBJ_DIR 2u

typedef struct {
    uint32_t kind, badge;
    uint32_t mount, gen, fid, flags;
    uint64_t pos;
    uint32_t cookie, end;
} obj_t;

typedef struct {
    const uint8_t *p;
    uint32_t len;
} comp_t;

static mount_t mounts[MOUNTS];
static obj_t objs[OBJECTS];
static uint32_t next_badge, service_port;
static uint8_t tx[1024] __attribute__((aligned(4)));
static uint8_t rx[1024] __attribute__((aligned(4)));
static uint8_t req_data[FND_IPC_MAX_DATA] __attribute__((aligned(4)));
static uint8_t reply_buf[FND_IPC_MAX_DATA] __attribute__((aligned(4)));
static comp_t comps_a[COMPS_MAX], comps_b[COMPS_MAX];

static int mount_ro(const mount_t *m) { return (m->flags & FND_NET_MOUNT_RO) || m->remote_ro; }

/* The association is gone: TIMEOUT if the server stopped answering, else
 * STALE (§11.2). */
static fnd_status mount_fail(mount_t *m, fnd_status st) {
    l3p_abort(m->assoc);
    m->up = 0;
    return st == FND_E_TIMEOUT ? FND_E_TIMEOUT : FND_E_STALE;
}

static fnd_status cfp_result(uint32_t type, uint32_t rlen, uint32_t *blen);

/* One request in tx (header filled here) of 4 + body bytes. The response
 * body is rx + 4, *blen bytes. A CFP error comes back as its status. */
static fnd_status cfp_call(mount_t *m, uint32_t type, uint32_t body, uint32_t *blen) {
    uint32_t rlen = 0;
    tx[0] = (uint8_t)type;
    tx[1] = tx[2] = tx[3] = 0;
    fnd_status st = l3p_txn_call(m->assoc, tx, 4u + body, rx, sizeof rx, &rlen);
    if (st != FND_OK) return mount_fail(m, st);
    return cfp_result(type, rlen, blen);
}

/* Checks the response in rx to a request of `type`. */
static fnd_status cfp_result(uint32_t type, uint32_t rlen, uint32_t *blen) {
    if (rlen < 4u) return FND_E_IO;
    if (rx[0] == R_ERROR) {
        int32_t status = rlen >= 8u ? (int32_t)rd32(rx + 4) : FND_E_IO;
        return status < 0 ? status : FND_E_IO;
    }
    if (rx[0] != (uint8_t)(type | 0x80u)) return FND_E_IO;
    if (blen) *blen = rlen - 4u;
    return FND_OK;
}

static uint32_t new_fid(mount_t *m) {
    uint32_t f = m->next_fid++;
    if (m->next_fid == 0xFFFFFFFFu) m->next_fid = ROOT_FID + 1u;
    return f;
}

static fnd_status cfp_fid_op(mount_t *m, uint32_t type, uint32_t fid, uint32_t *blen) {
    wr32(tx + 4, fid);
    return cfp_call(m, type, 4, blen);
}

static void clunk(mount_t *m, uint32_t fid) {
    if (m->up) cfp_fid_op(m, T_CLUNK, fid, 0);
}

/* Opens the session: L3P association, HELLO, ATTACH of the export at fid 1. */
static fnd_status mount_up(mount_t *m) {
    if (m->up) return FND_OK;
    uint32_t opts = m->flags & FND_NET_MOUNT_ENC ? L3P_OPT_ENC : 0;
    fnd_status st = l3p_open(m->addr, CFP_SOCKET, L3P_KIND_TXN, opts, m->assoc);
    if (st != FND_OK) return st;
    m->up = 1;
    m->next_fid = ROOT_FID + 1u;
    uint32_t blen;
    static const char client[] = "foundation";
    wr32(tx + 4, CFP_VERSION);
    wr16(tx + 8, sizeof client - 1u);
    wr16(tx + 10, 0);
    copy(tx + 12, client, sizeof client - 1u);
    st = cfp_call(m, T_HELLO, 8u + sizeof client - 1u, &blen);
    if (st == FND_OK && (blen < 16u || rd32(rx + 4) != CFP_VERSION)) st = FND_E_UNSUPPORTED;
    if (st == FND_OK) {
        uint32_t io = rd16(rx + 4 + 10);
        m->iounit = io && io < IOUNIT ? io : IOUNIT;
        uint32_t n = str_len(m->export_name);
        wr32(tx + 4, ROOT_FID);
        wr32(tx + 8, 0); /* UID 0 (§10) */
        wr16(tx + 12, n);
        wr16(tx + 14, 0);
        copy(tx + 16, m->export_name, n);
        st = cfp_call(m, T_ATTACH, 12u + n, &blen);
        if (st == FND_OK && blen < 20u) st = FND_E_IO;
    }
    if (st != FND_OK) {
        if (m->up) {
            l3p_close(m->assoc);
            m->up = 0;
        }
        return st;
    }
    m->remote_ro = rd32(rx + 4 + 16) & 1u;
    ++m->gen;
    return FND_OK;
}

/* WALK from `from` through n components to a new fid (16 per message,
 * §11.2); n = 0 clones. *qtype is the type of the last qid (a clone is
 * taken to be a directory: it is only made of directories here). */
static fnd_status walk(mount_t *m, uint32_t from, const comp_t *c, uint32_t n, uint32_t newfid, uint32_t *qtype) {
    uint32_t done = 0, have = 0;
    *qtype = QID_DIR;
    do {
        uint32_t k = 0, at = 4u + 12u;
        while (done + k < n && k < WALK_MAX) {
            uint32_t len = c[done + k].len;
            if (len > CFP_NAME_MAX) {
                if (have) clunk(m, newfid);
                return FND_E_NAME_TOO_LONG;
            }
            if (at + 2u + len > 4u + BODY_MAX) break;
            wr16(tx + at, len);
            copy(tx + at + 2u, c[done + k].p, len);
            at += 2u + len;
            ++k;
        }
        wr32(tx + 4, have ? newfid : from);
        wr32(tx + 8, newfid);
        wr16(tx + 12, k);
        wr16(tx + 14, 0);
        uint32_t blen;
        fnd_status st = cfp_call(m, T_WALK, at - 4u, &blen);
        if (st == FND_OK) {
            uint32_t nq = blen >= 8u ? rd16(rx + 4) : 0;
            int32_t status = blen >= 8u ? (int32_t)rd32(rx + 8) : FND_E_IO;
            if (nq < k || status != FND_OK || blen < 8u + 16u * k)
                st = status < 0 ? status : FND_E_IO;
            else if (k)
                *qtype = rx[4 + 8 + 16u * (k - 1u)];
        }
        if (st != FND_OK) {
            if (have) clunk(m, newfid);
            return st;
        }
        have = 1;
        done += k;
    } while (done < n);
    return FND_OK;
}

static int name_is(const uint8_t *p, uint32_t n, const char *s) {
    uint32_t i = 0;
    for (; i < n && s[i]; ++i) {
        uint8_t ch = p[i];
        if (ch >= 'A' && ch <= 'Z') ch = (uint8_t)(ch + 32u);
        if (ch != (uint8_t)s[i]) return 0;
    }
    return i == n && !s[i];
}

/* "name/a/b[/]" from fs: the mount, the components below its root, and
 * whether a trailing '/' asked for a directory. */
static fnd_status parse_path(const uint8_t *p, uint32_t len, mount_t **mp, comp_t *c, uint32_t *n,
                             uint32_t *want_dir) {
    uint32_t i = 0, count = 0;
    while (i < len && p[i] != '/') ++i;
    *mp = 0;
    for (uint32_t k = 0; k < MOUNTS; ++k)
        if (mounts[k].used && name_is(p, i, mounts[k].name)) *mp = &mounts[k];
    if (!*mp) return FND_E_NOT_FOUND;
    *want_dir = len > i && p[len - 1] == '/';
    for (;;) {
        while (i < len && p[i] == '/') ++i;
        if (i >= len) break;
        uint32_t s = i;
        while (i < len && p[i] != '/') ++i;
        uint32_t cl = i - s;
        if ((cl == 1 && p[s] == '.') || (cl == 2 && p[s] == '.' && p[s + 1] == '.')) return FND_E_BAD_PATH;
        if (count >= COMPS_MAX) return FND_E_NAME_TOO_LONG;
        c[count].p = p + s;
        c[count].len = cl;
        ++count;
    }
    *n = count;
    return FND_OK;
}

/* ---- objects ---- */

static obj_t *obj_find(uint32_t badge) {
    for (uint32_t i = 0; i < OBJECTS; ++i)
        if (objs[i].kind != OBJ_FREE && objs[i].badge == badge) return &objs[i];
    return 0;
}

static obj_t *obj_new(uint32_t kind, mount_t *m, uint32_t fid) {
    for (uint32_t i = 0; i < OBJECTS; ++i) {
        if (objs[i].kind != OBJ_FREE) continue;
        do
            next_badge = (next_badge % 0x7FFFFFFFu) + 1u;
        while (obj_find(next_badge));
        zero(&objs[i], sizeof objs[i]);
        objs[i].kind = kind;
        objs[i].badge = next_badge;
        objs[i].mount = (uint32_t)(m - mounts);
        objs[i].gen = m->gen;
        objs[i].fid = fid;
        return &objs[i];
    }
    return 0;
}

/* Replies with a CHANNEL for a new object, or undoes it. */
static fnd_status obj_reply(obj_t *o, mount_t *m, fnd_msg *rep) {
    int32_t h = fnd_sys_chan_create(service_port, o->badge);
    if (h < 0) {
        clunk(m, o->fid);
        zero(o, sizeof *o);
        return h;
    }
    rep->flags = FND_MSG_XFER_HANDLE;
    rep->a3 = (uint32_t)h;
    return FND_OK;
}

static void obj_gone(uint32_t badge) {
    obj_t *o = obj_find(badge);
    if (!o) return;
    mount_t *m = &mounts[o->mount];
    if (m->used && m->up && o->gen == m->gen) clunk(m, o->fid);
    zero(o, sizeof *o);
}

/* ---- path operations from fs (§11.2) ---- */

static fnd_status walk_to(mount_t *m, const comp_t *c, uint32_t n, uint32_t *fid, uint32_t *qtype) {
    *fid = new_fid(m);
    return walk(m, ROOT_FID, c, n, *fid, qtype);
}

/* STAT, SYNC, STATFS and SETATTR may use the root fid itself. */
static fnd_status walk_or_root(mount_t *m, const comp_t *c, uint32_t n, uint32_t *fid, uint32_t *qtype) {
    if (n == 0) {
        *fid = ROOT_FID;
        *qtype = QID_DIR;
        return FND_OK;
    }
    return walk_to(m, c, n, fid, qtype);
}

static void done_with(mount_t *m, uint32_t fid) {
    if (fid != ROOT_FID) clunk(m, fid);
}

static fnd_status cfp_open(mount_t *m, uint32_t fid, uint32_t flags) {
    wr32(tx + 4, fid);
    wr32(tx + 8, flags);
    uint32_t blen;
    return cfp_call(m, T_OPEN, 8, &blen);
}

/* FND_OPEN_* that CFP OPEN takes (§6.4). */
#define OPEN_PASS (FND_OPEN_READ | FND_OPEN_WRITE | FND_OPEN_TRUNC | FND_OPEN_APPEND | FND_OPEN_EXEC)

static fnd_status op_open(mount_t *m, const comp_t *c, uint32_t n, uint32_t want_dir, uint32_t flags, uint32_t mode,
                          fnd_msg *rep) {
    uint32_t fid, pfid, qt;
    fnd_status st;
    if (flags & ~0x7Fu || !(flags & (FND_OPEN_READ | FND_OPEN_WRITE | FND_OPEN_EXEC)) ||
        ((flags & FND_OPEN_EXEC) && (flags & FND_OPEN_WRITE)) ||
        ((flags & FND_OPEN_EXCL) && !(flags & FND_OPEN_CREATE)) ||
        ((flags & (FND_OPEN_TRUNC | FND_OPEN_APPEND)) && !(flags & FND_OPEN_WRITE)))
        return FND_E_BAD_ARG;
    if (n == 0) /* the mount point */
        return (flags & FND_OPEN_EXCL) ? FND_E_EXISTS : FND_E_IS_DIR;
    if ((flags & FND_OPEN_WRITE) && mount_ro(m)) return FND_E_READ_ONLY;
    if (!(flags & FND_OPEN_CREATE)) {
        if ((st = walk_to(m, c, n, &fid, &qt)) != FND_OK) return st;
    } else {
        if ((st = walk_to(m, c, n - 1u, &pfid, &qt)) != FND_OK) return st;
        for (uint32_t attempt = 0;; ++attempt) {
            fid = new_fid(m);
            st = walk(m, pfid, &c[n - 1u], 1, fid, &qt);
            if (st == FND_OK) {
                clunk(m, pfid);
                if (flags & FND_OPEN_EXCL) {
                    clunk(m, fid);
                    return FND_E_EXISTS;
                }
                break;
            }
            if (st != FND_E_NOT_FOUND || want_dir || mount_ro(m)) {
                clunk(m, pfid);
                return st == FND_E_NOT_FOUND && mount_ro(m) && !want_dir ? FND_E_READ_ONLY : st;
            }
            uint32_t len = c[n - 1u].len;
            if (len > CFP_NAME_MAX) {
                clunk(m, pfid);
                return FND_E_NAME_TOO_LONG;
            }
            wr32(tx + 4, pfid);
            tx[8] = KIND_FILE;
            tx[9] = 0;
            wr16(tx + 10, len);
            wr32(tx + 12, flags & (FND_OPEN_READ | FND_OPEN_WRITE | FND_OPEN_APPEND));
            wr32(tx + 16, mode);
            copy(tx + 20, c[n - 1u].p, len);
            uint32_t blen;
            st = cfp_call(m, T_CREATE, 16u + len, &blen);
            if (st == FND_OK) { /* pfid is the open file now */
                obj_t *o = obj_new(OBJ_FILE, m, pfid);
                if (!o) {
                    clunk(m, pfid);
                    return FND_E_LIMIT;
                }
                o->flags = flags;
                rep->a0 = FND_TYPE_FILE;
                return obj_reply(o, m, rep);
            }
            if (st != FND_E_EXISTS || attempt) { /* EXISTS: someone made it; walk again once */
                clunk(m, pfid);
                return st;
            }
        }
    }
    if (qt == QID_DIR || want_dir) {
        clunk(m, fid);
        return qt == QID_DIR ? FND_E_IS_DIR : FND_E_NOT_DIR;
    }
    if ((st = cfp_open(m, fid, flags & OPEN_PASS)) != FND_OK) {
        clunk(m, fid);
        return st;
    }
    obj_t *o = obj_new(OBJ_FILE, m, fid);
    if (!o) {
        clunk(m, fid);
        return FND_E_LIMIT;
    }
    o->flags = flags;
    rep->a0 = FND_TYPE_FILE;
    return obj_reply(o, m, rep);
}

/* STAT of fid into reply_buf (64 bytes, fnd_stat), the mount's read-only
 * setting included. */
static fnd_status stat_fid(mount_t *m, uint32_t fid) {
    uint32_t blen;
    fnd_status st = cfp_fid_op(m, T_STAT, fid, &blen);
    if (st != FND_OK) return st;
    if (blen < 16u + FND_STAT_BYTES) return FND_E_IO;
    copy(reply_buf, rx + 4 + 16, FND_STAT_BYTES);
    if (mount_ro(m)) reply_buf[15] |= FND_STAT_READ_ONLY;
    return FND_OK;
}

static fnd_status op_stat(mount_t *m, const comp_t *c, uint32_t n, uint32_t want_dir, fnd_msg *rep) {
    uint32_t fid, qt;
    fnd_status st = walk_or_root(m, c, n, &fid, &qt);
    if (st != FND_OK) return st;
    st = want_dir && qt != QID_DIR ? FND_E_NOT_DIR : stat_fid(m, fid);
    done_with(m, fid);
    if (st == FND_OK) {
        rep->buf = (uint32_t)(uintptr_t)reply_buf;
        rep->len = FND_STAT_BYTES;
    }
    return st;
}

static fnd_status op_dir_open(mount_t *m, const comp_t *c, uint32_t n, fnd_msg *rep) {
    uint32_t fid, qt;
    fnd_status st = walk_to(m, c, n, &fid, &qt);
    if (st != FND_OK) return st;
    if (qt != QID_DIR) st = FND_E_NOT_DIR;
    else st = cfp_open(m, fid, FND_OPEN_READ);
    if (st != FND_OK) {
        clunk(m, fid);
        return st;
    }
    obj_t *o = obj_new(OBJ_DIR, m, fid);
    if (!o) {
        clunk(m, fid);
        return FND_E_LIMIT;
    }
    return obj_reply(o, m, rep);
}

static fnd_status op_mkdir(mount_t *m, const comp_t *c, uint32_t n, uint32_t mode) {
    uint32_t pfid, qt;
    if (n == 0) return FND_E_EXISTS;
    if (mount_ro(m)) return FND_E_READ_ONLY;
    uint32_t len = c[n - 1u].len;
    if (len > CFP_NAME_MAX) return FND_E_NAME_TOO_LONG;
    fnd_status st = walk_to(m, c, n - 1u, &pfid, &qt);
    if (st != FND_OK) return st;
    wr32(tx + 4, pfid);
    tx[8] = KIND_DIR;
    tx[9] = 0;
    wr16(tx + 10, len);
    wr32(tx + 12, 0);
    wr32(tx + 16, mode);
    copy(tx + 20, c[n - 1u].p, len);
    uint32_t blen;
    st = cfp_call(m, T_CREATE, 16u + len, &blen);
    clunk(m, pfid); /* the parent, or the new directory */
    return st;
}

static fnd_status op_remove(mount_t *m, const comp_t *c, uint32_t n, uint32_t want_dir, uint32_t rmdir) {
    uint32_t fid, qt;
    if (n == 0) return FND_E_BUSY;
    if (mount_ro(m)) return FND_E_READ_ONLY;
    fnd_status st = walk_to(m, c, n, &fid, &qt);
    if (st != FND_OK) return st;
    if (!rmdir && want_dir && qt != QID_DIR) {
        clunk(m, fid);
        return FND_E_NOT_DIR;
    }
    wr32(tx + 4, fid);
    wr32(tx + 8, rmdir ? 1u : 0u);
    uint32_t blen;
    return cfp_call(m, T_REMOVE, 8, &blen); /* the fid is gone either way */
}

static fnd_status op_rename(const uint8_t *data, uint32_t len, uint32_t split) {
    mount_t *ma, *mb;
    uint32_t na, nb, wa, wb, fa, fb, qt;
    if (split == 0 || split >= len) return FND_E_BAD_PATH;
    fnd_status st = parse_path(data, split, &ma, comps_a, &na, &wa);
    if (st == FND_OK) st = parse_path(data + split, len - split, &mb, comps_b, &nb, &wb);
    if (st != FND_OK) return st;
    if (ma != mb) return FND_E_CROSS_DEVICE;
    if (na == 0) return FND_E_BUSY;
    if (nb == 0) return FND_E_EXISTS;
    if ((st = mount_up(ma)) != FND_OK) return st;
    if (mount_ro(ma)) return FND_E_READ_ONLY;
    const comp_t *oc = &comps_a[na - 1u], *nc = &comps_b[nb - 1u];
    if (oc->len > CFP_NAME_MAX || nc->len > CFP_NAME_MAX || oc->len + nc->len > RENAME_NAMES_MAX)
        return FND_E_NAME_TOO_LONG;
    if ((st = walk_to(ma, comps_a, na - 1u, &fa, &qt)) != FND_OK) return st;
    if ((st = walk_to(ma, comps_b, nb - 1u, &fb, &qt)) != FND_OK) {
        clunk(ma, fa);
        return st;
    }
    wr32(tx + 4, fa);
    wr32(tx + 8, fb);
    wr16(tx + 12, oc->len);
    wr16(tx + 14, nc->len);
    copy(tx + 16, oc->p, oc->len);
    copy(tx + 16 + oc->len, nc->p, nc->len);
    uint32_t blen;
    st = cfp_call(ma, T_RENAME, 12u + oc->len + nc->len, &blen);
    clunk(ma, fa);
    clunk(ma, fb);
    return st;
}

static fnd_status setattr(mount_t *m, uint32_t fid, uint32_t mask, uint32_t mode, uint32_t uid, uint64_t size) {
    zero(tx + 4, 36);
    wr32(tx + 4, fid);
    wr32(tx + 8, mask);
    wr32(tx + 12, mode);
    wr32(tx + 16, uid);
    wr64(tx + 20, size);
    uint32_t blen;
    return cfp_call(m, T_SETATTR, 36, &blen);
}

static fnd_status op_attr(mount_t *m, const comp_t *c, uint32_t n, uint32_t op, uint32_t value) {
    uint32_t fid, qt;
    if (mount_ro(m)) return FND_E_READ_ONLY;
    fnd_status st = walk_or_root(m, c, n, &fid, &qt);
    if (st != FND_OK) return st;
    st = op == FND_FS_CHMOD ? setattr(m, fid, SET_MODE, value, 0, 0) : setattr(m, fid, SET_UID, 0, value, 0);
    done_with(m, fid);
    return st;
}

static fnd_status op_sync(mount_t *m, const comp_t *c, uint32_t n) {
    uint32_t fid, qt;
    fnd_status st = walk_or_root(m, c, n, &fid, &qt);
    if (st != FND_OK) return st;
    st = cfp_fid_op(m, T_SYNC, fid, 0);
    done_with(m, fid);
    return st;
}

static fnd_status op_volume(mount_t *m, fnd_msg *rep) {
    uint32_t blen;
    fnd_status st = cfp_fid_op(m, T_STATFS, ROOT_FID, &blen);
    if (st != FND_OK) return st;
    if (blen < FND_VOLINFO_BYTES) return FND_E_IO;
    copy(reply_buf, rx + 4, FND_VOLINFO_BYTES);
    wr32(reply_buf, FND_VOL_NET); /* §11.2 */
    if (mount_ro(m)) reply_buf[7] |= FND_VOL_READ_ONLY;
    wr32(reply_buf + 24, FND_VOL_NO_SLOT);
    rep->buf = (uint32_t)(uintptr_t)reply_buf;
    rep->len = FND_VOLINFO_BYTES;
    return FND_OK;
}

static fnd_status fs_op(const fnd_msg *q, const uint8_t *data, uint32_t len, fnd_msg *rep) {
    uint32_t op = FND_FS_OPEN + (q->op - FND_NET_FS);
    if (op == FND_FS_RENAME) {
        if (q->a1 || q->a2 || q->a3) return FND_E_BAD_ARG;
        return op_rename(data, len, q->a0);
    }
    mount_t *m;
    uint32_t n, want_dir;
    fnd_status st = parse_path(data, len, &m, comps_a, &n, &want_dir);
    if (st != FND_OK) return st;
    if ((st = mount_up(m)) != FND_OK) return st;
    switch (op) {
    case FND_FS_OPEN:
        return op_open(m, comps_a, n, want_dir, q->a0, q->a1, rep);
    case FND_FS_STAT:
        return op_stat(m, comps_a, n, want_dir, rep);
    case FND_FS_DIR_OPEN:
        return op_dir_open(m, comps_a, n, rep);
    case FND_FS_MKDIR:
        return op_mkdir(m, comps_a, n, q->a1);
    case FND_FS_UNLINK:
    case FND_FS_RMDIR:
        return op_remove(m, comps_a, n, want_dir, op == FND_FS_RMDIR);
    case FND_FS_CHMOD:
    case FND_FS_CHOWN:
        return op_attr(m, comps_a, n, op, q->a0);
    case FND_FS_SYNC:
        return op_sync(m, comps_a, n);
    case FND_FS_VOLUME_INFO:
        return op_volume(m, rep);
    default:
        return FND_E_UNSUPPORTED;
    }
}

/* ---- streams of open files and directories ---- */

/* READ of up to `want` bytes at o->pos into reply_buf: one request per
 * IOUNIT, up to the L3P window in flight (§11.2). Short only at EOF. */
static fnd_status read_pipe(obj_t *o, mount_t *m, uint32_t want, uint32_t *got) {
    uint32_t io = m->iounit, n = (want + io - 1u) / io, sent = 0, done = 0, eof = 0;
    uint32_t xids[8];
    fnd_status err = FND_OK;
    *got = 0;
    while (done < sent || (sent < n && !eof && err == FND_OK)) {
        while (sent < n && sent - done < L3P_TXN_WINDOW && !eof && err == FND_OK) {
            uint32_t count = want - sent * io < io ? want - sent * io : io;
            tx[0] = T_READ;
            tx[1] = tx[2] = tx[3] = 0;
            wr32(tx + 4, o->fid);
            wr64(tx + 8, o->pos + (uint64_t)sent * io);
            wr16(tx + 16, count);
            wr16(tx + 18, 0);
            fnd_status st = l3p_txn_send(m->assoc, tx, 20, &xids[sent]);
            if (st == FND_E_WOULD_BLOCK) break;
            if (st != FND_OK) return mount_fail(m, st);
            ++sent;
        }
        uint32_t rlen, blen;
        fnd_status st = l3p_txn_wait(m->assoc, xids[done], rx, sizeof rx, &rlen);
        if (st != FND_OK) return mount_fail(m, st);
        uint32_t asked = want - done * io < io ? want - done * io : io;
        ++done;
        if (eof || err != FND_OK) continue; /* drained */
        if ((st = cfp_result(T_READ, rlen, &blen)) != FND_OK) {
            err = st;
            continue;
        }
        uint32_t count = blen >= 4u ? rd16(rx + 4) : 0xFFFFFFFFu;
        if (count > asked || blen < 4u + count) {
            err = FND_E_IO;
            continue;
        }
        copy(reply_buf + *got, rx + 8, count);
        *got += count;
        if (count < asked) eof = 1;
    }
    return err;
}

static fnd_status write_pipe(obj_t *o, mount_t *m, const uint8_t *data, uint32_t len) {
    uint32_t io = m->iounit, n = (len + io - 1u) / io, sent = 0, done = 0;
    uint32_t xids[8];
    uint64_t end = o->pos;
    fnd_status err = FND_OK;
    while (done < sent || (sent < n && err == FND_OK)) {
        while (sent < n && sent - done < L3P_TXN_WINDOW && err == FND_OK) {
            uint32_t count = len - sent * io < io ? len - sent * io : io;
            tx[0] = T_WRITE;
            tx[1] = tx[2] = tx[3] = 0;
            wr32(tx + 4, o->fid);
            wr64(tx + 8, o->pos + (uint64_t)sent * io);
            wr16(tx + 16, count);
            wr16(tx + 18, 0);
            copy(tx + 20, data + sent * io, count);
            fnd_status st = l3p_txn_send(m->assoc, tx, 20u + count, &xids[sent]);
            if (st == FND_E_WOULD_BLOCK) break;
            if (st != FND_OK) return mount_fail(m, st);
            ++sent;
        }
        uint32_t rlen, blen;
        fnd_status st = l3p_txn_wait(m->assoc, xids[done], rx, sizeof rx, &rlen);
        if (st != FND_OK) return mount_fail(m, st);
        ++done;
        if (err != FND_OK) continue;
        if ((st = cfp_result(T_WRITE, rlen, &blen)) != FND_OK) err = st;
        else if (blen < 12u) err = FND_E_IO;
        else end = rd64(rx + 8);
    }
    if (err == FND_OK) o->pos = o->flags & FND_OPEN_APPEND ? end : o->pos + len;
    return err;
}

static fnd_status file_op(obj_t *o, mount_t *m, const fnd_msg *q, const uint8_t *data, uint32_t len,
                          uint32_t total, uint32_t cap, fnd_msg *rep) {
    uint32_t readable = (o->flags & (FND_OPEN_READ | FND_OPEN_EXEC)) != 0;
    uint32_t writable = (o->flags & FND_OPEN_WRITE) != 0;
    fnd_status st;
    switch (q->op) {
    case FND_ST_READ: {
        if (q->a0 == 0 || q->a0 > FND_IPC_MAX_DATA || q->a1 || q->a2 || q->a3 || len) return FND_E_BAD_ARG;
        if (!readable) return FND_E_ACCESS;
        uint32_t want = q->a0 < cap ? q->a0 : cap, got = 0;
        if (want && (st = read_pipe(o, m, want, &got)) != FND_OK) return st;
        o->pos += got;
        rep->a0 = got;
        rep->buf = (uint32_t)(uintptr_t)reply_buf;
        rep->len = got;
        return FND_OK;
    }
    case FND_ST_WRITE:
        if (len == 0 || total != len || q->a0 || q->a1 || q->a2 || q->a3) return FND_E_BAD_ARG;
        if (!writable) return FND_E_ACCESS;
        if ((st = write_pipe(o, m, data, len)) != FND_OK) return st;
        rep->a0 = len;
        return FND_OK;
    case FND_ST_SEEK: {
        if (q->a0 > FND_SEEK_END || q->a3 || len) return FND_E_BAD_ARG;
        int64_t base = 0, delta = (int64_t)((uint64_t)q->a1 << 32 | q->a2);
        if (q->a0 == FND_SEEK_CUR) base = (int64_t)o->pos;
        if (q->a0 == FND_SEEK_END) {
            if ((st = stat_fid(m, o->fid)) != FND_OK) return st;
            base = (int64_t)rd64(reply_buf + 16);
        }
        if ((delta < 0 && base + delta < 0) || (delta > 0 && base > INT64_MAX - delta)) return FND_E_BAD_ARG;
        o->pos = (uint64_t)(base + delta);
        rep->a0 = (uint32_t)(o->pos >> 32);
        rep->a1 = (uint32_t)o->pos;
        return FND_OK;
    }
    case FND_ST_STAT:
        if (q->a0 || q->a1 || q->a2 || q->a3 || len) return FND_E_BAD_ARG;
        if (cap < FND_STAT_BYTES) return FND_E_LIMIT;
        if ((st = stat_fid(m, o->fid)) != FND_OK) return st;
        rep->buf = (uint32_t)(uintptr_t)reply_buf;
        rep->len = FND_STAT_BYTES;
        return FND_OK;
    case FND_ST_TRUNCATE:
        if (q->a0 || q->a3 || len) return FND_E_BAD_ARG;
        if (!writable) return FND_E_ACCESS;
        return setattr(m, o->fid, SET_SIZE, 0, 0, (uint64_t)q->a1 << 32 | q->a2);
    case FND_ST_SYNC:
        if (q->a0 || q->a1 || q->a2 || q->a3 || len) return FND_E_BAD_ARG;
        return cfp_fid_op(m, T_SYNC, o->fid, 0);
    default:
        return FND_E_UNSUPPORTED;
    }
}

/* DIR_NEXT: one READDIR, its entries with rec_len added (API §10.2). A CFP
 * entry is the API entry minus 4 bytes and at least 72 bytes, so asking
 * for at most cap * 18 / 19 bytes always fits in cap. */
static fnd_status dir_op(obj_t *o, mount_t *m, const fnd_msg *q, uint32_t len, uint32_t cap, fnd_msg *rep) {
    if (q->op != FND_DIR_NEXT && q->op != FND_DIR_REWIND) return FND_E_UNSUPPORTED;
    if (q->a0 || q->a1 || q->a2 || q->a3 || len) return FND_E_BAD_ARG;
    if (q->op == FND_DIR_REWIND) {
        o->cookie = o->end = 0;
        return FND_OK;
    }
    if (cap > FND_IPC_MAX_DATA) cap = FND_IPC_MAX_DATA;
    uint32_t max = cap / 19u * 18u;
    if (max > BODY_MAX) max = BODY_MAX;
    if (max < 76u) return FND_E_LIMIT;
    rep->buf = (uint32_t)(uintptr_t)reply_buf;
    if (o->end) return FND_OK;
    wr32(tx + 4, o->fid);
    wr32(tx + 8, o->cookie);
    wr16(tx + 12, max);
    wr16(tx + 14, 0);
    uint32_t blen;
    fnd_status st = cfp_call(m, T_READDIR, 12, &blen);
    if (st != FND_OK) return st;
    if (blen < 8u) return FND_E_IO;
    const uint8_t *b = rx + 4;
    uint32_t count = rd16(b + 4), at = 8, used = 0;
    for (uint32_t i = 0; i < count; ++i) {
        if (at + 68u > blen) return FND_E_IO;
        uint32_t nb = rd16(b + at + 64), size = (68u + nb + 3u) & ~3u, rec = size + 4u;
        if (nb == 0 || nb > CFP_NAME_MAX || at + size > blen || used + rec > cap) return FND_E_IO;
        zero(reply_buf + used, rec);
        wr32(reply_buf + used, rec);
        copy(reply_buf + used + 4, b + at, 68u + nb);
        if (mount_ro(m)) reply_buf[used + 4 + 15] |= FND_STAT_READ_ONLY;
        used += rec;
        at += size;
    }
    o->cookie = rd32(b);
    if (count == 0) o->end = 1;
    rep->a0 = count;
    rep->len = used;
    return FND_OK;
}

/* ---- mounts (the command) ---- */

/* FND_NET_SET: tells fs which names are under /net. */
static void publish(void) {
    uint8_t names[MOUNTS * (FND_NET_NAME_MAX + 1u)];
    uint32_t at = 0;
    for (uint32_t i = 0; i < MOUNTS; ++i) {
        if (!mounts[i].used) continue;
        uint32_t n = str_len(mounts[i].name);
        copy(names + at, mounts[i].name, n + 1u);
        at += n + 1u;
    }
    fnd_call(fnd_fs(), FND_NET_SET, 0, names, at, 0, 0, 0);
}

static fnd_status op_mount(const fnd_msg *q, const uint8_t *data, uint32_t len) {
    uint32_t elen = q->a2;
    if ((q->a1 & ~(FND_NET_MOUNT_RO | FND_NET_MOUNT_ENC)) || q->a3 || elen == 0 || elen >= len) return FND_E_BAD_ARG;
    if (!good_name(data, elen) || !good_name(data + elen, len - elen)) return FND_E_BAD_ARG;
    mount_t *m = 0;
    for (uint32_t i = 0; i < MOUNTS; ++i) {
        if (mounts[i].used && name_is(data + elen, len - elen, mounts[i].name)) return FND_E_EXISTS;
        if (!mounts[i].used && !m) m = &mounts[i];
    }
    if (!m) return FND_E_LIMIT;
    uint32_t gen = m->gen;
    zero(m, sizeof *m);
    m->gen = gen; /* objects of an earlier mount here stay STALE */
    m->addr = q->a0;
    m->flags = q->a1;
    copy(m->export_name, data, elen);
    copy(m->name, data + elen, len - elen);
    m->assoc = (l3p_assoc *)fnd_mem_alloc(sizeof *m->assoc);
    if (!m->assoc) return FND_E_NO_MEMORY;
    fnd_status st = mount_up(m);
    if (st != FND_OK) {
        fnd_mem_free(m->assoc);
        m->assoc = 0;
        return st;
    }
    m->used = 1;
    publish();
    return FND_OK;
}

static fnd_status op_umount(const uint8_t *data, uint32_t len) {
    for (uint32_t i = 0; i < MOUNTS; ++i) {
        mount_t *m = &mounts[i];
        if (!m->used || !name_is(data, len, m->name)) continue;
        for (uint32_t k = 0; k < OBJECTS; ++k)
            if (objs[k].kind != OBJ_FREE && objs[k].mount == i) return FND_E_BUSY;
        if (m->up) l3p_close(m->assoc);
        fnd_mem_free(m->assoc);
        m->assoc = 0;
        m->up = m->used = 0;
        publish();
        return FND_OK;
    }
    return FND_E_NOT_FOUND;
}

/* FND_NET_LIST: per mount addr, flags, up (u32, host order), name[32],
 * export[32]. */
#define LIST_RECORD FND_NET_LIST_RECORD
static fnd_status op_list(uint32_t cap, fnd_msg *rep) {
    uint32_t at = 0, count = 0;
    for (uint32_t i = 0; i < MOUNTS; ++i) {
        mount_t *m = &mounts[i];
        if (!m->used) continue;
        if (at + LIST_RECORD > cap) return FND_E_LIMIT;
        uint32_t words[3] = {m->addr, m->flags | (m->remote_ro ? FND_NET_MOUNT_RO : 0), m->up};
        copy(reply_buf + at, words, 12);
        copy(reply_buf + at + 12, m->name, 32);
        copy(reply_buf + at + 44, m->export_name, 32);
        at += LIST_RECORD;
        ++count;
    }
    rep->a0 = count;
    rep->buf = (uint32_t)(uintptr_t)reply_buf;
    rep->len = at;
    return FND_OK;
}

static fnd_status request(uint32_t badge, const fnd_msg *q, uint32_t len, uint32_t total, uint32_t cap,
                          fnd_msg *rep) {
    if (q->flags) return FND_E_BAD_ARG; /* no handles are taken */
    obj_t *o = obj_find(badge);
    if (o) {
        mount_t *m = &mounts[o->mount];
        if (!m->used || !m->up || o->gen != m->gen) return FND_E_STALE;
        return o->kind == OBJ_DIR ? dir_op(o, m, q, len, cap, rep) : file_op(o, m, q, req_data, len, total, cap, rep);
    }
    if (!(badge & KERNEL_BADGE)) return FND_E_BAD_HANDLE;
    if (total > len) return FND_E_NAME_TOO_LONG;
    if (q->op >= FND_NET_FS + (FND_FS_OPEN & 0xFFu) && q->op <= FND_NET_FS + (FND_FS_VOLUME_INFO & 0xFFu))
        return fs_op(q, req_data, len, rep);
    switch (q->op) {
    case FND_NET_MOUNT:
        return op_mount(q, req_data, len);
    case FND_NET_UMOUNT:
        return op_umount(req_data, len);
    case FND_NET_LIST:
        return op_list(cap, rep);
    default:
        return FND_E_UNSUPPORTED;
    }
}

/* Runs the L3P timers of the idle mounts (retransmission, keep-alive). */
static void poll_mounts(void) {
    for (uint32_t i = 0; i < MOUNTS; ++i) {
        mount_t *m = &mounts[i];
        if (m->used && m->up && l3p_poll(m->assoc) != FND_OK) mount_fail(m, FND_E_STALE);
    }
}

static int32_t service(void) {
    int32_t port = fnd_sys_port_create();
    if (port < 0) return port;
    fnd_status st = fnd_sys_svc_register("netfs", (uint32_t)port);
    if (st != FND_OK) return st;
    service_port = (uint32_t)port;
    for (;;) {
        fnd_recv r;
        zero(&r, sizeof r);
        r.data = (uint32_t)(uintptr_t)req_data;
        r.cap = sizeof req_data;
        r.timeout = FND_FOREVER;
        for (uint32_t i = 0; i < MOUNTS; ++i)
            if (mounts[i].up) r.timeout = POLL_TICKS;
        st = fnd_sys_ipc_recv(service_port, &r);
        poll_mounts();
        if (st != FND_OK) continue;
        if (r.token == 0) {
            if (r.msg.op == FND_NOTIFY_CLIENT_GONE) obj_gone(r.badge);
            continue;
        }
        if (r.msg.flags & FND_MSG_XFER_HANDLE) fnd_sys_handle_close(r.msg.a3);
        fnd_msg rep;
        zero(&rep, sizeof rep);
        st = request(r.badge, &r.msg, r.msg.len, r.total, r.reply_cap, &rep);
        if (st != FND_OK) {
            if ((rep.flags & FND_MSG_XFER_HANDLE) && rep.a3) fnd_sys_handle_close(rep.a3);
            zero(&rep, sizeof rep);
        }
        rep.op = (uint32_t)st;
        /* A CHANNEL that does not reach the client is closed here, and
         * its CLIENT_GONE frees the object. */
        if (fnd_sys_ipc_reply(r.token, &rep) != FND_OK && (rep.flags & FND_MSG_XFER_HANDLE))
            fnd_sys_handle_close(rep.a3);
    }
}


int main(void) {
    fnd_init();
    fnd_exit(service());
}
