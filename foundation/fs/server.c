/* The fs protocol (doc/spec_Foundation_API.md §4-§7, §10). See server.h. */
#include "server.h"

#include <string.h>

#include "name.h"

#define OBJ_FREE 0u
#define OBJ_SESSION 1u
#define OBJ_FILE 2u
#define OBJ_DIR 3u
#define OBJ_NULL 4u /* /dev/null */

#define KERNEL_BADGE 0x80000000u

typedef struct {
    uint32_t kind;
    uint32_t badge;
    vfs_cred cred;  /* OBJ_SESSION */
    vfs_node node;  /* OBJ_FILE, OBJ_DIR */
    uint32_t flags; /* FND_OPEN_* the object was opened with */
    uint64_t pos;   /* OBJ_FILE */
    uint32_t phase, cursor; /* OBJ_DIR: §7 listing position */
} fs_obj;

static fs_obj objs[FS_OBJECTS];
static uint32_t next_badge;
static uint8_t reply_buf[FS_IPC_MAX] FS_ALIGNED;
static vfs_walk walk_a, walk_b;

void fs_server_init(void) {
    memset(objs, 0, sizeof objs);
    next_badge = 0;
}

/* ---- objects ---------------------------------------------------------- */

static fs_obj *obj_find(uint32_t badge) {
    for (uint32_t i = 0; i < FS_OBJECTS; ++i)
        if (objs[i].kind != OBJ_FREE && objs[i].badge == badge)
            return &objs[i];
    return 0;
}

/* §4.4: badges 1..0x7FFFFFFF, not reused right after they are freed. */
static fs_obj *obj_new(uint32_t kind, uint32_t badge) {
    for (uint32_t i = 0; i < FS_OBJECTS; ++i) {
        if (objs[i].kind != OBJ_FREE)
            continue;
        if (!badge) {
            do {
                next_badge = (next_badge % 0x7FFFFFFFu) + 1u;
            } while (obj_find(next_badge));
            badge = next_badge;
        }
        memset(&objs[i], 0, sizeof objs[i]);
        objs[i].kind = kind;
        objs[i].badge = badge;
        return &objs[i];
    }
    return 0;
}

static fnd_status cred_set_cwd(vfs_cred *c, vfs_node node, const char *path, uint32_t len) {
    char *copy = (char *)fs_alloc(len + 1);
    if (!copy)
        return FND_E_NO_MEMORY;
    memcpy(copy, path, len);
    copy[len] = 0;
    fs_free(c->cwd_path);
    c->cwd_path = copy;
    c->cwd_len = len;
    c->cwd = node;
    return FND_OK;
}

static void obj_free(fs_obj *o) {
    if (o->kind == OBJ_SESSION)
        fs_free(o->cred.cwd_path);
    memset(o, 0, sizeof *o);
}

/* A session reached through svc_lookup: v1 gives every session UID 0
 * (§6.1) and the cwd "/". */
static fs_obj *session_new(uint32_t badge) {
    fs_obj *o = obj_new(OBJ_SESSION, badge);
    if (!o)
        return 0;
    vfs_node root = {VFS_ROOT, VFS_DIR_INO};
    if (cred_set_cwd(&o->cred, root, "/", 1) != FND_OK) {
        obj_free(o);
        return 0;
    }
    return o;
}

void fs_server_gone(uint32_t badge) {
    fs_obj *o = obj_find(badge);
    if (o)
        obj_free(o); /* writes are synchronous: nothing to flush (§4.3) */
}

/* FS2 §9, §11.2: a node is busy while a handle or a cwd refers to it, or,
 * with `subtree`, to anything below it, through any alias of its volume. */
static int busy(vfs_node target, uint32_t subtree) {
    for (uint32_t i = 0; i < FS_OBJECTS; ++i) {
        const fs_obj *o = &objs[i];
        vfs_node ref;
        if (o->kind == OBJ_FILE || o->kind == OBJ_DIR)
            ref = o->node;
        else if (o->kind == OBJ_SESSION)
            ref = o->cred.cwd;
        else
            continue;
        if (!vfs_same_volume(ref, target))
            continue;
        if (ref.ino == target.ino)
            return 1;
        const vfs_mount *m = &vfs_mounts[ref.mnt];
        uint32_t under = 0;
        if (subtree && m->kind == VFS_KIND_FS2 && fs2_is_under(m->vol, ref.ino, target.ino, &under) != FND_OK)
            return 1;
        if (under)
            return 1;
    }
    return 0;
}

/* ---- helpers ---------------------------------------------------------- */

static uint64_t join64(uint32_t hi, uint32_t lo) { return ((uint64_t)hi << 32) | lo; }

static fs2_vol *fs2_of(vfs_node n) {
    const vfs_mount *m = &vfs_mounts[n.mnt];
    return m->kind == VFS_KIND_FS2 ? m->vol : 0;
}

/* §6.3: FND_MODE_DEFAULT, or only bits 0..5. */
static fnd_status mode_arg(uint32_t mode, uint32_t def, uint32_t *out) {
    if (mode == FND_MODE_DEFAULT) {
        *out = def;
        return FND_OK;
    }
    if (mode & ~FS2_MODE_MASK)
        return FND_E_BAD_ARG;
    *out = mode;
    return FND_OK;
}

/* §11.2: changes need an FS2 backend that is not mounted read-only. */
static fnd_status writable_dir(vfs_node dir) {
    const vfs_mount *m = &vfs_mounts[dir.mnt];
    if (m->kind != VFS_KIND_FS2)
        return FND_E_READ_ONLY;
    return m->vol->writable ? FND_OK : FND_E_READ_ONLY;
}

/* §6.4: the permission READ and WRITE need. A table, because the bit
 * reversal the compiler makes of the obvious expression is not supported
 * by tools/llvm-src32. */
static uint32_t rw_need(uint32_t flags) {
    static const uint8_t need[4] = {0, VFS_R, VFS_W, VFS_R | VFS_W};
    return need[flags & (FND_OPEN_READ | FND_OPEN_WRITE)];
}

static fnd_status permit_node(const vfs_cred *c, vfs_node n, uint32_t need, vfs_info *info) {
    fnd_status st = vfs_get_info(n, info);
    return st != FND_OK ? st : vfs_permit(c, info, need);
}

/* ---- stream operations (§5) ---------------------------------------------- */

static void stream_op(fs_obj *o, const fs_req *q, const uint8_t *data, uint32_t len, uint32_t total,
                      uint32_t cap, fs_rep *r) {
    vfs_info info;
    fs2_vol *vol = o->kind == OBJ_FILE ? fs2_of(o->node) : 0;
    uint32_t file = o->kind == OBJ_FILE;
    uint32_t readable = (o->flags & (FND_OPEN_READ | FND_OPEN_EXEC)) != 0;
    uint32_t writable = (o->flags & FND_OPEN_WRITE) != 0;
    fnd_status st;
    switch (q->op) {
    case FND_ST_READ: {
        if (q->a0 == 0 || q->a0 > FS_IPC_MAX || q->a1 || q->a2 || q->a3 || len) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        if (!readable) {
            r->op = (uint32_t)FND_E_ACCESS;
            return;
        }
        uint32_t want = q->a0 < cap ? q->a0 : cap, got = 0;
        if (file) {
            st = vfs_read(o->node, o->pos, reply_buf, want, &got);
            if (st != FND_OK) {
                r->op = (uint32_t)st;
                return;
            }
            o->pos += got;
        }
        r->a0 = got;
        r->data = reply_buf;
        r->len = got;
        return;
    }
    case FND_ST_WRITE:
        if (len == 0 || total != len || q->a0 || q->a1 || q->a2 || q->a3) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        if (!writable) {
            r->op = (uint32_t)FND_E_ACCESS;
            return;
        }
        if (vol) {
            if (o->flags & FND_OPEN_APPEND) {
                if ((st = vfs_get_info(o->node, &info)) != FND_OK) {
                    r->op = (uint32_t)st;
                    return;
                }
                o->pos = info.inode.size;
            }
            if (o->pos + len < o->pos) {
                r->op = (uint32_t)FND_E_BAD_ARG;
                return;
            }
            st = fs2_write(vol, o->node.ino, o->pos, data, len, fs_now());
            if (st != FND_OK) {
                r->op = (uint32_t)st;
                return;
            }
            o->pos += len;
        }
        r->a0 = len;
        return;
    case FND_ST_SEEK: {
        if (q->a0 > FND_SEEK_END || q->a3 || len) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        if (!file) {
            r->op = (uint32_t)FND_E_UNSUPPORTED;
            return;
        }
        int64_t base = 0, delta = (int64_t)join64(q->a1, q->a2);
        if (q->a0 == FND_SEEK_CUR)
            base = (int64_t)o->pos;
        if (q->a0 == FND_SEEK_END) {
            if ((st = vfs_get_info(o->node, &info)) != FND_OK) {
                r->op = (uint32_t)st;
                return;
            }
            base = (int64_t)info.inode.size;
        }
        if ((delta < 0 && base + delta < 0) || (delta > 0 && base > INT64_MAX - delta)) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        o->pos = (uint64_t)(base + delta);
        r->a0 = (uint32_t)(o->pos >> 32);
        r->a1 = (uint32_t)o->pos;
        return;
    }
    case FND_ST_STAT:
        if (q->a0 || q->a1 || q->a2 || q->a3 || len) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        if (cap < FND_STAT_BYTES) {
            r->op = (uint32_t)FND_E_LIMIT;
            return;
        }
        if (o->kind == OBJ_NULL) {
            memset(&info, 0, sizeof info);
            info.type = FND_TYPE_DEVICE;
            info.mode = vfs_devnodes[1].mode;
        } else if ((st = vfs_get_info(o->node, &info)) != FND_OK) {
            r->op = (uint32_t)st;
            return;
        }
        vfs_stat_encode(&info, reply_buf);
        r->data = reply_buf;
        r->len = FND_STAT_BYTES;
        return;
    case FND_ST_TRUNCATE:
        if (q->a0 || q->a3 || len) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        if (!writable) {
            r->op = (uint32_t)FND_E_ACCESS;
            return;
        }
        if (!vol) {
            r->op = (uint32_t)FND_E_UNSUPPORTED;
            return;
        }
        r->op = (uint32_t)fs2_truncate(vol, o->node.ino, join64(q->a1, q->a2), fs_now());
        return;
    case FND_ST_SYNC:
        if (q->a0 || q->a1 || q->a2 || q->a3 || len) {
            r->op = (uint32_t)FND_E_BAD_ARG;
            return;
        }
        r->op = vol ? (uint32_t)fs2_sync(vol) : FND_OK;
        return;
    default:
        r->op = (uint32_t)FND_E_UNSUPPORTED;
    }
}

/* ---- directory listing (§7, §10.2) ------------------------------------- */

static const char *const root_reserved[4] = {"dev", "dd", "os", "tmp"};
static const uint32_t root_reserved_mnt[4] = {VFS_DEV, VFS_DD, VFS_OS, VFS_TMP};

typedef struct {
    vfs_node node;
    const uint8_t *name;
    uint32_t name_bytes, name_chars;
} dir_item;

static fs2_dirent dir_entry;

/* The next entry at (phase, cursor), and the position after it. */
static fnd_status dir_peek(const fs_obj *o, dir_item *it, uint32_t *phase, uint32_t *cursor) {
    const vfs_mount *m = &vfs_mounts[o->node.mnt];
    int root = o->node.mnt == VFS_ROOT && o->node.ino == VFS_DIR_INO;
    while (*phase == 0) {
        if (m->kind == VFS_KIND_FS2) {
            fnd_status st = fs2_dir_next(m->vol, o->node.ino, cursor, &dir_entry);
            if (st == FND_END)
                break;
            if (st != FND_OK)
                return st;
            /* §11.1: the reserved names hide the disk's entries. */
            if (root && vfs_reserved(o->node, dir_entry.name, dir_entry.name_bytes))
                continue;
            it->node.mnt = o->node.mnt;
            it->node.ino = dir_entry.ino;
            it->name = dir_entry.name;
            it->name_bytes = dir_entry.name_bytes;
            it->name_chars = 0;
            fnd_name_check(dir_entry.name, dir_entry.name_bytes, &it->name_chars);
            return FND_OK;
        }
        if (m->kind == VFS_KIND_DSFS) {
            static dsfs_entry e;
            while (*cursor < DSFS_ENTRIES) {
                dsfs_entry_get(m->dsfs, (*cursor)++, &e);
                if (!e.used)
                    continue;
                it->node.mnt = o->node.mnt;
                it->node.ino = 1 + *cursor;
                it->name = e.name;
                it->name_bytes = it->name_chars = e.name_len;
                return FND_OK;
            }
            break;
        }
        if (o->node.mnt == VFS_DD) {
            while (*cursor < 4 && !vfs_mounts[VFS_DD0 + *cursor].present)
                ++*cursor;
            if (*cursor >= 4)
                break;
            it->node.mnt = VFS_DD0 + *cursor;
            it->node.ino = VFS_DIR_INO;
            it->name = (const uint8_t *)vfs_mounts[VFS_DD0 + *cursor].name;
            it->name_bytes = it->name_chars = 1;
            ++*cursor;
            return FND_OK;
        }
        if (m->kind == VFS_KIND_DEV && *cursor < vfs_devnode_count) {
            const char *dn = vfs_devnodes[*cursor].name;
            it->node.mnt = VFS_DEV;
            it->node.ino = 2 + *cursor;
            it->name = (const uint8_t *)dn;
            it->name_bytes = it->name_chars = (uint32_t)strlen(dn);
            ++*cursor;
            return FND_OK;
        }
        break;
    }
    if (*phase == 0) {
        *phase = 1;
        *cursor = 0;
    }
    if (root) {
        while (*cursor < 4 && !vfs_mounts[root_reserved_mnt[*cursor]].present)
            ++*cursor;
        if (*cursor < 4) {
            it->node.mnt = root_reserved_mnt[*cursor];
            it->node.ino = VFS_DIR_INO;
            it->name = (const uint8_t *)root_reserved[*cursor];
            it->name_bytes = it->name_chars = (uint32_t)strlen(root_reserved[*cursor]);
            ++*cursor;
            return FND_OK;
        }
    }
    return FND_END;
}

static void dir_op(fs_obj *o, const fs_req *q, uint32_t len, uint32_t cap, fs_rep *r) {
    if (q->op != FND_DIR_NEXT && q->op != FND_DIR_REWIND) {
        r->op = (uint32_t)FND_E_UNSUPPORTED;
        return;
    }
    if (q->a0 || q->a1 || q->a2 || q->a3 || len) {
        r->op = (uint32_t)FND_E_BAD_ARG;
        return;
    }
    if (q->op == FND_DIR_REWIND) {
        o->phase = o->cursor = 0;
        return;
    }
    if (cap > FS_IPC_MAX)
        cap = FS_IPC_MAX;
    uint32_t used = 0, count = 0;
    for (;;) {
        dir_item it;
        uint32_t phase = o->phase, cursor = o->cursor;
        fnd_status st = dir_peek(o, &it, &phase, &cursor);
        if (st == FND_END)
            break;
        if (st != FND_OK) {
            if (!count) {
                r->op = (uint32_t)st;
                return;
            }
            break;
        }
        uint32_t rec = (FND_DIRENT_HEAD + it.name_bytes + 3u) & ~3u;
        if (used + rec > cap) {
            if (!count) {
                r->op = (uint32_t)FND_E_LIMIT;
                return;
            }
            break;
        }
        vfs_info info;
        uint8_t *e = reply_buf + used;
        if (vfs_get_info(it.node, &info) != FND_OK)
            memset(&info, 0, sizeof info);
        memset(e, 0, rec);
        fs_put32(e, rec);
        vfs_stat_encode(&info, e + 4);
        fs_put16(e + 68, it.name_bytes);
        fs_put16(e + 70, it.name_chars);
        memcpy(e + 72, it.name, it.name_bytes);
        used += rec;
        ++count;
        o->phase = phase;
        o->cursor = cursor;
    }
    r->a0 = count;
    r->data = reply_buf;
    r->len = used;
}

/* ---- session operations (§6) -------------------------------------------- */

static fnd_status open_device(fs_obj *s, const vfs_info *info, uint32_t flags, fs_rep *r) {
    if (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE | FND_OPEN_CREATE))
        return FND_E_ACCESS;
    uint32_t need = rw_need(flags);
    fnd_status st = vfs_permit(&s->cred, info, need);
    if (st != FND_OK)
        return st;
    flags &= FND_OPEN_READ | FND_OPEN_WRITE;
    if (info->dev->kind == VFS_DEV_NULL) {
        fs_obj *o = obj_new(OBJ_NULL, 0);
        if (!o)
            return FND_E_LIMIT;
        o->flags = flags;
        r->new_badge = o->badge;
    } else {
        uint32_t h;
        if ((st = fs_dev_open(info->dev->service, flags, &h)) != FND_OK)
            return st;
        r->fwd_handle = h;
    }
    r->a0 = info->type;
    return FND_OK;
}

static fnd_status op_open(fs_obj *s, const fs_req *q, const uint8_t *path, uint32_t len, fs_rep *r) {
    uint32_t flags = q->a0, mode;
    fnd_status st;
    vfs_info info;
    if (q->a2 || q->a3 || (flags & ~0x7Fu) || !(flags & (FND_OPEN_READ | FND_OPEN_WRITE | FND_OPEN_EXEC)))
        return FND_E_BAD_ARG;
    if (((flags & FND_OPEN_EXEC) && (flags & FND_OPEN_WRITE)) ||
        ((flags & FND_OPEN_EXCL) && !(flags & FND_OPEN_CREATE)) ||
        ((flags & (FND_OPEN_TRUNC | FND_OPEN_APPEND)) && !(flags & FND_OPEN_WRITE)))
        return FND_E_BAD_ARG;
    if ((st = mode_arg(q->a1, FS2_MODE_FILE_DEFAULT, &mode)) != FND_OK)
        return st;
    vfs_walk *w = &walk_a;
    if ((st = vfs_walk_path(&s->cred, path, len, 1, w)) != FND_OK) {
        /* "/" and paths ending in "." or ".." name a directory. */
        if (st == FND_E_BAD_PATH && vfs_walk_path(&s->cred, path, len, 0, w) == FND_OK)
            return FND_E_IS_DIR;
        return st;
    }
    vfs_node node = w->node;
    if (!w->found) {
        if (!(flags & FND_OPEN_CREATE))
            return FND_E_NOT_FOUND;
        if (w->want_dir)
            return FND_E_NOT_FOUND;
        if (vfs_reserved(w->parent, w->last, w->last_len))
            return FND_E_EXISTS;
        if ((st = permit_node(&s->cred, w->parent, VFS_W | VFS_X, &info)) != FND_OK)
            return st;
        if ((st = writable_dir(w->parent)) != FND_OK)
            return st;
        node.mnt = w->parent.mnt;
        st = fs2_create(fs2_of(w->parent), w->parent.ino, w->last, w->last_len, FS2_TYPE_FILE, mode,
                        s->cred.uid, fs_now(), &node.ino);
        if (st != FND_OK)
            return st;
        flags &= ~FND_OPEN_TRUNC; /* new and empty */
        if ((st = vfs_get_info(node, &info)) != FND_OK)
            return st;
    } else {
        if ((flags & FND_OPEN_CREATE) && (flags & FND_OPEN_EXCL))
            return FND_E_EXISTS;
        if ((st = vfs_get_info(node, &info)) != FND_OK)
            return st;
        if (info.type == FND_TYPE_DIR)
            return FND_E_IS_DIR;
        if (w->want_dir)
            return FND_E_NOT_DIR;
        if (info.dev)
            return open_device(s, &info, flags, r);
        uint32_t need = rw_need(flags) | (flags & FND_OPEN_EXEC ? VFS_R | VFS_X : 0);
        if ((st = vfs_permit(&s->cred, &info, need)) != FND_OK)
            return st;
        if ((flags & FND_OPEN_WRITE) && info.read_only)
            return FND_E_READ_ONLY;
    }
    fs_obj *o = obj_new(OBJ_FILE, 0);
    if (!o)
        return FND_E_LIMIT;
    if (flags & FND_OPEN_TRUNC) {
        st = fs2_truncate(fs2_of(node), node.ino, 0, fs_now());
        if (st != FND_OK) {
            obj_free(o);
            return st;
        }
    }
    o->node = node;
    o->flags = flags;
    r->new_badge = o->badge;
    r->a0 = FND_TYPE_FILE;
    return FND_OK;
}

static fnd_status op_dir_open(fs_obj *s, const uint8_t *path, uint32_t len, fs_rep *r) {
    vfs_walk *w = &walk_a;
    vfs_info info;
    fnd_status st = vfs_walk_path(&s->cred, path, len, 0, w);
    if (st != FND_OK)
        return st;
    if ((st = vfs_get_info(w->node, &info)) != FND_OK)
        return st;
    if (info.type != FND_TYPE_DIR)
        return FND_E_NOT_DIR;
    if ((st = vfs_permit(&s->cred, &info, VFS_R | VFS_X)) != FND_OK)
        return st;
    fs_obj *o = obj_new(OBJ_DIR, 0);
    if (!o)
        return FND_E_LIMIT;
    o->node = w->node;
    r->new_badge = o->badge;
    return FND_OK;
}

/* A parent-mode walk failed with BAD_PATH: "/" itself cannot be removed
 * or moved (FS2 §9), anything else ending in "." or ".." is a bad path. */
static fnd_status not_a_name(fs_obj *s, const uint8_t *path, uint32_t len, fnd_status st) {
    vfs_walk *w = &walk_b;
    if (st == FND_E_BAD_PATH && vfs_walk_path(&s->cred, path, len, 0, w) == FND_OK && w->node.mnt == VFS_ROOT &&
        w->node.ino == VFS_DIR_INO)
        return FND_E_BUSY;
    return st;
}

/* Parent-mode walk for a change to an existing name: the backend must be
 * writable and the caller needs W + X on the parent (§8). */
static fnd_status walk_change(fs_obj *s, const uint8_t *path, uint32_t len, vfs_walk *w) {
    vfs_info info;
    fnd_status st = vfs_walk_path(&s->cred, path, len, 1, w);
    if (st != FND_OK)
        return st;
    if (!w->found)
        return FND_E_NOT_FOUND;
    if (w->node.mnt != w->parent.mnt)
        return FND_E_BUSY; /* a mount point: dev, dd, os, tmp or /dd/<n> */
    if ((st = permit_node(&s->cred, w->parent, VFS_W | VFS_X, &info)) != FND_OK)
        return st;
    return writable_dir(w->parent);
}

static fnd_status op_mkdir(fs_obj *s, const fs_req *q, const uint8_t *path, uint32_t len) {
    vfs_walk *w = &walk_a;
    vfs_info info;
    uint32_t mode, ino;
    fnd_status st;
    if (q->a0 || q->a2 || q->a3)
        return FND_E_BAD_ARG;
    if ((st = mode_arg(q->a1, FS2_MODE_DIR_DEFAULT, &mode)) != FND_OK)
        return st;
    if ((st = vfs_walk_path(&s->cred, path, len, 1, w)) != FND_OK)
        return st == FND_E_BAD_PATH && vfs_walk_path(&s->cred, path, len, 0, w) == FND_OK ? FND_E_EXISTS : st;
    if (w->found || vfs_reserved(w->parent, w->last, w->last_len))
        return FND_E_EXISTS;
    if ((st = permit_node(&s->cred, w->parent, VFS_W | VFS_X, &info)) != FND_OK)
        return st;
    if ((st = writable_dir(w->parent)) != FND_OK)
        return st;
    return fs2_create(fs2_of(w->parent), w->parent.ino, w->last, w->last_len, FS2_TYPE_DIR, mode, s->cred.uid,
                      fs_now(), &ino);
}

static fnd_status op_remove(fs_obj *s, const fs_req *q, const uint8_t *path, uint32_t len, uint32_t want_dir) {
    vfs_walk *w = &walk_a;
    vfs_info info;
    fnd_status st;
    if (q->a0 || q->a1 || q->a2 || q->a3)
        return FND_E_BAD_ARG;
    if ((st = walk_change(s, path, len, w)) != FND_OK)
        return not_a_name(s, path, len, st);
    if ((st = vfs_get_info(w->node, &info)) != FND_OK)
        return st;
    if (!want_dir && info.type == FND_TYPE_DIR)
        return FND_E_IS_DIR;
    if (want_dir && info.type != FND_TYPE_DIR)
        return FND_E_NOT_DIR;
    if (!want_dir && w->want_dir)
        return FND_E_NOT_DIR;
    if (busy(w->node, 0))
        return FND_E_BUSY;
    return fs2_remove(fs2_of(w->parent), w->parent.ino, w->last, w->last_len, want_dir, fs_now());
}

static fnd_status op_rename(fs_obj *s, const fs_req *q, const uint8_t *data, uint32_t len) {
    vfs_walk *a = &walk_a, *b = &walk_b;
    vfs_info info;
    fnd_status st;
    if (q->a1 || q->a2 || q->a3 || q->a0 == 0 || q->a0 >= len)
        return q->a0 == 0 || q->a0 == len ? FND_E_BAD_PATH : FND_E_BAD_ARG;
    if ((st = walk_change(s, data, q->a0, a)) != FND_OK)
        return not_a_name(s, data, q->a0, st);
    if ((st = vfs_walk_path(&s->cred, data + q->a0, len - q->a0, 1, b)) != FND_OK)
        return st;
    if (!vfs_same_volume(a->parent, b->parent) || a->parent.mnt != b->parent.mnt)
        return vfs_mounts[b->parent.mnt].kind == VFS_KIND_FS2 ? FND_E_CROSS_DEVICE : FND_E_READ_ONLY;
    if (vfs_reserved(b->parent, b->last, b->last_len))
        return FND_E_EXISTS;
    if ((st = permit_node(&s->cred, b->parent, VFS_W | VFS_X, &info)) != FND_OK)
        return st;
    if ((st = vfs_get_info(a->node, &info)) != FND_OK)
        return st;
    if (a->want_dir && info.type != FND_TYPE_DIR)
        return FND_E_NOT_DIR;
    if (busy(a->node, info.type == FND_TYPE_DIR))
        return FND_E_BUSY;
    return fs2_rename(fs2_of(a->parent), a->parent.ino, a->last, a->last_len, b->parent.ino, b->last, b->last_len,
                      fs_now());
}

static fnd_status op_attr(fs_obj *s, const fs_req *q, const uint8_t *path, uint32_t len) {
    vfs_walk *w = &walk_a;
    vfs_info info;
    fnd_status st;
    if (q->a1 || q->a2 || q->a3)
        return FND_E_BAD_ARG;
    if ((st = vfs_walk_path(&s->cred, path, len, 0, w)) != FND_OK)
        return st;
    if ((st = vfs_get_info(w->node, &info)) != FND_OK)
        return st;
    fs2_vol *vol = fs2_of(w->node);
    if (q->op == FND_FS_CHMOD) {
        if (q->a0 & ~FS2_MODE_MASK)
            return FND_E_BAD_ARG;
        if (s->cred.uid != 0 && s->cred.uid != info.uid)
            return FND_E_ACCESS;
    } else {
        if (q->a0 == 0xFFFFFFFFu)
            return FND_E_BAD_ARG;
        if (s->cred.uid != 0)
            return FND_E_ACCESS;
    }
    if (!vol || !vol->writable)
        return FND_E_READ_ONLY;
    if (q->op == FND_FS_CHMOD)
        return fs2_set_owner(vol, w->node.ino, 1, q->a0, 0, 0, fs_now());
    return fs2_set_owner(vol, w->node.ino, 0, 0, 1, q->a0, fs_now());
}

static fnd_status op_volume(fs_obj *s, const fs_req *q, const uint8_t *path, uint32_t len, uint32_t cap,
                            fs_rep *r) {
    vfs_walk *w = &walk_a;
    fnd_status st;
    if (q->a0 || q->a1 || q->a2 || q->a3)
        return FND_E_BAD_ARG;
    if ((st = vfs_walk_path(&s->cred, path, len, 0, w)) != FND_OK)
        return st;
    const vfs_mount *m = &vfs_mounts[w->node.mnt];
    if (q->op == FND_FS_SYNC)
        return m->kind == VFS_KIND_FS2 ? fs2_sync(m->vol) : FND_OK;
    if (cap < FND_VOLINFO_BYTES)
        return FND_E_LIMIT;
    uint8_t *v = reply_buf;
    memset(v, 0, FND_VOLINFO_BYTES);
    fs_put32(v, m->fs_type);
    if (m->kind == VFS_KIND_FS2) {
        fs_put32(v + 4, m->vol->writable ? 0 : FND_VOL_READ_ONLY);
        fs_put64(v + 8, (uint64_t)FS2_PAYLOAD * (m->vol->total_blocks - m->vol->data_start));
        fs_put64(v + 16, (uint64_t)FS2_PAYLOAD * m->vol->free_blocks);
    } else if (m->kind == VFS_KIND_DSFS) {
        fs_put32(v + 4, FND_VOL_READ_ONLY);
        fs_put64(v + 8, 512ull * (m->dsfs->total_blocks - m->dsfs->data_start));
        fs_put64(v + 16, 512ull * m->dsfs->free_blocks);
    } else {
        fs_put32(v + 4, FND_VOL_READ_ONLY);
    }
    fs_put32(v + 24, m->slot);
    r->data = v;
    r->len = FND_VOLINFO_BYTES;
    return FND_OK;
}

static fnd_status op_cwd(fs_obj *s, const fs_req *q, const uint8_t *path, uint32_t len, uint32_t cap,
                         fs_rep *r) {
    vfs_walk *w = &walk_a;
    vfs_info info;
    fnd_status st;
    if (q->a0 || q->a1 || q->a2 || q->a3)
        return FND_E_BAD_ARG;
    if (q->op == FND_FS_CWD_GET) {
        if (len)
            return FND_E_BAD_ARG;
        if (cap < s->cred.cwd_len)
            return FND_E_LIMIT;
        r->data = (const uint8_t *)s->cred.cwd_path;
        r->len = s->cred.cwd_len;
        return FND_OK;
    }
    if (q->op == FND_FS_SESSION_NEW) {
        if (len)
            return FND_E_BAD_ARG;
        fs_obj *o = obj_new(OBJ_SESSION, 0);
        if (!o)
            return FND_E_LIMIT;
        o->cred.uid = s->cred.uid;
        if ((st = cred_set_cwd(&o->cred, s->cred.cwd, s->cred.cwd_path, s->cred.cwd_len)) != FND_OK) {
            obj_free(o);
            return st;
        }
        r->new_badge = o->badge;
        return FND_OK;
    }
    /* FS_CWD_SET: a directory with search permission (§6.3). */
    if ((st = vfs_walk_path(&s->cred, path, len, 0, w)) != FND_OK)
        return st;
    if ((st = permit_node(&s->cred, w->node, VFS_X, &info)) != FND_OK)
        return st;
    if (info.type != FND_TYPE_DIR)
        return FND_E_NOT_DIR;
    return cred_set_cwd(&s->cred, w->node, w->path, w->path_len);
}

static void session_op(fs_obj *s, const fs_req *q, const uint8_t *data, uint32_t len, uint32_t total,
                       uint32_t cap, fs_rep *r) {
    fnd_status st;
    vfs_info info;
    /* §6.2: a path is 1..4096 bytes; the receive buffer holds 4096, so a
     * longer one shows as total > len. */
    if (total > len) {
        r->op = (uint32_t)FND_E_NAME_TOO_LONG;
        return;
    }
    switch (q->op) {
    case FND_FS_OPEN:
        st = op_open(s, q, data, len, r);
        break;
    case FND_FS_STAT:
        if (q->a0 || q->a1 || q->a2 || q->a3) {
            st = FND_E_BAD_ARG;
            break;
        }
        if (cap < FND_STAT_BYTES) {
            st = FND_E_LIMIT;
            break;
        }
        st = vfs_walk_path(&s->cred, data, len, 0, &walk_a);
        if (st == FND_OK)
            st = vfs_get_info(walk_a.node, &info);
        if (st == FND_OK) {
            vfs_stat_encode(&info, reply_buf);
            r->data = reply_buf;
            r->len = FND_STAT_BYTES;
        }
        break;
    case FND_FS_DIR_OPEN:
        st = q->a0 || q->a1 || q->a2 || q->a3 ? FND_E_BAD_ARG : op_dir_open(s, data, len, r);
        break;
    case FND_FS_MKDIR:
        st = op_mkdir(s, q, data, len);
        break;
    case FND_FS_UNLINK:
    case FND_FS_RMDIR:
        st = op_remove(s, q, data, len, q->op == FND_FS_RMDIR);
        break;
    case FND_FS_RENAME:
        st = op_rename(s, q, data, len);
        break;
    case FND_FS_CHMOD:
    case FND_FS_CHOWN:
        st = op_attr(s, q, data, len);
        break;
    case FND_FS_SYNC:
    case FND_FS_VOLUME_INFO:
        st = op_volume(s, q, data, len, cap, r);
        break;
    case FND_FS_CWD_SET:
    case FND_FS_CWD_GET:
    case FND_FS_SESSION_NEW:
        st = op_cwd(s, q, data, len, cap, r);
        break;
    default:
        st = FND_E_UNSUPPORTED;
    }
    if (st != FND_OK) {
        r->new_badge = r->fwd_handle = 0;
        r->a0 = r->a1 = r->a2 = r->a3 = 0;
        r->data = 0;
        r->len = 0;
    }
    r->op = (uint32_t)st;
}

void fs_server_request(uint32_t badge, const fs_req *q, const uint8_t *data, uint32_t len, uint32_t total,
                       uint32_t reply_cap, fs_rep *r) {
    memset(r, 0, sizeof *r);
    fs_obj *o = obj_find(badge);
    if (!o && (badge & KERNEL_BADGE))
        o = session_new(badge);
    if (!o) {
        r->op = (uint32_t)(badge & KERNEL_BADGE ? FND_E_LIMIT : FND_E_BAD_HANDLE);
        return;
    }
    if (q->flags) {
        r->op = (uint32_t)FND_E_BAD_ARG; /* fs takes no handles */
        return;
    }
    uint32_t op = q->op;
    if (o->kind == OBJ_SESSION) {
        if (op >= 0x0100u && op <= 0x01FFu)
            session_op(o, q, data, len, total, reply_cap, r);
        else
            r->op = (uint32_t)FND_E_UNSUPPORTED;
    } else if (o->kind == OBJ_DIR) {
        dir_op(o, q, len, reply_cap, r);
    } else if (op >= 0x0001u && op <= 0x00FFu) {
        stream_op(o, q, data, len, total, reply_cap, r);
    } else {
        r->op = (uint32_t)FND_E_UNSUPPORTED;
    }
}
