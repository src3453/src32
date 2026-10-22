#ifndef FOUNDATION_FS_FS_H
#define FOUNDATION_FS_FS_H
#include <stdint.h>

#include <foundation/fnd.h>

/* Pieces of the fs service that do not depend on IPC: the FS2 driver
 * (fs2.c), the VFS (vfs.c) and the protocol (proto.c). They build for the
 * guest and, for tests, with the host C compiler (tests/fs). The service
 * main loop (main.c) and the platform functions below tie them to the
 * kernel. */

typedef struct {
    int64_t sec; /* UNIX seconds */
    uint32_t ms; /* 0..999 */
} fs_time;

/* A block device of 512-byte sectors. read/write return FND_OK or an error
 * (FND_E_IO, FND_E_NO_MEDIA, FND_E_READ_ONLY). */
typedef struct fs_dev {
    void *ctx;
    uint32_t sectors;
    uint32_t read_only;
    fnd_status (*read)(void *ctx, uint32_t lba, uint32_t count, void *buf);
    fnd_status (*write)(void *ctx, uint32_t lba, uint32_t count, const void *buf);
} fs_dev;

/* Platform: the service (main.c) or the host test provides these. */
void *fs_alloc(uint32_t size); /* zeroed, 8-byte aligned; 0 if out of memory */
void fs_free(void *p);
fs_time fs_now(void);

static inline uint32_t fs_be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}
static inline uint32_t fs_be16(const uint8_t *p) { return ((uint32_t)p[0] << 8) | p[1]; }
static inline uint64_t fs_be64(const uint8_t *p) { return ((uint64_t)fs_be32(p) << 32) | fs_be32(p + 4); }
static inline void fs_put32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >> 8);
    p[3] = (uint8_t)v;
}
static inline void fs_put16(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 8);
    p[1] = (uint8_t)v;
}
static inline void fs_put64(uint8_t *p, uint64_t v) {
    fs_put32(p, (uint32_t)(v >> 32));
    fs_put32(p + 4, (uint32_t)v);
}

#endif
