#ifndef FOUNDATION_DISK_CACHE_H
#define FOUNDATION_DISK_CACHE_H
#include <stdint.h>

/* Four LRU windows (256 KiB total), private to the single-threaded disk
 * service. Returned bytes remain valid until the next cache operation. */
#define DISK_CACHE_WINDOWS 4u
#define DISK_CACHE_SECTORS 128u
#define DISK_CACHE_ALL 0xFFFFFFFFu
typedef int32_t (*disk_read_fn)(uint32_t drive, uint32_t lba, uint32_t count, void *buf);

int32_t disk_cache_read(uint32_t drive, uint32_t lba, uint32_t count, uint32_t capacity,
                        disk_read_fn read, const uint8_t **out);
void disk_cache_drop(uint32_t drive);
/* Invalidate every overlapping window before attempting a write, including
 * windows on which a partial write could leave old bytes after an error. */
void disk_cache_write(uint32_t drive, uint32_t lba, uint32_t count);
#endif
