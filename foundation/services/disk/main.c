/* The disk service (spec_FS2.md §11: fs -> disk -> kernel blk_io -> IDC).
 * It needs the DISK capability for blk_io and SVC to register "disk". The
 * protocol is in disk.h.
 *
 * Every IDC command costs the drive's access latency (spec_IDC.md: 10 ms on
 * an HDD, 100 ms on an FDD) while one call moves only 8 sectors, so a read
 * that misses fetches up to 128 sectors into one of four LRU windows.
 * Metadata, executable and WAL reads can then coexist. Writes reach the
 * drive at once and drop every overlapping window; errors drop all windows
 * and IDENTIFY drops the drive's windows. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "disk.h"
#include "cache.h"

#define BLK_READ 1u
#define BLK_WRITE 2u
#define BLK_IDENTIFY 3u

/* §10.4 fnd_blk. */
typedef struct {
    uint32_t drive, op, lba, count, buf, reserved;
} fnd_blk;

static uint8_t data[FND_IPC_MAX_DATA] __attribute__((aligned(4)));
static uint32_t capacity[4];                         /* from IDENTIFY, 0: unknown */
static uint32_t client;

static int32_t blk(uint32_t drive, uint32_t op, uint32_t lba, uint32_t count, void *buf) {
    fnd_blk b = {drive, op, lba, count, (uint32_t)(uintptr_t)buf, 0};
    return fnd_sys_blk_io(&b);
}

static int32_t read_sectors(uint32_t drive, uint32_t lba, uint32_t count, void *buf) {
    return blk(drive, BLK_READ, lba, count, buf);
}

/* Reads count sectors at lba; *out points at them. */
static int32_t disk_read(uint32_t drive, uint32_t lba, uint32_t count, const uint8_t **out) {
    return disk_cache_read(drive, lba, count, drive < 4 ? capacity[drive] : 0, read_sectors, out);
}

int main(void) {
    fnd_init();
    int32_t port = fnd_sys_port_create();
    if (port < 0 || fnd_sys_svc_register("disk", (uint32_t)port) != FND_OK)
        fnd_exit(FND_E_BAD_HANDLE);
    fnd_recv r = {{0}, (uint32_t)(uintptr_t)data, sizeof data, 0, 0, 0, 0, FND_FOREVER, 0};
    fnd_msg rep = {0};
    uint32_t token = 0;
    for (;;) {
        r.data = (uint32_t)(uintptr_t)data;
        r.cap = sizeof data;
        r.timeout = FND_FOREVER;
        int32_t st = token ? fnd_sys_ipc_reply_recv(token, &rep, (uint32_t)port, &r)
                           : fnd_sys_ipc_recv((uint32_t)port, &r);
        token = 0;
        if (st != FND_OK)
            continue;
        if (r.token == 0) {
            if (r.msg.op == FND_NOTIFY_CLIENT_GONE && r.badge == client)
                client = 0xFFFFFFFFu; /* the fs service is gone: nobody else */
            continue;
        }
        if (!client)
            client = r.badge;
        token = r.token;
        rep = (fnd_msg){0, 0, 0, 0, 0, 0, 0, 0};
        const fnd_msg *m = &r.msg;
        uint32_t count = m->op == DISK_WRITE ? m->len / 512u : m->a2;
        if (r.badge != client) {
            st = FND_E_ACCESS;
        } else if (m->flags || m->a3) {
            st = FND_E_BAD_ARG;
        } else if (m->op == DISK_IDENTIFY) {
            uint32_t info[4];
            st = m->a1 || m->a2 || m->len ? FND_E_BAD_ARG : blk(m->a0, BLK_IDENTIFY, 0, 0, info);
            if (m->a0 < 4)
                capacity[m->a0] = st == FND_OK && info[1] == 512u && (info[3] & DISK_FLAG_PRESENT) ? info[2] : 0;
            disk_cache_drop(m->a0); /* the media may have changed */
            if (st == FND_OK)
                rep.a0 = info[0], rep.a1 = info[1], rep.a2 = info[2], rep.a3 = info[3];
        } else if (m->op == DISK_READ || m->op == DISK_WRITE) {
            if (count == 0 || count > DISK_MAX_SECTORS || (m->op == DISK_READ && m->len) ||
                (m->op == DISK_WRITE && (m->a2 || m->len % 512u || r.total != m->len)))
                st = FND_E_BAD_ARG;
            else if (m->op == DISK_READ && r.reply_cap < count * 512u)
                st = FND_E_LIMIT;
            else if (m->op == DISK_READ) {
                const uint8_t *sectors;
                st = disk_read(m->a0, m->a1, count, &sectors);
                rep.buf = (uint32_t)(uintptr_t)sectors;
                rep.len = st == FND_OK ? count * 512u : 0;
            } else {
                disk_cache_write(m->a0, m->a1, count);
                st = blk(m->a0, BLK_WRITE, m->a1, count, data);
                if (st == FND_OK)
                    rep.a0 = count;
            }
            if (st != FND_OK)
                disk_cache_drop(DISK_CACHE_ALL);
        } else {
            st = FND_E_UNSUPPORTED;
        }
        rep.op = (uint32_t)st;
    }
}
