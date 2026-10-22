/* Host harness of the fs service core (foundation/fs) for test_fs.py:
 * RAM block devices over buffers that Python owns, the platform functions
 * of fs.h, and flat entry points for ctypes. */
#include <stdlib.h>
#include <string.h>

#include "server.h"

#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT
#endif

typedef struct {
    uint8_t *data;
    uint32_t fail_after; /* writes left before FND_E_IO; 0xFFFFFFFF = never */
    uint32_t writes;
} memdev;

static fnd_status mem_read(void *ctx, uint32_t lba, uint32_t count, void *buf) {
    memdev *m = (memdev *)ctx;
    memcpy(buf, m->data + (size_t)lba * 512u, (size_t)count * 512u);
    return FND_OK;
}

static fnd_status mem_write(void *ctx, uint32_t lba, uint32_t count, const void *buf) {
    memdev *m = (memdev *)ctx;
    if (m->fail_after != 0xFFFFFFFFu) {
        if (m->fail_after == 0)
            return FND_E_IO;
        --m->fail_after;
    }
    ++m->writes;
    memcpy(m->data + (size_t)lba * 512u, buf, (size_t)count * 512u);
    return FND_OK;
}

static memdev mems[VFS_MOUNTS];
static fs_dev devs[VFS_MOUNTS];
static fs2_vol *vols[VFS_MOUNTS];
static fs_time now_value;
static uint32_t dev_open_handle = 77, dev_open_flags;
static int32_t dev_open_status;

void *fs_alloc(uint32_t size) { return calloc(1, size ? size : 1); }
void fs_free(void *p) { free(p); }
fs_time fs_now(void) { return now_value; }

fnd_status fs_dev_open(const char *service, uint32_t flags, uint32_t *handle) {
    (void)service;
    dev_open_flags = flags;
    if (dev_open_status)
        return dev_open_status;
    *handle = dev_open_handle;
    return FND_OK;
}

EXPORT void host_init(void) {
    for (uint32_t i = 0; i < VFS_MOUNTS; ++i) {
        free(vols[i]);
        vols[i] = 0;
    }
    vfs_init();
    fs_server_init();
}

EXPORT void host_set_time(int64_t sec, uint32_t ms) {
    now_value.sec = sec;
    now_value.ms = ms;
}

EXPORT uint32_t host_dev_open_flags(void) { return dev_open_flags; }

static void dev_setup(uint32_t mnt, uint8_t *image, uint32_t sectors, uint32_t read_only) {
    mems[mnt].data = image;
    mems[mnt].fail_after = 0xFFFFFFFFu;
    mems[mnt].writes = 0;
    devs[mnt].ctx = &mems[mnt];
    devs[mnt].sectors = sectors;
    devs[mnt].read_only = read_only;
    devs[mnt].read = mem_read;
    devs[mnt].write = mem_write;
}

/* Mounts an FS2 image at `mnt`. `alias` >= 0 also attaches the same volume
 * there (the boot disk at "/" and /dd/<n>). */
EXPORT int32_t host_attach(uint32_t mnt, uint8_t *image, uint32_t sectors, uint32_t writable, uint32_t read_only,
                           uint32_t fs_type, uint32_t slot, int32_t alias) {
    dev_setup(mnt, image, sectors, read_only);
    vols[mnt] = (fs2_vol *)calloc(1, sizeof(fs2_vol));
    fnd_status st = fs2_mount(vols[mnt], &devs[mnt], writable);
    if (st != FND_OK)
        return st;
    vfs_mount_fs2(mnt, vols[mnt], fs_type, slot);
    if (alias >= 0)
        vfs_mount_fs2((uint32_t)alias, vols[mnt], fs_type, slot);
    return FND_OK;
}

EXPORT int32_t host_format(uint8_t *image, uint32_t sectors, uint32_t total, uint32_t boot_blocks,
                           uint32_t inodes, uint32_t root_mode) {
    memdev m = {image, 0xFFFFFFFFu, 0};
    fs_dev d = {&m, sectors, 0, mem_read, mem_write};
    return fs2_format(&d, total, boot_blocks, inodes, root_mode, now_value);
}

EXPORT int32_t host_unmount(uint32_t mnt) { return fs2_unmount(vols[mnt]); }

EXPORT void host_fail_writes_after(uint32_t mnt, uint32_t n) { mems[mnt].fail_after = n; }
EXPORT uint32_t host_writes(uint32_t mnt) { return mems[mnt].writes; }

/* out: op, a0, a1, a2, a3, new_badge, fwd_handle, len. */
EXPORT void host_request(uint32_t badge, uint32_t op, uint32_t flags, uint32_t a0, uint32_t a1, uint32_t a2,
                         uint32_t a3, const uint8_t *data, uint32_t len, uint32_t total, uint32_t reply_cap,
                         uint32_t *out, uint8_t *reply) {
    fs_req q = {op, flags, a0, a1, a2, a3};
    fs_rep r;
    fs_server_request(badge, &q, data, len, total, reply_cap, &r);
    out[0] = r.op, out[1] = r.a0, out[2] = r.a1, out[3] = r.a2, out[4] = r.a3;
    out[5] = r.new_badge, out[6] = r.fwd_handle, out[7] = r.len;
    if (r.len)
        memcpy(reply, r.data, r.len);
}

EXPORT void host_gone(uint32_t badge) { fs_server_gone(badge); }
