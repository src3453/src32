/* The pipe service (doc/spec_Foundation_API.md §9a): anonymous pipes behind the stream protocol.
 *
 * It registers as "pipe". A channel from svc_lookup only takes PIPE_OPEN, which makes a pipe (a0 = 0:
 * replies the read end and a0 = the pipe id) or attaches the write end of an existing pipe (a0 = 1,
 * a1 = id). An end is a channel of the service's port whose badge names it, so the ends survive
 * fork (the handles are shared) and dup, and CLIENT_GONE (the last handle of an end closed)
 * is what makes EOF and EPIPE. ST_READ and ST_WRITE wait in one queue and are answered when
 * data or room arrives. A write is atomic: it waits until the whole request fits. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#define PIPES 32u
#define PIPE_SIZE FND_IPC_MAX_DATA /* a write of the largest request can always fit in the end */
#define PENDING 32u
#define LOOKUP_BADGE 0x80000000u

struct pipe {
    uint32_t id; /* 0: free */
    uint32_t read_badge, write_badge; /* 0: not open (never attached, or closed) */
    uint32_t head, count;
    uint8_t buf[PIPE_SIZE];
};

struct pending {
    uint32_t token, badge; /* badge 0: free */
    uint32_t op;
    uint32_t max; /* ST_READ: bytes asked for; ST_WRITE: bytes in wbuf */
    uint8_t wbuf[FND_IPC_MAX_DATA];
};

static struct pipe pipes[PIPES];
static struct pending pending[PENDING];
static uint32_t next_id, next_badge;
static uint8_t data[FND_IPC_MAX_DATA];
static uint8_t out[FND_IPC_MAX_DATA];

static void reply(uint32_t token, int32_t result, uint32_t a0, const void *buf, uint32_t len) {
    fnd_msg msg = {(uint32_t)result, 0, a0, 0, 0, 0, (uint32_t)(uintptr_t)buf, len};
    fnd_sys_ipc_reply(token, &msg); /* a client that is gone needs no answer */
}

static struct pipe *by_badge(uint32_t badge, int *is_write) {
    for (uint32_t i = 0; badge && i < PIPES; ++i) {
        if (pipes[i].id && pipes[i].read_badge == badge) {
            *is_write = 0;
            return &pipes[i];
        }
        if (pipes[i].id && pipes[i].write_badge == badge) {
            *is_write = 1;
            return &pipes[i];
        }
    }
    return 0;
}

static uint32_t new_badge(void) {
    /* §4.4: a badge is not reused right after it is freed. */
    next_badge = next_badge >= 0x7FFFFFFFu ? 1u : next_badge + 1u;
    return next_badge;
}

/* Answers every waiting request that the pipe's state now settles. */
static void settle(struct pipe *p) {
    for (uint32_t i = 0; i < PENDING; ++i) {
        struct pending *q = &pending[i];
        if (!q->badge)
            continue;
        if (q->op == FND_ST_READ && q->badge == p->read_badge) {
            if (p->count) {
                uint32_t n = p->count < q->max ? p->count : q->max;
                for (uint32_t k = 0; k < n; ++k)
                    out[k] = p->buf[(p->head + k) % PIPE_SIZE];
                p->head = (p->head + n) % PIPE_SIZE;
                p->count -= n;
                reply(q->token, FND_OK, 0, out, n);
                q->badge = 0;
            } else if (!p->write_badge) { /* EOF */
                reply(q->token, FND_OK, 0, 0, 0);
                q->badge = 0;
            }
        } else if (q->op == FND_ST_WRITE && q->badge == p->write_badge) {
            if (!p->read_badge) {
                reply(q->token, FND_E_PEER_GONE, 0, 0, 0);
                q->badge = 0;
            } else if (p->count + q->max <= PIPE_SIZE) {
                for (uint32_t k = 0; k < q->max; ++k)
                    p->buf[(p->head + p->count + k) % PIPE_SIZE] = q->wbuf[k];
                p->count += q->max;
                reply(q->token, FND_OK, q->max, 0, 0);
                q->badge = 0;
            }
        }
    }
}

static void settle_all(void) {
    /* A read that was answered may make room for a write, and the reverse, so go around until quiet. */
    for (uint32_t round = 0; round < 2u; ++round) {
        for (uint32_t i = 0; i < PIPES; ++i) {
            if (pipes[i].id)
                settle(&pipes[i]);
        }
    }
}

static struct pending *queue(const fnd_recv *r, uint32_t op, uint32_t max) {
    for (uint32_t i = 0; i < PENDING; ++i) {
        if (!pending[i].badge) {
            pending[i].token = r->token;
            pending[i].badge = r->badge;
            pending[i].op = op;
            pending[i].max = max;
            return &pending[i];
        }
    }
    return 0;
}

/* PIPE_OPEN on a lookup channel; replies itself on success. */
static int32_t pipe_open(fnd_handle port, const fnd_recv *r) {
    uint32_t mode = r->msg.a0, id = r->msg.a1;
    struct pipe *p = 0;
    if (mode == 0) {
        for (uint32_t i = 0; i < PIPES; ++i) {
            if (!pipes[i].id) {
                p = &pipes[i];
                break;
            }
        }
        if (!p)
            return FND_E_LIMIT;
        next_id = next_id >= 0x7FFFFFFFu ? 1u : next_id + 1u;
        *p = (struct pipe){0};
        p->id = next_id;
        p->read_badge = new_badge();
        int32_t ch = fnd_sys_chan_create(port, p->read_badge);
        if (ch < 0) {
            p->id = 0;
            return ch;
        }
        fnd_msg msg = {FND_OK, FND_MSG_XFER_HANDLE, p->id, 0, 0, (uint32_t)ch, 0, 0};
        if (fnd_sys_ipc_reply(r->token, &msg) != FND_OK) {
            fnd_sys_handle_close((uint32_t)ch);
            p->id = 0;
        }
        return FND_OK;
    }
    if (mode != 1)
        return FND_E_BAD_ARG;
    for (uint32_t i = 0; i < PIPES; ++i) {
        if (pipes[i].id == id && id) {
            p = &pipes[i];
            break;
        }
    }
    if (!p || p->write_badge)
        return FND_E_NOT_FOUND;
    uint32_t badge = new_badge();
    int32_t ch = fnd_sys_chan_create(port, badge);
    if (ch < 0)
        return ch;
    fnd_msg msg = {FND_OK, FND_MSG_XFER_HANDLE, p->id, 0, 0, (uint32_t)ch, 0, 0};
    if (fnd_sys_ipc_reply(r->token, &msg) != FND_OK) {
        fnd_sys_handle_close((uint32_t)ch);
        return FND_OK;
    }
    p->write_badge = badge;
    return FND_OK;
}

/* Returns 1 if the request was answered (or will be later). */
static int handle(fnd_handle port, const fnd_recv *r, int32_t *result) {
    const fnd_msg *m = &r->msg;
    if (r->badge & LOOKUP_BADGE) {
        if (m->op != FND_PIPE_OPEN) {
            *result = FND_E_UNSUPPORTED;
            return 0;
        }
        *result = pipe_open(port, r);
        return *result == FND_OK;
    }
    int is_write;
    struct pipe *p = by_badge(r->badge, &is_write);
    if (!p) {
        *result = FND_E_BAD_HANDLE;
        return 0;
    }
    *result = FND_OK;
    switch (m->op) {
    case FND_ST_READ:
        if (is_write)
            *result = FND_E_ACCESS;
        else if (m->a0 == 0 || m->a0 > FND_IPC_MAX_DATA)
            *result = FND_E_BAD_ARG;
        else if (!queue(r, FND_ST_READ, m->a0 < r->reply_cap ? m->a0 : r->reply_cap))
            *result = FND_E_LIMIT;
        else {
            settle_all();
            return 1;
        }
        return 0;
    case FND_ST_WRITE:
        if (!is_write)
            *result = FND_E_ACCESS;
        else if (m->len > PIPE_SIZE)
            *result = FND_E_BAD_ARG;
        else if (m->len == 0)
            reply(r->token, FND_OK, 0, 0, 0), *result = FND_OK;
        else {
            struct pending *q = queue(r, FND_ST_WRITE, m->len);
            if (!q) {
                *result = FND_E_LIMIT;
            } else {
                for (uint32_t k = 0; k < m->len; ++k)
                    q->wbuf[k] = data[k];
                settle_all();
                return 1;
            }
        }
        return m->len == 0 && *result == FND_OK;
    case FND_ST_STAT: {
        fnd_stat_t st = {FND_TYPE_PIPE};
        st.mode = is_write ? 0x12u : 0x24u; /* owner W / owner R (spec_FS2 §8) */
        for (uint32_t i = 0; i < sizeof st; ++i)
            out[i] = ((const uint8_t *)&st)[i];
        reply(r->token, FND_OK, 0, out, sizeof st);
        return 1;
    }
    case FND_ST_SYNC:
        return 0;
    default:
        *result = FND_E_UNSUPPORTED; /* ST_SEEK, ST_TRUNCATE */
        return 0;
    }
}

/* CLIENT_GONE (§4.3): the last handle of an end closed. */
static void client_gone(uint32_t badge) {
    int is_write;
    struct pipe *p = by_badge(badge, &is_write);
    if (!p)
        return;
    if (is_write)
        p->write_badge = 0;
    else
        p->read_badge = 0;
    for (uint32_t i = 0; i < PENDING; ++i) { /* requests of the end that is gone */
        if (pending[i].badge == badge)
            pending[i].badge = 0;
    }
    settle(p);
    if (!p->read_badge && !p->write_badge)
        p->id = 0;
}

int main(void) {
    fnd_init();
    int32_t port = fnd_sys_port_create();
    if (port < 0 || fnd_sys_svc_register("pipe", port) != FND_OK)
        fnd_exit(FND_E_BAD_HANDLE);
    for (;;) {
        fnd_recv r = {{0}, (uint32_t)(uintptr_t)data, sizeof data, 0, 0, 0, 0, FND_FOREVER, 0};
        int32_t status = fnd_sys_ipc_recv((uint32_t)port, &r);
        if (status == FND_OK && r.token == 0) {
            if (r.msg.op == FND_NOTIFY_CLIENT_GONE)
                client_gone(r.badge);
        } else if (status == FND_OK) {
            int32_t result;
            r.msg.buf = 0; /* the data is in data[] */
            if (!handle((fnd_handle)port, &r, &result))
                reply(r.token, result, 0, 0, 0);
        }
    }
}
