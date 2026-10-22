#ifndef FOUNDATION_FS_VFS_H
#define FOUNDATION_FS_VFS_H
#include <stdint.h>

#include "fs2.h"

/* The VFS of the fs service (doc/spec_FS2.md §11): the mount table, path
 * resolution against a session's cwd, permissions, and the attributes of
 * every node. The operations on top of it are in server.c. */

/* Mounts. Every mount is attached to the root directory of its parent:
 * dev, dd, os and tmp to "/", and the IDC slots to "/dd". */
enum {
    VFS_ROOT,
    VFS_DEV,
    VFS_DD,
    VFS_OS,
    VFS_TMP,
    VFS_DD0, /* VFS_DD0 + n is /dd/<n> */
    VFS_MOUNTS = VFS_DD0 + 4
};

#define VFS_KIND_FS2 1u     /* an FS2 volume */
#define VFS_KIND_VIRTUAL 2u /* a fixed directory: "/" without a disk, /dd, /os */
#define VFS_KIND_DEV 3u     /* /dev and its nodes */

#define VFS_PATH_MAX 4096u
#define VFS_DIR_INO 1u /* the root of every mount */

/* /dev nodes come from the trusted boot configuration (§11.1). The
 * configuration format is not specified yet, so the table is built in
 * (vfs.c). `service` 0 is a node the fs service itself serves. */
#define VFS_DEV_NULL 0u
#define VFS_DEV_SERVICE 1u
typedef struct {
    const char *name;
    uint32_t kind;
    const char *service; /* dev.<name> for VFS_DEV_SERVICE */
    uint32_t type;       /* FND_TYPE_DEVICE or FND_TYPE_CONSOLE */
    uint32_t uid, mode;
} vfs_devnode;

typedef struct {
    uint32_t present;
    uint32_t kind;
    uint32_t fs_type; /* FND_VOL_* for FS_VOLUME_INFO */
    uint32_t slot;    /* IDC slot, or FND_VOL_NO_SLOT */
    uint32_t parent;  /* mount this one is attached to */
    const char *name; /* its name in the parent's root */
    fs2_vol *vol;     /* VFS_KIND_FS2; "/" and /dd/<boot> share one */
} vfs_mount;

typedef struct {
    uint32_t mnt, ino;
} vfs_node;

/* What the VFS knows about one node. */
typedef struct {
    uint32_t type; /* FND_TYPE_* */
    uint32_t mode, uid;
    uint32_t read_only;
    fs2_inode inode; /* for FS2 nodes */
    const vfs_devnode *dev;
} vfs_info;

typedef struct {
    uint32_t uid;
    vfs_node cwd;
    char *cwd_path; /* canonical absolute path of the stored names */
    uint32_t cwd_len;
} vfs_cred;

/* The result of a walk. */
typedef struct {
    uint32_t found;     /* `node` exists */
    vfs_node node;      /* the target */
    vfs_node parent;    /* parent-mode walks: the directory of `last` */
    const uint8_t *last; /* parent-mode walks: the last component */
    uint32_t last_len;
    uint32_t want_dir;  /* the path ended in '/' */
    char path[VFS_PATH_MAX + 1]; /* canonical path of the target (or parent) */
    uint32_t path_len;
} vfs_walk;

extern vfs_mount vfs_mounts[VFS_MOUNTS];
extern const vfs_devnode vfs_devnodes[];
extern const uint32_t vfs_devnode_count;

void vfs_init(void);
/* Attaches FS2 volumes. `fs_type` is FND_VOL_FS2 or FND_VOL_RAM. */
void vfs_mount_fs2(uint32_t mnt, fs2_vol *vol, uint32_t fs_type, uint32_t slot);

fnd_status vfs_get_info(vfs_node n, vfs_info *out);
vfs_node vfs_parent(vfs_node n);
/* 1 if both nodes are the same object (aliases of one volume included). */
int vfs_same_volume(vfs_node a, vfs_node b);

#define VFS_R 4u
#define VFS_W 2u
#define VFS_X 1u
/* §8: owner bits if uid is the owner, other bits otherwise; UID 0 skips
 * the check. */
fnd_status vfs_permit(const vfs_cred *c, const vfs_info *i, uint32_t need);

/* Walks `path` (§11.1). With parent = 0 the whole path is resolved and a
 * missing target is FND_E_NOT_FOUND. With parent = 1 all but the last
 * component are resolved; the last one must be a name, and `found` says
 * whether it exists. */
fnd_status vfs_walk_path(const vfs_cred *c, const uint8_t *path, uint32_t len, uint32_t parent, vfs_walk *w);

/* Lookup of one name in a directory node. */
fnd_status vfs_lookup(vfs_node dir, const uint8_t *name, uint32_t len, vfs_node *out, const uint8_t **stored,
                      uint32_t *stored_len);

/* Reserved names of "/" (dev, dd, os, tmp), compared by folded key. */
int vfs_reserved(vfs_node dir, const uint8_t *name, uint32_t len);

/* Fills a 64-byte fnd_stat (§10.1). */
void vfs_stat_encode(const vfs_info *i, uint8_t *out);

#endif
