#ifndef FOUNDATION_FS_DSFS_H
#define FOUNDATION_FS_DSFS_H
#include <stdint.h>

#include "fs.h"

/* Read-only DSFS driver (doc/spec_DSFS.md) for the VFS (spec_FS2.md §11:
 * "DSFS driver (read-only at first)"). The 256-entry directory stays in
 * memory after the mount check. */

#define DSFS_ENTRIES 256u
#define DSFS_DIR_BYTES 8192u
#define DSFS_NAME_MAX 12u /* NAME.EXT */

typedef struct {
    fs_dev *dev;
    uint32_t total_blocks, data_start;
    uint32_t free_blocks;
    uint8_t *dir; /* DSFS_DIR_BYTES */
} dsfs_vol;

typedef struct {
    uint32_t used;
    uint32_t start, size;
    uint32_t created, modified;
    uint8_t name[DSFS_NAME_MAX];
    uint32_t name_len;
} dsfs_entry;

/* §7. FND_E_FORMAT if the media is not a valid DSFS volume. */
fnd_status dsfs_mount(dsfs_vol *v, fs_dev *dev);
void dsfs_unmount(dsfs_vol *v);
void dsfs_entry_get(const dsfs_vol *v, uint32_t index, dsfs_entry *e);
fnd_status dsfs_read(dsfs_vol *v, uint32_t index, uint64_t off, void *buf, uint32_t len, uint32_t *got);

#endif
