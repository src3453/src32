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
} fs_rep;

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
