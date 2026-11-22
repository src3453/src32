/* The fs service (spec_Foundation_API.md §6, §7; spec_FS2.md §11). It
 * mounts the disks the disk service reports, a RAM volume at /tmp, and
 * serves the protocol of server.c on the PORT registered as "fs".
 *
 * Mounts at start-up: IDC slot n is /dd/<n> when it holds a valid FS2 or
 * DSFS volume, and the FS2 volume of the boot drive (sys_info boot_drive)
 * is also "/". A volume that fails the enabled mount checks is not shown
 * (KAN-38: boot/build.py temporarily skips the full FS2 §10 scan);
 * DIRTY media need an offline fsck (FND_E_NEEDS_FSCK). Nothing is logged:
 * debug_write faults on machines without a UART (src32_testbench). */
#include <string.h>

#include <foundation/sys.h>

#include "../services/disk/disk.h"
#include "server.h"

/* /tmp (§11.1): an FS2 volume in RAM, uid 0, mode 0x003F. fs lives in a
 * 512 KiB slot whose heap (about 110 KiB after the image and the stack) holds
 * two fs2_vol (about 21 KiB each) and this image; 128 sectors left almost
 * nothing for the directory indexes once /bin had 14 commands (KAN-17). */
#define TMP_SECTORS 96u
#define TMP_INODES 32u

static uint8_t req_data[FS_IPC_MAX] __attribute__((aligned(4)));
static uint32_t disk_channel;

static fs_dev disk_devs[4];
static uint32_t disk_slots[4] = {0, 1, 2, 3};
static fs_dev tmp_dev;
static uint8_t *tmp_image;

/* ---- platform (fs.h, server.h) ---------------------------------------- */

void *fs_alloc(uint32_t size) {
    void *p = fnd_mem_alloc(size ? size : 1u);
    if (p)
        fs_zero(p, size);
    return p;
}

void fs_free(void *p) { fnd_mem_free(p); }

/* §7: the RTC gives seconds only, so ms = 0. */
fs_time fs_now(void) {
    fs_time t;
    uint32_t lo = (uint32_t)fnd_sys_time_get(FND_TIME_RTC_LO); /* latches HI */
    uint32_t hi = (uint32_t)fnd_sys_time_get(FND_TIME_RTC_HI);
    t.sec = (int64_t)(((uint64_t)hi << 32) | lo);
    t.ms = 0;
    return t;
}

/* §6.4, §9: fs asks the device service for a stream and forwards it. */
fnd_status fs_dev_open(const char *service, uint32_t flags, uint32_t *handle) {
    int32_t ch = fnd_sys_svc_lookup(service);
    if (ch < 0)
        return ch == FND_E_NOT_FOUND ? FND_E_NO_MEDIA : ch;
    uint32_t args[4] = {flags, 0, 0, 0};
    fnd_msg rep;
    fnd_status st = fnd_call((fnd_handle)ch, FND_DEV_OPEN, args, 0, 0, 0, 0, &rep);
    fnd_sys_handle_close((uint32_t)ch);
    if (st != FND_OK)
        return st;
    if (!(rep.flags & FND_MSG_XFER_HANDLE) || !rep.a3)
        return FND_E_IO;
    *handle = rep.a3;
    return FND_OK;
}

/* ---- block devices ------------------------------------------------------ */

static fnd_status disk_read(void *ctx, uint32_t lba, uint32_t count, void *buf) {
    uint32_t drive = *(const uint32_t *)ctx;
    while (count) {
        uint32_t n = count < DISK_MAX_SECTORS ? count : DISK_MAX_SECTORS;
        uint32_t args[4] = {drive, lba, n, 0};
        fnd_msg rep;
        fnd_status st = fnd_call(disk_channel, DISK_READ, args, 0, 0, buf, n * 512u, &rep);
        if (st != FND_OK)
            return st;
        if (rep.len != n * 512u)
            return FND_E_IO;
        buf = (uint8_t *)buf + n * 512u;
        lba += n;
        count -= n;
    }
    return FND_OK;
}

static fnd_status disk_write(void *ctx, uint32_t lba, uint32_t count, const void *buf) {
    uint32_t drive = *(const uint32_t *)ctx;
    while (count) {
        uint32_t n = count < DISK_MAX_SECTORS ? count : DISK_MAX_SECTORS;
        uint32_t args[4] = {drive, lba, 0, 0};
        fnd_status st = fnd_call(disk_channel, DISK_WRITE, args, buf, n * 512u, 0, 0, 0);
        if (st != FND_OK)
            return st;
        buf = (const uint8_t *)buf + n * 512u;
        lba += n;
        count -= n;
    }
    return FND_OK;
}

static fnd_status ram_read(void *ctx, uint32_t lba, uint32_t count, void *buf) {
    fs_copy(buf, (uint8_t *)ctx + lba * 512u, count * 512u);
    return FND_OK;
}

static fnd_status ram_write(void *ctx, uint32_t lba, uint32_t count, const void *buf) {
    fs_copy((uint8_t *)ctx + lba * 512u, buf, count * 512u);
    return FND_OK;
}

static void mount_disks(void) {
    uint8_t info[32] __attribute__((aligned(4)));
    uint32_t boot = 0xFFFFFFFFu;
    if (fnd_sys_info(FND_INFO_SYSTEM, info, sizeof info) >= 0)
        boot = ((uint32_t)info[24] << 24) | ((uint32_t)info[25] << 16) | ((uint32_t)info[26] << 8) | info[27];
    /* The kernel starts disk before fs; give it a moment to register. */
    int32_t ch = FND_E_NOT_FOUND;
    for (uint32_t tries = 0; tries < 10u && ch < 0; ++tries) {
        ch = fnd_sys_svc_lookup("disk");
        if (ch < 0)
            fnd_sys_sleep(1u); /* up to 100 ms, in 10 ms ticks */
    }
    if (ch < 0)
        return;
    disk_channel = (uint32_t)ch;
    for (uint32_t d = 0; d < 4; ++d) {
        uint32_t args[4] = {d, 0, 0, 0};
        fnd_msg rep;
        if (fnd_call(disk_channel, DISK_IDENTIFY, args, 0, 0, 0, 0, &rep) != FND_OK ||
            !(rep.a3 & DISK_FLAG_PRESENT) || rep.a1 != 512u)
            continue;
        fs_dev *dev = &disk_devs[d];
        dev->ctx = &disk_slots[d];
        dev->sectors = rep.a2;
        dev->read_only = (rep.a3 & DISK_FLAG_READ_ONLY) != 0;
        dev->read = disk_read;
        dev->write = disk_write;
        fs2_vol *vol = (fs2_vol *)fs_alloc(sizeof *vol);
        fnd_status st = vol ? fs2_mount(vol, dev, 1) : FND_E_NO_MEMORY;
        if (st == FND_OK) {
            vfs_mount_fs2(VFS_DD0 + d, vol, FND_VOL_FS2, d);
            if (d == boot)
                vfs_mount_fs2(VFS_ROOT, vol, FND_VOL_FS2, d);
            continue;
        }
        fs_free(vol);
        if (st == FND_E_FORMAT) {
            dsfs_vol *dv = (dsfs_vol *)fs_alloc(sizeof *dv);
            fnd_status ds = dv ? dsfs_mount(dv, dev) : FND_E_NO_MEMORY;
            if (ds == FND_OK) {
                vfs_mount_dsfs(VFS_DD0 + d, dv, d);
                continue;
            }
            fs_free(dv);
        }
    }
}

static void mount_tmp(void) {
    tmp_image = (uint8_t *)fs_alloc(TMP_SECTORS * 512u);
    fs2_vol *vol = (fs2_vol *)fs_alloc(sizeof *vol);
    if (!tmp_image || !vol)
        return;
    tmp_dev.ctx = tmp_image;
    tmp_dev.sectors = TMP_SECTORS;
    tmp_dev.read = ram_read;
    tmp_dev.write = ram_write;
    if (fs2_format(&tmp_dev, TMP_SECTORS, 0, TMP_INODES, 0x003Fu, fs_now()) == FND_OK &&
        fs2_mount(vol, &tmp_dev, 1) == FND_OK)
        vfs_mount_fs2(VFS_TMP, vol, FND_VOL_RAM, FND_VOL_NO_SLOT);
}

/* ---- /net: the workers that talk to netfs (doc/spec_NET_FP.md §11) ------ */

/* A request below /net/<name> may wait up to 15 s for the network, so the
 * main loop does not call netfs itself: it hands the request (an fs_net_job
 * from server.c) and the client's token to a worker over a private PORT and
 * goes on serving. The worker calls netfs and replies to the token. With
 * NET_WORKERS busy, the main loop waits for one to finish. */
#define NET_WORKERS 2u
#define NET_JOB_MAX (sizeof(fs_net_job) + FS_IPC_MAX)
#define NET_STACK_WORDS 512u

typedef struct {
    uint8_t job[NET_JOB_MAX] __attribute__((aligned(4)));
    uint8_t rep[FND_STAT_BYTES] __attribute__((aligned(4)));
    uint32_t netfs; /* CHANNEL from svc_lookup("netfs"), or 0 */
    uint32_t stack[NET_STACK_WORDS] __attribute__((aligned(8)));
} net_worker;

static net_worker *net_workers;
static uint32_t job_port, job_chan, internal_chan;

/* One call to netfs; finds it again if it was restarted. */
static fnd_status netfs_call(net_worker *w, fnd_msg *req, fnd_msg *rep) {
    for (uint32_t tries = 0; tries < 2u; ++tries) {
        if (!w->netfs) {
            int32_t h = fnd_sys_svc_lookup("netfs");
            if (h < 0)
                return FND_E_NOT_FOUND; /* no netfs: nothing is mounted */
            w->netfs = (uint32_t)h;
        }
        fnd_msg out = *req;
        memset(rep, 0, sizeof *rep);
        rep->buf = (uint32_t)(uintptr_t)w->rep;
        rep->len = sizeof w->rep;
        fnd_status st = fnd_sys_ipc_call(w->netfs, &out, rep);
        if (st != FND_E_PEER_GONE && st != FND_E_BAD_HANDLE)
            return st;
        fnd_sys_handle_close(w->netfs);
        w->netfs = 0;
    }
    return FND_E_NOT_FOUND;
}

static void net_do(net_worker *w, uint32_t client) {
    const fs_net_job *j = (const fs_net_job *)w->job;
    const uint8_t *path = w->job + sizeof *j;
    uint32_t cwd_set = j->fs_op == FND_FS_CWD_SET;
    uint32_t op = cwd_set ? FND_FS_STAT : j->fs_op;
    fnd_msg req = {FND_NET_FS + (op & 0xFFu), 0, j->a0, j->a1, j->a2, j->a3, (uint32_t)(uintptr_t)path, j->len};
    fnd_msg rep, out;
    memset(&out, 0, sizeof out);
    fnd_status st = netfs_call(w, &req, &rep);
    if (st != FND_OK) {
        out.op = (uint32_t)st;
    } else if (cwd_set) {
        /* §6.3: a directory; the main loop owns the session, so it sets the
         * cwd on the internal CHANNEL. */
        out.op = rep.op;
        if ((int32_t)rep.op == FND_OK) {
            if (rep.len < FND_STAT_BYTES || fs_be32(w->rep) != FND_TYPE_DIR) {
                out.op = (uint32_t)FND_E_NOT_DIR;
            } else {
                uint32_t n = j->len;
                while (n > 0 && path[n - 1] == '/')
                    --n;
                /* FS_NET_CWD data: badge, "/net/" and the path, built in
                 * place in front of the path (the header is 32 bytes). */
                uint8_t *msg = (uint8_t *)path - 9;
                uint32_t session = j->session;
                memcpy(msg, &session, 4);
                memcpy(msg + 4, "/net/", 5);
                out.op = (uint32_t)fnd_call(internal_chan, FS_NET_CWD, 0, msg, 9 + n, 0, 0, 0);
            }
        }
        if ((rep.flags & FND_MSG_XFER_HANDLE) && rep.a3)
            fnd_sys_handle_close(rep.a3);
    } else {
        out = rep;
        out.buf = (uint32_t)(uintptr_t)w->rep;
        if (out.len > j->reply_cap)
            out.len = 0, out.op = (uint32_t)FND_E_LIMIT;
    }
    if (fnd_sys_ipc_reply(client, &out) != FND_OK && (out.flags & FND_MSG_XFER_HANDLE) && out.a3)
        fnd_sys_handle_close(out.a3); /* the client is gone */
}

static void net_worker_main(net_worker *w) {
    for (;;) {
        fnd_recv r;
        memset(&r, 0, sizeof r);
        r.data = (uint32_t)(uintptr_t)w->job;
        r.cap = sizeof w->job;
        r.timeout = FND_FOREVER;
        if (fnd_sys_ipc_recv(job_port, &r) != FND_OK || !r.token)
            continue;
        fnd_msg ack;
        memset(&ack, 0, sizeof ack);
        uint32_t client = r.msg.a0;
        int ok = r.msg.len >= sizeof(fs_net_job) &&
                 r.msg.len == sizeof(fs_net_job) + ((const fs_net_job *)w->job)->len;
        ack.op = ok ? FND_OK : (uint32_t)FND_E_BAD_ARG;
        fnd_sys_ipc_reply(r.token, &ack); /* the main loop goes on */
        if (ok)
            net_do(w, client);
    }
}

/* One entry per worker: the thread's argument register is not part of the
 * C ABI the toolchain documents. */
static void net_worker0(void) { net_worker_main(&net_workers[0]); }
static void net_worker1(void) { net_worker_main(&net_workers[1]); }

static void net_start(uint32_t port) {
    int32_t jp = fnd_sys_port_create();
    if (jp < 0)
        return;
    int32_t jc = fnd_sys_chan_create((uint32_t)jp, 1u);
    uint32_t badge = fs_server_internal();
    int32_t ic = badge ? fnd_sys_chan_create(port, badge) : FND_E_LIMIT;
    net_workers = (net_worker *)fs_alloc(NET_WORKERS * sizeof(net_worker));
    if (jc < 0 || ic < 0 || !net_workers)
        return; /* /net then answers NOT_FOUND below its mounts */
    job_port = (uint32_t)jp;
    job_chan = (uint32_t)jc;
    internal_chan = (uint32_t)ic;
    fnd_sys_thread_create(net_worker0, &net_workers[0].stack[NET_STACK_WORDS - 2u], 0);
    fnd_sys_thread_create(net_worker1, &net_workers[1].stack[NET_STACK_WORDS - 2u], 0);
}

/* Gives the forwarded request to a worker; FND_OK if one took it. */
static fnd_status net_forward(uint32_t token, const fs_rep *a) {
    if (!job_chan)
        return FND_E_NOT_FOUND;
    fnd_msg req = {1u, 0, token, 0, 0, 0, (uint32_t)(uintptr_t)a->data, a->len};
    fnd_msg ack;
    memset(&ack, 0, sizeof ack);
    fnd_status st = fnd_sys_ipc_call(job_chan, &req, &ack);
    return st != FND_OK ? st : (fnd_status)ack.op;
}

/* ---- the PORT ------------------------------------------------------------- */

int main(void) {
    fnd_init();
    vfs_init();
    fs_server_init();
    mount_disks();
    mount_tmp();
    int32_t port = fnd_sys_port_create();
    if (port < 0 || fnd_sys_svc_register("fs", (uint32_t)port) != FND_OK)
        fnd_exit(FND_E_BAD_HANDLE);
    net_start((uint32_t)port);
    fnd_recv r;
    fnd_msg rep;
    uint32_t token = 0, sent = 0;
    for (;;) {
        memset(&r, 0, sizeof r);
        r.data = (uint32_t)(uintptr_t)req_data;
        r.cap = sizeof req_data;
        r.timeout = FND_FOREVER;
        int32_t st;
        if (token && sent) {
            /* §8.4: a reply whose transfer fails, or whose client is gone,
             * leaves the handle with fs; closing it brings CLIENT_GONE,
             * which frees the object. */
            if (fnd_sys_ipc_reply(token, &rep) != FND_OK)
                fnd_sys_handle_close(sent);
            st = fnd_sys_ipc_recv((uint32_t)port, &r);
        } else if (token) {
            st = fnd_sys_ipc_reply_recv(token, &rep, (uint32_t)port, &r);
        } else {
            st = fnd_sys_ipc_recv((uint32_t)port, &r);
        }
        token = sent = 0;
        if (st != FND_OK)
            continue;
        if (r.token == 0) {
            if (r.msg.op == FND_NOTIFY_CLIENT_GONE)
                fs_server_gone(r.badge);
            continue;
        }
        fs_req q = {r.msg.op, r.msg.flags, r.msg.a0, r.msg.a1, r.msg.a2, r.msg.a3};
        fs_rep a;
        if (r.msg.flags & FND_MSG_XFER_HANDLE)
            fnd_sys_handle_close(r.msg.a3); /* fs takes no handles; the request fails */
        fs_server_request(r.badge, &q, req_data, r.msg.len, r.total, r.reply_cap, &a);
        memset(&rep, 0, sizeof rep);
        if (a.net) {
            fnd_status st = net_forward(r.token, &a);
            if (st == FND_OK)
                continue; /* the worker replies */
            memset(&a, 0, sizeof a);
            a.op = (uint32_t)st;
        }
        if (a.new_badge) {
            int32_t h = fnd_sys_chan_create((uint32_t)port, a.new_badge);
            if (h < 0) {
                fs_server_gone(a.new_badge);
                a.op = (uint32_t)h;
                a.a0 = 0, a.data = 0, a.len = 0;
            } else {
                sent = (uint32_t)h;
            }
        } else if (a.fwd_handle) {
            sent = a.fwd_handle;
        }
        rep.op = a.op;
        rep.a0 = a.a0, rep.a1 = a.a1, rep.a2 = a.a2, rep.a3 = a.a3;
        if (sent) {
            rep.flags = FND_MSG_XFER_HANDLE;
            rep.a3 = sent;
        }
        rep.buf = (uint32_t)(uintptr_t)a.data;
        rep.len = a.len;
        token = r.token;
    }
}
