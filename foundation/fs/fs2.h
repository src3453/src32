#ifndef FOUNDATION_FS_FS2_H
#define FOUNDATION_FS_FS2_H
#include <stdint.h>

#include "fs.h"

/* FS2 volume driver (doc/spec_FS2.md). It reads and writes one volume
 * through an fs_dev and knows nothing about paths, mounts or permissions:
 * those belong to the VFS (vfs.c). Every function returns FND_OK or a
 * negative Foundation error (spec_Foundation_API.md §3). */

#define FS2_BLOCK 512u
#define FS2_PAYLOAD 508u
#define FS2_INODE_BYTES 128u
#define FS2_DIRENT 1040u
#define FS2_NAME_MAX_BYTES 1024u
#define FS2_NAME_MAX_CHARS 256u
#define FS2_ROOT 1u
#define FS2_CASEFOLD_VERSION 0x00100000u /* Unicode 16.0.0 */

#define FS2_TYPE_FREE 0u
#define FS2_TYPE_FILE 1u
#define FS2_TYPE_DIR 2u

#define FS2_MODE_FILE_DEFAULT 0x0034u
#define FS2_MODE_DIR_DEFAULT 0x003Du
#define FS2_MODE_MASK 0x003Fu

#define FS2_SB_DIRTY 1u

#define FS2_CACHE_SECTORS 32u
#define FS2_CACHE_META 8u
/* Sectors read per cache miss: every IDC command costs the drive's access
 * latency (spec_IDC.md, 10 ms on an HDD), and chains are mostly contiguous. */
#define FS2_READ_AHEAD 8u
#define FS2_HINTS 8u

typedef struct {
    uint32_t type, mode, uid, parent, first, count;
    uint64_t size;
    fs_time created, modified, changed;
} fs2_inode;

typedef struct {
    uint32_t lba;
    uint32_t valid;
    uint8_t data[FS2_BLOCK];
} fs2_cache_line;

/* Where block `index` of inode `ino` lives, so that sequential access does
 * not walk the chain from its start every time. */
typedef struct {
    uint32_t ino, index, lba, age;
} fs2_hint;

typedef struct {
    fs_dev *dev;
    uint32_t writable; /* mounted read-write */
    uint32_t dirty;    /* DIRTY=1 is on the media */
    uint32_t failed;   /* an I/O error stopped new updates (§9.1) */
    uint32_t flags;    /* superblock flags as read */
    uint32_t total_blocks, boot_blocks, boot_size;
    uint32_t bitmap_start, bitmap_blocks;
    uint32_t inode_start, inode_blocks, inode_count;
    uint32_t data_start;
    uint32_t free_blocks;
    fs_time created;
    uint8_t *bitmap;     /* bitmap_blocks * 512 bytes, as on the media */
    uint8_t *inode_used; /* one bit per inode number, bit n = inode n */
    uint32_t clock;
    fs2_cache_line cache[FS2_CACHE_SECTORS];
    fs2_hint hints[FS2_HINTS];
    uint8_t ahead[FS2_READ_AHEAD * FS2_BLOCK];
} fs2_vol;

typedef struct {
    uint32_t ino;
    uint32_t name_bytes, name_chars;
    uint8_t name[FS2_NAME_MAX_BYTES];
} fs2_dirent;

/* Formats the device (§9 format). For RAM volumes and tests; the host tool
 * tools/fs2/fs2.py formats disks. */
fnd_status fs2_format(fs_dev *dev, uint32_t total_blocks, uint32_t boot_blocks, uint32_t inode_count,
                      uint32_t root_mode, fs_time now);

/* §10: verifies everything before using the volume. DIRTY media are refused
 * with FND_E_NEEDS_FSCK. `writable` = 0 mounts read-only. */
fnd_status fs2_mount(fs2_vol *v, fs_dev *dev, uint32_t writable);
/* §9.1: writes DIRTY = 0 last. The VFS checks that nothing refers to it. */
fnd_status fs2_unmount(fs2_vol *v);

fnd_status fs2_inode_read(fs2_vol *v, uint32_t ino, fs2_inode *out);

/* Directory entries. `slot` is the entry index; fs2_dir_next returns
 * FND_END after the last used one. */
fnd_status fs2_dir_next(fs2_vol *v, uint32_t dir, uint32_t *slot, fs2_dirent *out);
fnd_status fs2_lookup(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t *ino,
                      fs2_dirent *stored);
fnd_status fs2_dir_empty(fs2_vol *v, uint32_t dir);

fnd_status fs2_read(fs2_vol *v, uint32_t ino, uint64_t off, void *buf, uint32_t len, uint32_t *got);
fnd_status fs2_write(fs2_vol *v, uint32_t ino, uint64_t off, const void *buf, uint32_t len, fs_time now);
fnd_status fs2_truncate(fs2_vol *v, uint32_t ino, uint64_t size, fs_time now);

/* `name` must already be a valid FS2 name (fnd_name_check). */
fnd_status fs2_create(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t type,
                      uint32_t mode, uint32_t uid, fs_time now, uint32_t *ino);
/* Removes a file (want_dir = 0) or an empty directory (want_dir = 1). */
fnd_status fs2_remove(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t want_dir,
                      fs_time now);
fnd_status fs2_rename(fs2_vol *v, uint32_t src_dir, const uint8_t *src, uint32_t src_len, uint32_t dst_dir,
                      const uint8_t *dst, uint32_t dst_len, fs_time now);
fnd_status fs2_set_owner(fs2_vol *v, uint32_t ino, uint32_t set_mode, uint32_t mode, uint32_t set_uid,
                         uint32_t uid, fs_time now);
fnd_status fs2_sync(fs2_vol *v);

/* 1 if `ino` is `anc` or below it. */
fnd_status fs2_is_under(fs2_vol *v, uint32_t ino, uint32_t anc, uint32_t *result);

#endif
