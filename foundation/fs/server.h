#ifndef FOUNDATION_FS_SERVER_H
#define FOUNDATION_FS_SERVER_H
#include <stdint.h>

#include "vfs.h"

/* The fs protocol (doc/spec_Foundation_API.md §5-§7) without the IPC: the
 * main loop (main.c) receives a request on the fs PORT, hands its badge,
 * words and data to fs_server_request, and sends back what it fills in.
 *
 * Objects are sessions, open files and open directories. Each is one
 * CHANNEL to the fs PORT and is known by its badge (§4). Sessions reached
 * with svc_lookup("fs") carry a kernel badge (bit 31) and are created on
 * their first request. */

#define FS_IPC_MAX 4096u
#define FS_OBJECTS 128u

typedef struct {
    uint32_t op, flags, a0, a1, a2, a3;
} fs_req;

typedef struct {
    uint32_t op; /* the result */
    uint32_t a0, a1, a2, a3;
    /* At most one of these is set; the main loop then sends a3 with
     * XFER_HANDLE. new_badge: create a CHANNEL with this badge and send it
     * (if chan_create fails, call fs_server_gone(new_badge) and reply with
     * its error instead). fwd_handle: a handle fs already holds, from a
     * device service (§6.4). */
    uint32_t new_badge;
    uint32_t fwd_handle;
    const uint8_t *data;
    uint32_t len;
    /* net = 1: the request is below /net/<name> and goes to netfs (doc/
     * spec_NET_FP.md §11). The main loop does not reply: it hands the
     * fs_net_job in [data, data + len) and the client's token to its
     * worker thread, which calls netfs and replies to the token itself, so
     * that fs keeps serving local paths while the network answers. */
    uint32_t net;
} fs_rep;

/* A forwarded request. The netfs operation is FND_NET_FS + (fs_op & 0xFF)
 * with a0..a3 and the path below /net ("name/a/b", a trailing '/' when the
 * caller's path named a directory) as data; FS_RENAME sends both paths with
 * a0 = the length of the first. FS_CWD_SET is sent as FS_STAT, and the
 * worker then sets the session's cwd with FS_NET_CWD on its internal
 * CHANNEL (fs_server_internal). */
typedef struct {
    uint32_t fs_op;
    uint32_t a0, a1, a2, a3;
    uint32_t session;   /* the session's badge (FS_CWD_SET) */
    uint32_t reply_cap; /* the client's */
    uint32_t len;       /* path bytes that follow */
} fs_net_job;

/* fs-internal operation on the CHANNEL of fs_server_internal: data = the
 * session badge (u32, host order) and the absolute cwd path. */
#define FS_NET_CWD 0x05F0u

/* Creates the internal object whose badge the worker's CHANNEL carries;
 * 0 if the object table is full. */
uint32_t fs_server_internal(void);

void fs_server_init(void);

/* `len` bytes of the request data arrived out of `total` sent; reply_cap is
 * the client's reply capacity. */
void fs_server_request(uint32_t badge, const fs_req *q, const uint8_t *data, uint32_t len, uint32_t total,
                       uint32_t reply_cap, fs_rep *r);

/* FND_NOTIFY_CLIENT_GONE: the last handle of the badge's CHANNEL closed. */
void fs_server_gone(uint32_t badge);

/* Platform: opens a stream of a device service (DEV_OPEN, §9) and returns
 * the stream handle, which fs then forwards to the client. */
fnd_status fs_dev_open(const char *service, uint32_t flags, uint32_t *handle);

#endif
