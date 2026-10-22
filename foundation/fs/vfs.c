/* The VFS namespace (doc/spec_FS2.md §11). See vfs.h. */
#include "vfs.h"

#include <string.h>

#include "name.h"

vfs_mount vfs_mounts[VFS_MOUNTS];

/* Built-in boot configuration of /dev (§11.1). con is the console service
 * (spec_Foundation_API.md §9: it registers dev.con); null is served by fs. */
const vfs_devnode vfs_devnodes[] = {
    {"con", VFS_DEV_SERVICE, "dev.con", FND_TYPE_CONSOLE, 0, 0x0036u},
    {"null", VFS_DEV_NULL, 0, FND_TYPE_DEVICE, 0, 0x0036u},
};
const uint32_t vfs_devnode_count = sizeof vfs_devnodes / sizeof vfs_devnodes[0];

static const char *const dd_names[4] = {"0", "1", "2", "3"};

/* Virtual directories: uid 0, owner/other RX (§11.1). DSFS: files owner/
 * other R, the root owner/other RX (§11.2). */
#define VIRTUAL_DIR_MODE 0x002Du
#define DSFS_FILE_MODE 0x0024u
#define DSFS_ROOT_MODE 0x002Du

static uint32_t cstr_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

void vfs_init(void) {
    memset(vfs_mounts, 0, sizeof vfs_mounts);
    static const struct {
        uint32_t mnt, kind;
        const char *name;
    } fixed[] = {
        {VFS_ROOT, VFS_KIND_VIRTUAL, ""}, {VFS_DEV, VFS_KIND_DEV, "dev"}, {VFS_DD, VFS_KIND_VIRTUAL, "dd"},
        {VFS_OS, VFS_KIND_VIRTUAL, "os"}, {VFS_TMP, VFS_KIND_VIRTUAL, "tmp"},
    };
    for (uint32_t i = 0; i < sizeof fixed / sizeof fixed[0]; ++i) {
        vfs_mount *m = &vfs_mounts[fixed[i].mnt];
        m->present = fixed[i].mnt != VFS_TMP; /* /tmp appears with its RAM volume */
        m->kind = fixed[i].kind;
        m->fs_type = FND_VOL_VIRTUAL;
        m->slot = FND_VOL_NO_SLOT;
        m->parent = VFS_ROOT;
        m->name = fixed[i].name;
    }
    for (uint32_t n = 0; n < 4; ++n) {
        vfs_mount *m = &vfs_mounts[VFS_DD0 + n];
        m->kind = VFS_KIND_FS2;
        m->parent = VFS_DD;
        m->name = dd_names[n];
        m->slot = n;
    }
}

void vfs_mount_fs2(uint32_t mnt, fs2_vol *vol, uint32_t fs_type, uint32_t slot) {
    vfs_mount *m = &vfs_mounts[mnt];
    m->present = 1;
    m->kind = VFS_KIND_FS2;
    m->fs_type = fs_type;
    m->slot = slot;
    m->vol = vol;
}

void vfs_mount_dsfs(uint32_t mnt, dsfs_vol *vol, uint32_t slot) {
    vfs_mount *m = &vfs_mounts[mnt];
    m->present = 1;
    m->kind = VFS_KIND_DSFS;
    m->fs_type = FND_VOL_DSFS;
    m->slot = slot;
    m->dsfs = vol;
}

fnd_status vfs_read(vfs_node n, uint64_t off, void *buf, uint32_t len, uint32_t *got) {
    const vfs_mount *m = &vfs_mounts[n.mnt];
    if (m->kind == VFS_KIND_FS2)
        return fs2_read(m->vol, n.ino, off, buf, len, got);
    if (m->kind == VFS_KIND_DSFS && n.ino >= 2)
        return dsfs_read(m->dsfs, n.ino - 2, off, buf, len, got);
    *got = 0;
    return FND_OK;
}

static const char *const reserved_names[4] = {"dev", "dd", "os", "tmp"};
static const uint32_t reserved_mounts[4] = {VFS_DEV, VFS_DD, VFS_OS, VFS_TMP};

static int reserved_index(const uint8_t *name, uint32_t len) {
    for (uint32_t i = 0; i < 4; ++i) {
        const char *r = reserved_names[i];
        if (fnd_name_key_equal((const uint8_t *)r, cstr_len(r), name, len))
            return (int)i;
    }
    return -1;
}

int vfs_reserved(vfs_node dir, const uint8_t *name, uint32_t len) {
    return dir.mnt == VFS_ROOT && dir.ino == VFS_DIR_INO && reserved_index(name, len) >= 0;
}

fnd_status vfs_get_info(vfs_node n, vfs_info *out) {
    const vfs_mount *m = &vfs_mounts[n.mnt];
    memset(out, 0, sizeof *out);
    if (!m->present)
        return FND_E_NOT_FOUND;
    if (m->kind == VFS_KIND_FS2) {
        fnd_status st = fs2_inode_read(m->vol, n.ino, &out->inode);
        if (st != FND_OK)
            return st;
        if (out->inode.type == FS2_TYPE_FREE)
            return FND_E_NOT_FOUND;
        out->type = out->inode.type == FS2_TYPE_DIR ? FND_TYPE_DIR : FND_TYPE_FILE;
        out->mode = out->inode.mode;
        out->uid = out->inode.uid;
        out->read_only = !m->vol->writable;
        return FND_OK;
    }
    if (n.ino == VFS_DIR_INO) {
        out->type = FND_TYPE_DIR;
        out->mode = VIRTUAL_DIR_MODE;
        out->read_only = 1;
        if (m->kind == VFS_KIND_DSFS) {
            out->inode.type = FS2_TYPE_DIR;
            out->inode.mode = DSFS_ROOT_MODE;
            out->mode = DSFS_ROOT_MODE;
        }
        return FND_OK;
    }
    if (m->kind == VFS_KIND_DSFS) {
        /* §11.2: DSFS has no UID or mode; seconds are zero-extended. */
        dsfs_entry e;
        if (n.ino - 2 >= DSFS_ENTRIES)
            return FND_E_NOT_FOUND;
        dsfs_entry_get(m->dsfs, n.ino - 2, &e);
        if (!e.used)
            return FND_E_NOT_FOUND;
        out->type = FND_TYPE_FILE;
        out->mode = DSFS_FILE_MODE;
        out->read_only = 1;
        out->inode.type = FS2_TYPE_FILE;
        out->inode.size = e.size;
        out->inode.created.sec = e.created;
        out->inode.modified.sec = out->inode.changed.sec = e.modified;
        return FND_OK;
    }
    if (m->kind == VFS_KIND_DEV && n.ino >= 2 && n.ino - 2 < vfs_devnode_count) {
        out->dev = &vfs_devnodes[n.ino - 2];
        out->type = out->dev->type;
        out->mode = out->dev->mode;
        out->uid = out->dev->uid;
        return FND_OK;
    }
    return FND_E_NOT_FOUND;
}

vfs_node vfs_parent(vfs_node n) {
    vfs_node p = n;
    if (n.ino == VFS_DIR_INO) {
        if (n.mnt != VFS_ROOT)
            p.mnt = vfs_mounts[n.mnt].parent;
        return p;
    }
    if (vfs_mounts[n.mnt].kind == VFS_KIND_FS2) {
        fs2_inode i;
        if (fs2_inode_read(vfs_mounts[n.mnt].vol, n.ino, &i) == FND_OK)
            p.ino = i.parent;
        return p;
    }
    p.ino = VFS_DIR_INO;
    return p;
}

int vfs_same_volume(vfs_node a, vfs_node b) {
    const vfs_mount *x = &vfs_mounts[a.mnt], *y = &vfs_mounts[b.mnt];
    if (x->kind == VFS_KIND_FS2 && y->kind == VFS_KIND_FS2)
        return x->vol == y->vol;
    return a.mnt == b.mnt;
}

fnd_status vfs_permit(const vfs_cred *c, const vfs_info *i, uint32_t need) {
    if (c->uid == 0)
        return FND_OK;
    uint32_t bits = c->uid == i->uid ? (i->mode >> 3) & 7u : i->mode & 7u;
    return (bits & need) == need ? FND_OK : FND_E_ACCESS;
}

static fs2_dirent lookup_entry;

fnd_status vfs_lookup(vfs_node dir, const uint8_t *name, uint32_t len, vfs_node *out, const uint8_t **stored,
                      uint32_t *stored_len) {
    const vfs_mount *m = &vfs_mounts[dir.mnt];
    if (dir.mnt == VFS_ROOT && dir.ino == VFS_DIR_INO) {
        int r = reserved_index(name, len);
        if (r >= 0) {
            if (!vfs_mounts[reserved_mounts[r]].present)
                return FND_E_NOT_FOUND;
            out->mnt = reserved_mounts[r];
            out->ino = VFS_DIR_INO;
            *stored = (const uint8_t *)reserved_names[r];
            *stored_len = cstr_len(reserved_names[r]);
            return FND_OK;
        }
    }
    if (m->kind == VFS_KIND_FS2) {
        uint32_t ino;
        fnd_status st = fs2_lookup(m->vol, dir.ino, name, len, &ino, &lookup_entry);
        if (st != FND_OK)
            return st;
        out->mnt = dir.mnt;
        out->ino = ino;
        *stored = lookup_entry.name;
        *stored_len = lookup_entry.name_bytes;
        return FND_OK;
    }
    if (dir.ino != VFS_DIR_INO)
        return FND_E_NOT_DIR;
    if (m->kind == VFS_KIND_DSFS) {
        /* §11.1: valid 8.3 names, compared like every other name. */
        static dsfs_entry e;
        for (uint32_t i = 0; i < DSFS_ENTRIES; ++i) {
            dsfs_entry_get(m->dsfs, i, &e);
            if (e.used && fnd_name_key_equal(e.name, e.name_len, name, len)) {
                out->mnt = dir.mnt;
                out->ino = 2 + i;
                *stored = e.name;
                *stored_len = e.name_len;
                return FND_OK;
            }
        }
        return FND_E_NOT_FOUND;
    }
    if (dir.mnt == VFS_DD) {
        for (uint32_t n = 0; n < 4; ++n) {
            if (vfs_mounts[VFS_DD0 + n].present && len == 1 && name[0] == (uint8_t)('0' + n)) {
                out->mnt = VFS_DD0 + n;
                out->ino = VFS_DIR_INO;
                *stored = (const uint8_t *)dd_names[n];
                *stored_len = 1;
                return FND_OK;
            }
        }
    } else if (m->kind == VFS_KIND_DEV) {
        for (uint32_t i = 0; i < vfs_devnode_count; ++i) {
            const char *dn = vfs_devnodes[i].name;
            if (fnd_name_key_equal((const uint8_t *)dn, cstr_len(dn), name, len)) {
                out->mnt = VFS_DEV;
                out->ino = 2 + i;
                *stored = (const uint8_t *)dn;
                *stored_len = cstr_len(dn);
                return FND_OK;
            }
        }
    }
    return FND_E_NOT_FOUND;
}

static void path_pop(vfs_walk *w) {
    while (w->path_len > 1 && w->path[w->path_len - 1] != '/')
        --w->path_len;
    if (w->path_len > 1)
        --w->path_len;
    w->path[w->path_len] = 0;
}

static fnd_status path_push(vfs_walk *w, const uint8_t *name, uint32_t len) {
    uint32_t sep = w->path_len > 1;
    if (w->path_len + sep + len > VFS_PATH_MAX)
        return FND_E_NAME_TOO_LONG;
    if (sep)
        w->path[w->path_len++] = '/';
    memcpy(w->path + w->path_len, name, len);
    w->path_len += len;
    w->path[w->path_len] = 0;
    return FND_OK;
}

static int is_dot(const uint8_t *c, uint32_t n) { return n == 1 && c[0] == '.'; }
static int is_dotdot(const uint8_t *c, uint32_t n) { return n == 2 && c[0] == '.' && c[1] == '.'; }

/* The directory `cur` may be passed through: it is a directory and the
 * walker has X on it. */
static fnd_status enter(const vfs_cred *c, vfs_node cur) {
    vfs_info i;
    fnd_status st = vfs_get_info(cur, &i);
    if (st != FND_OK)
        return st;
    if (i.type != FND_TYPE_DIR)
        return FND_E_NOT_DIR;
    return vfs_permit(c, &i, VFS_X);
}

fnd_status vfs_walk_path(const vfs_cred *c, const uint8_t *path, uint32_t len, uint32_t parent, vfs_walk *w) {
    vfs_node cur;
    fnd_status st;
    memset(w, 0, sizeof *w - sizeof w->path);
    w->path[0] = 0;
    if (len == 0)
        return FND_E_BAD_PATH;
    if (len > VFS_PATH_MAX)
        return FND_E_NAME_TOO_LONG;
    for (uint32_t i = 0; i < len; ++i)
        if (path[i] == 0)
            return FND_E_BAD_PATH;
    if (path[0] == '/') {
        cur.mnt = VFS_ROOT;
        cur.ino = VFS_DIR_INO;
        w->path[0] = '/';
        w->path_len = 1;
    } else {
        /* §11.1: the limit also applies after making the path absolute. */
        if (c->cwd_len + 1 + len > VFS_PATH_MAX)
            return FND_E_NAME_TOO_LONG;
        cur = c->cwd;
        memcpy(w->path, c->cwd_path, c->cwd_len);
        w->path_len = c->cwd_len;
    }
    w->path[w->path_len] = 0;
    w->want_dir = path[len - 1] == '/';
    /* The last component, for parent-mode walks. */
    uint32_t last_at = len, end = len;
    while (end > 0 && path[end - 1] == '/')
        --end;
    if (end > 0) {
        last_at = end;
        while (last_at > 0 && path[last_at - 1] != '/')
            --last_at;
    }
    uint32_t i = 0;
    for (;;) {
        while (i < len && path[i] == '/')
            ++i;
        if (i >= len)
            break;
        uint32_t s = i;
        while (i < len && path[i] != '/')
            ++i;
        const uint8_t *comp = path + s;
        uint32_t clen = i - s;
        if (parent && s == last_at) {
            if (is_dot(comp, clen) || is_dotdot(comp, clen))
                return FND_E_BAD_PATH;
            if ((st = enter(c, cur)) != FND_OK)
                return st;
            uint32_t chars;
            if ((st = fnd_name_check(comp, clen, &chars)) != FND_OK)
                return st;
            const uint8_t *stored;
            uint32_t stored_len;
            st = vfs_lookup(cur, comp, clen, &w->node, &stored, &stored_len);
            if (st != FND_OK && st != FND_E_NOT_FOUND)
                return st;
            w->found = st == FND_OK;
            w->parent = cur;
            w->last = comp;
            w->last_len = clen;
            return FND_OK;
        }
        if ((st = enter(c, cur)) != FND_OK)
            return st;
        if (is_dot(comp, clen))
            continue;
        if (is_dotdot(comp, clen)) {
            cur = vfs_parent(cur);
            path_pop(w);
            continue;
        }
        uint32_t chars;
        if ((st = fnd_name_check(comp, clen, &chars)) != FND_OK)
            return st;
        const uint8_t *stored;
        uint32_t stored_len;
        vfs_node next;
        if ((st = vfs_lookup(cur, comp, clen, &next, &stored, &stored_len)) != FND_OK)
            return st;
        if ((st = path_push(w, stored, stored_len)) != FND_OK)
            return st;
        cur = next;
    }
    if (parent)
        return FND_E_BAD_PATH; /* no name to create or remove ("/", ".") */
    vfs_info info;
    if ((st = vfs_get_info(cur, &info)) != FND_OK)
        return st;
    if (w->want_dir && info.type != FND_TYPE_DIR)
        return FND_E_NOT_DIR;
    w->found = 1;
    w->node = cur;
    return FND_OK;
}

static void put_time(uint8_t *p, fs_time t) {
    fs_put64(p, (uint64_t)t.sec);
    fs_put32(p + 8, t.ms);
}

void vfs_stat_encode(const vfs_info *i, uint8_t *out) {
    memset(out, 0, FND_STAT_BYTES);
    fs_put32(out + 0, i->type);
    fs_put32(out + 4, i->mode);
    fs_put32(out + 8, i->uid);
    uint32_t flags = i->read_only ? FND_STAT_READ_ONLY : 0;
    if (i->type == FND_TYPE_FILE)
        flags |= FND_STAT_SEEKABLE;
    fs_put32(out + 12, flags);
    if (i->type == FND_TYPE_FILE || (i->type == FND_TYPE_DIR && i->inode.type == FS2_TYPE_DIR)) {
        fs_put64(out + 16, i->inode.size);
        put_time(out + 24, i->inode.created);
        put_time(out + 36, i->inode.modified);
        put_time(out + 48, i->inode.changed);
    }
}
