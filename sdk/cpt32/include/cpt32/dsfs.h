#ifndef CPT32_DSFS_H
#define CPT32_DSFS_H
#include <stdbool.h>
#include <stdint.h>

#define DSFS_OK 0u
#define DSFS_ERR_IO 1u
#define DSFS_ERR_MAGIC 2u
#define DSFS_ERR_VERSION 3u
#define DSFS_ERR_LAYOUT 4u
#define DSFS_ERR_NOT_FOUND 5u
#define DSFS_ERR_FULL 6u
#define DSFS_ERR_ARGS 7u
#define DSFS_DIR_BLOCKS 16u
#define DSFS_ENTRIES 256u
#define DSFS_ENTRY_SIZE 32u
#define DSFS_NAME_BYTES 11u
#define DSFS_SECTOR_SIZE 512u

/* DSFS keeps one mounted volume and one remembered bitmap buffer globally.
 * Mount/dir buffers hold at least 8192 bytes; bitmap buffers hold
 * dsfs_bitmap_blocks()*512 bytes; file buffers hold the file's sector-rounded
 * size. Names are exactly 11 space-padded 8.3 bytes, without a dot or NUL. */
void dsfs_set_time(uint32_t unix_seconds);
uint32_t dsfs_mount(uint32_t slot, void *buffer);
uint32_t dsfs_dir_load(void *buffer);
uint32_t dsfs_dir_save_sector(void *dir_buffer, uint32_t index);
uint32_t dsfs_bitmap_load(void *buffer);
uint32_t dsfs_bitmap_save(void);
uint32_t dsfs_bm_get(uint32_t block);
void dsfs_bm_set(uint32_t block);
void dsfs_bm_clear(uint32_t block);
uint8_t *dsfs_entry_addr(void *buffer, uint32_t index);
uint32_t dsfs_entry_status(const void *entry);
uint32_t dsfs_entry_start(const void *entry);
uint32_t dsfs_entry_size(const void *entry);
uint32_t dsfs_entry_created(const void *entry);
uint32_t dsfs_entry_modified(const void *entry);
bool dsfs_name_eq(const void *a, const void *b);
int32_t dsfs_find(const void *buffer, const void *name);
uint32_t dsfs_count(const void *buffer);
uint32_t dsfs_read_file(void *dir_buffer, const void *name, void *dest);
uint32_t dsfs_write_file(void *dir_buffer, void *bitmap_buffer,
                         const void *name, void *src, uint32_t bytes);
uint32_t dsfs_delete_file(void *dir_buffer, void *bitmap_buffer,
                          const void *name);

/* Read-only access to the singleton mount cache. bitmap_buffer is NULL until
 * dsfs_bitmap_load has been called successfully or attempted. */
bool dsfs_is_mounted(void);
uint32_t dsfs_slot(void);
uint32_t dsfs_total_blocks(void);
uint32_t dsfs_bitmap_start(void);
uint32_t dsfs_bitmap_blocks(void);
uint32_t dsfs_dir_start(void);
uint32_t dsfs_data_start(void);
uint32_t dsfs_time(void);
uint8_t *dsfs_bitmap_buffer(void);
#endif
