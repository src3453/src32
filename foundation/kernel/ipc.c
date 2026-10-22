/* Handles, IPC and service registration (doc/spec_Foundation.md §6, §8).
 *
 * A caller blocks in SEND until a receiver takes its request, then in REPLY
 * until the server answers its token. Data moves directly between the two
 * processes' memory; the kernel sees all RAM, so it copies with plain loads
 * and stores after checking both ranges (§5.4). Waiting threads are found by
 * scanning the thread table, which is small: the queue of a port is its
 * callers in SEND ordered by seq. */
#include "internal.h"

#define MAX_PORTS 32u
#define MAX_CHANNELS 128u
#define MAX_SERVICES 32u /* §8.6 */
#define SERVICE_NAME_MAX 31u
#define LOOKUP_BADGE 0x80000000u
#define TOKEN_INDEX_BITS 8u

struct fnd_port {
    uint32_t used, refs;
    struct fnd_chan *gone_head, *gone_tail; /* CLIENT_GONE pending (§8.5) */
};

struct fnd_chan {
    uint32_t used, refs;
    struct fnd_port *port; /* NULL once the port is gone */
    uint32_t badge;
    struct fnd_chan *gone_next; /* on its port's pending list when refs = 0 */
};

static struct fnd_port ports[MAX_PORTS];
static struct fnd_chan channels[MAX_CHANNELS];
static struct {
    struct fnd_port *port;
    char name[SERVICE_NAME_MAX + 1u];
} services[MAX_SERVICES];
static uint32_t next_seq, next_lookup_badge;

void kmem_copy(uint32_t dst, uint32_t src, uint32_t len) {
    volatile uint8_t *d = (volatile uint8_t *)(uintptr_t)dst;
    const volatile uint8_t *s = (const volatile uint8_t *)(uintptr_t)src;
    if (((dst | src | len) & 3u) == 0) {
        for (uint32_t i = 0; i < len; i += 4u)
            *(volatile uint32_t *)(uintptr_t)(dst + i) = *(const volatile uint32_t *)(uintptr_t)(src + i);
        return;
    }
    for (uint32_t i = 0; i < len; ++i)
        d[i] = s[i];
}

void kmem_zero(uint32_t start, uint32_t end) {
    for (uint32_t a = start; a < end; a += 4u)
        *(volatile uint32_t *)(uintptr_t)a = 0;
}

/* A user struct of size bytes at addr: 4-byte aligned and inside regions
 * with perm. */
static int32_t user_struct(const struct fnd_proc *proc, uint32_t addr, uint32_t size, uint32_t perm) {
    if (addr & 3u)
        return FND_E_BAD_ARG;
    return proc_user_range_ok(proc, addr, size, perm) ? FND_OK : FND_E_ACCESS;
}

/* ---- objects and handles ---- */

static void obj_ref(uint32_t kind, void *object) {
    if (kind == FND_OBJ_PORT)
        ++((struct fnd_port *)object)->refs;
    else if (kind == FND_OBJ_CHANNEL)
        ++((struct fnd_chan *)object)->refs;
    else
        ++((struct fnd_proc *)object)->refs;
}

static struct fnd_thread *find_receiver(const struct fnd_port *port);
static void deliver_gone(struct fnd_thread *receiver, struct fnd_port *port);

static void port_destroy(struct fnd_port *port) {
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        struct fnd_thread *t = &fnd_threads[i];
        if (t->state == T_BLOCKED && t->port == port &&
            (t->reason == B_SEND || t->reason == B_REPLY || t->reason == B_RECV))
            sched_wake(t, t->reason == B_RECV ? FND_E_BAD_HANDLE : FND_E_PEER_GONE);
    }
    for (unsigned i = 0; i < MAX_CHANNELS; ++i) {
        struct fnd_chan *c = &channels[i];
        if (c->used && c->port == port) {
            c->port = 0;
            if (c->refs == 0)
                c->used = 0; /* a CLIENT_GONE nobody will read */
        }
    }
    for (unsigned i = 0; i < MAX_SERVICES; ++i) {
        if (services[i].port == port)
            services[i].port = 0;
    }
    port->used = 0;
}

static void chan_last_closed(struct fnd_chan *chan) {
    struct fnd_port *port = chan->port;
    if (!port) {
        chan->used = 0;
        return;
    }
    chan->gone_next = 0;
    if (port->gone_tail)
        port->gone_tail->gone_next = chan;
    else
        port->gone_head = chan;
    port->gone_tail = chan;
    struct fnd_thread *receiver = find_receiver(port);
    if (receiver) {
        deliver_gone(receiver, port);
        sched_wake(receiver, FND_OK);
    }
}

static void obj_unref(uint32_t kind, void *object) {
    if (kind == FND_OBJ_PORT) {
        struct fnd_port *port = object;
        if (--port->refs == 0)
            port_destroy(port);
    } else if (kind == FND_OBJ_CHANNEL) {
        struct fnd_chan *chan = object;
        if (--chan->refs == 0)
            chan_last_closed(chan);
    } else {
        proc_release(object);
    }
}

int32_t handle_add(struct fnd_proc *proc, uint32_t kind, void *object) {
    for (uint32_t i = 0; i < FND_MAX_HANDLES; ++i) {
        if (proc->handles[i].kind == FND_OBJ_NONE) {
            proc->handles[i].kind = kind;
            proc->handles[i].object = object;
            obj_ref(kind, object);
            return (int32_t)(i + 1u);
        }
    }
    return FND_E_LIMIT;
}

uint32_t handle_free_count(const struct fnd_proc *proc) {
    uint32_t n = 0;
    for (uint32_t i = 0; i < FND_MAX_HANDLES; ++i)
        n += proc->handles[i].kind == FND_OBJ_NONE;
    return n;
}

static struct fnd_handle_entry *entry(struct fnd_proc *proc, uint32_t h) {
    if (h == 0 || h > FND_MAX_HANDLES || proc->handles[h - 1u].kind == FND_OBJ_NONE)
        return 0;
    return &proc->handles[h - 1u];
}

int32_t sys_handle_close(struct fnd_proc *proc, uint32_t h) {
    struct fnd_handle_entry *e = entry(proc, h);
    if (!e)
        return FND_E_BAD_HANDLE;
    uint32_t kind = e->kind;
    void *object = e->object;
    e->kind = FND_OBJ_NONE;
    e->object = 0;
    obj_unref(kind, object);
    return FND_OK;
}

void handle_close_all(struct fnd_proc *proc) {
    for (uint32_t h = 1; h <= FND_MAX_HANDLES; ++h)
        sys_handle_close(proc, h);
}

int32_t handle_dup_into(struct fnd_proc *from, uint32_t h, struct fnd_proc *to) {
    struct fnd_handle_entry *e = entry(from, h);
    if (!e)
        return FND_E_BAD_HANDLE;
    return handle_add(to, e->kind, e->object);
}

int32_t sys_handle_dup(struct fnd_proc *proc, uint32_t h) { return handle_dup_into(proc, h, proc); }

/* Moves handle h of from into to (§8.4). Returns the new number. */
static int32_t handle_move(struct fnd_proc *from, uint32_t h, struct fnd_proc *to) {
    int32_t moved = handle_dup_into(from, h, to);
    if (moved > 0)
        sys_handle_close(from, h);
    return moved;
}

int32_t sys_port_create(struct fnd_proc *proc) {
    for (unsigned i = 0; i < MAX_PORTS; ++i) {
        struct fnd_port *port = &ports[i];
        if (port->used)
            continue;
        *port = (struct fnd_port){1u, 0, 0, 0};
        int32_t h = handle_add(proc, FND_OBJ_PORT, port);
        if (h < 0)
            port->used = 0;
        return h;
    }
    return FND_E_LIMIT;
}

static int32_t new_channel(struct fnd_proc *proc, struct fnd_port *port, uint32_t badge) {
    for (unsigned i = 0; i < MAX_CHANNELS; ++i) {
        struct fnd_chan *chan = &channels[i];
        if (chan->used)
            continue;
        *chan = (struct fnd_chan){1u, 0, port, badge, 0};
        int32_t h = handle_add(proc, FND_OBJ_CHANNEL, chan);
        if (h < 0)
            chan->used = 0;
        return h;
    }
    return FND_E_LIMIT;
}

int32_t sys_chan_create(struct fnd_proc *proc, uint32_t h, uint32_t badge) {
    struct fnd_handle_entry *e = entry(proc, h);
    if (!e || e->kind != FND_OBJ_PORT)
        return FND_E_BAD_HANDLE;
    if (badge == 0 || badge > 0x7FFFFFFFu)
        return FND_E_BAD_ARG;
    return new_channel(proc, e->object, badge);
}

/* ---- calls ---- */

static struct fnd_thread *find_receiver(const struct fnd_port *port) {
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        struct fnd_thread *t = &fnd_threads[i];
        if (t->state == T_BLOCKED && t->reason == B_RECV && t->port == port)
            return t;
    }
    return 0;
}

static struct fnd_thread *next_caller(const struct fnd_port *port) {
    struct fnd_thread *best = 0;
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        struct fnd_thread *t = &fnd_threads[i];
        if (t->state == T_BLOCKED && t->reason == B_SEND && t->port == port &&
            (!best || (int32_t)(t->seq - best->seq) < 0))
            best = t;
    }
    return best;
}

static uint32_t token_of(const struct fnd_thread *t) {
    return (uint32_t)(t - fnd_threads + 1) | t->gen << TOKEN_INDEX_BITS;
}

/* Writes a received message into the receiver's fnd_recv at rcv. */
static void write_recv(uint32_t rcv, const fnd_msg *msg, uint32_t token, uint32_t badge,
                       uint32_t reply_cap, uint32_t total) {
    fnd_recv *r = (fnd_recv *)(uintptr_t)rcv;
    r->msg = *msg;
    r->msg.buf = r->data;
    r->token = token;
    r->badge = badge;
    r->reply_cap = reply_cap;
    r->total = total;
}

static void deliver_gone(struct fnd_thread *receiver, struct fnd_port *port) {
    struct fnd_chan *chan = port->gone_head;
    port->gone_head = chan->gone_next;
    if (!port->gone_head)
        port->gone_tail = 0;
    fnd_msg msg = {FND_NOTIFY_CLIENT_GONE, 0, 0, 0, 0, 0, 0, 0};
    write_recv(receiver->rcv, &msg, 0, chan->badge, 0, 0);
    chan->used = 0;
}

/* Hands caller's request to receiver, which is in sys_ipc_recv or blocked
 * in RECV. Returns 0, or an error for the caller (its handle could not be
 * transferred), which the caller is then woken with. */
static int32_t deliver_call(struct fnd_thread *receiver, struct fnd_thread *caller) {
    fnd_msg msg = caller->req;
    fnd_recv *r = (fnd_recv *)(uintptr_t)receiver->rcv;
    if (msg.flags & FND_MSG_XFER_HANDLE) {
        int32_t moved = handle_move(caller->proc, msg.a3, receiver->proc);
        if (moved < 0)
            return moved;
        msg.a3 = (uint32_t)moved;
    }
    uint32_t n = msg.len < r->cap ? msg.len : r->cap;
    kmem_copy(r->data, msg.buf, n);
    msg.len = n;
    write_recv(receiver->rcv, &msg, token_of(caller), caller->badge, caller->rep_cap, caller->req.len);
    caller->reason = B_REPLY;
    caller->wait_proc = receiver->proc; /* the process that may answer */
    return FND_OK;
}

/* Gives receiver (blocked or not) the next item of port, if any. */
static int take_next(struct fnd_thread *receiver, struct fnd_port *port) {
    if (port->gone_head) {
        deliver_gone(receiver, port);
        return 1;
    }
    struct fnd_thread *caller;
    while ((caller = next_caller(port)) != 0) {
        int32_t status = deliver_call(receiver, caller);
        if (status == FND_OK)
            return 1;
        sched_wake(caller, status);
    }
    return 0;
}

/* Validates a message and its data for sending. */
static int32_t read_msg(const struct fnd_proc *proc, uint32_t addr, fnd_msg *msg) {
    int32_t status = user_struct(proc, addr, sizeof *msg, CPT32_PERM_R);
    if (status != FND_OK)
        return status;
    *msg = *(const fnd_msg *)(uintptr_t)addr;
    if (msg->flags & ~FND_MSG_XFER_HANDLE)
        return FND_E_BAD_ARG;
    if (msg->len > FND_IPC_MAX_DATA)
        return FND_E_LIMIT;
    if (!proc_user_range_ok(proc, msg->buf, msg->len, CPT32_PERM_R))
        return FND_E_ACCESS;
    return FND_OK;
}

int32_t sys_ipc_call(struct fnd_thread *thread, uint32_t ch, uint32_t req, uint32_t rep) {
    struct fnd_proc *proc = thread->proc;
    struct fnd_handle_entry *e = entry(proc, ch);
    if (!e || e->kind != FND_OBJ_CHANNEL)
        return FND_E_BAD_HANDLE;
    struct fnd_chan *chan = e->object;
    fnd_msg msg;
    int32_t status = read_msg(proc, req, &msg);
    if (status == FND_OK)
        status = user_struct(proc, rep, sizeof(fnd_msg), CPT32_PERM_R | CPT32_PERM_W);
    if (status != FND_OK)
        return status;
    const fnd_msg *r = (const fnd_msg *)(uintptr_t)rep;
    if (!proc_user_range_ok(proc, r->buf, r->len, CPT32_PERM_W))
        return FND_E_ACCESS;
    if ((msg.flags & FND_MSG_XFER_HANDLE) && !entry(proc, msg.a3))
        return FND_E_BAD_HANDLE;
    if (!chan->port)
        return FND_E_PEER_GONE;
    thread->req = msg;
    thread->rep = rep;
    thread->rep_buf = r->buf;
    thread->rep_cap = r->len;
    thread->port = chan->port;
    thread->badge = chan->badge;
    thread->seq = ++next_seq;
    ++thread->gen;
    sched_block(thread, B_SEND, FND_FOREVER);
    struct fnd_thread *receiver = find_receiver(chan->port);
    if (receiver && take_next(receiver, chan->port))
        sched_wake(receiver, FND_OK);
    return FND_OK; /* the result comes with the reply */
}

int32_t sys_ipc_recv(struct fnd_thread *thread, uint32_t h, uint32_t rcv) {
    struct fnd_proc *proc = thread->proc;
    struct fnd_handle_entry *e = entry(proc, h);
    if (!e || e->kind != FND_OBJ_PORT)
        return FND_E_BAD_HANDLE;
    int32_t status = user_struct(proc, rcv, sizeof(fnd_recv), CPT32_PERM_R | CPT32_PERM_W);
    if (status != FND_OK)
        return status;
    const fnd_recv *r = (const fnd_recv *)(uintptr_t)rcv;
    if (r->reserved)
        return FND_E_BAD_ARG;
    if (!proc_user_range_ok(proc, r->data, r->cap, CPT32_PERM_W))
        return FND_E_ACCESS;
    thread->rcv = rcv;
    if (take_next(thread, e->object))
        return FND_OK;
    if (r->timeout == 0)
        return FND_E_WOULD_BLOCK;
    thread->port = e->object;
    sched_block(thread, B_RECV, r->timeout);
    return FND_OK;
}

static struct fnd_thread *token_caller(const struct fnd_proc *proc, uint32_t token) {
    uint32_t index = token & ((1u << TOKEN_INDEX_BITS) - 1u);
    if (index == 0 || index > FND_MAX_THREADS)
        return 0;
    struct fnd_thread *t = &fnd_threads[index - 1u];
    if (t->state != T_BLOCKED || t->reason != B_REPLY || t->wait_proc != proc ||
        (t->gen & (0xFFFFFFFFu >> TOKEN_INDEX_BITS)) != token >> TOKEN_INDEX_BITS)
        return 0;
    return t;
}

int32_t sys_ipc_reply(struct fnd_thread *thread, uint32_t token, uint32_t addr) {
    struct fnd_proc *proc = thread->proc;
    fnd_msg msg;
    int32_t status = read_msg(proc, addr, &msg);
    if (status != FND_OK)
        return status;
    if ((msg.flags & FND_MSG_XFER_HANDLE) && !entry(proc, msg.a3))
        return FND_E_BAD_HANDLE;
    struct fnd_thread *caller = token_caller(proc, token);
    if (!caller)
        return FND_E_PEER_GONE;
    if (msg.len > caller->rep_cap)
        return FND_E_LIMIT; /* the token stays valid (§8.2) */
    if (msg.flags & FND_MSG_XFER_HANDLE) {
        int32_t moved = handle_move(proc, msg.a3, caller->proc);
        if (moved < 0)
            return moved;
        msg.a3 = (uint32_t)moved;
    }
    kmem_copy(caller->rep_buf, msg.buf, msg.len);
    msg.buf = caller->rep_buf;
    *(fnd_msg *)(uintptr_t)caller->rep = msg;
    sched_wake(caller, FND_OK);
    return FND_OK;
}

int32_t sys_ipc_reply_recv(struct fnd_thread *thread, uint32_t token, uint32_t msg, uint32_t port,
                           uint32_t rcv) {
    int32_t status = sys_ipc_reply(thread, token, msg);
    if (status < 0 && status != FND_E_PEER_GONE)
        return status;
    return sys_ipc_recv(thread, port, rcv);
}

void ipc_timed_out(struct fnd_thread *thread) {
    sched_wake(thread, thread->reason == B_SLEEP ? FND_OK : FND_E_TIMEOUT);
}

/* ---- services (§8.6) ---- */

/* Copies a NUL-terminated service name from user memory and checks
 * [a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*. */
static int32_t read_name(const struct fnd_proc *proc, uint32_t addr, char name[SERVICE_NAME_MAX + 1u]) {
    for (uint32_t i = 0;; ++i) {
        if (i > SERVICE_NAME_MAX)
            return FND_E_BAD_ARG;
        if (!proc_user_range_ok(proc, addr + i, 1u, CPT32_PERM_R))
            return FND_E_ACCESS;
        char c = *(const volatile char *)(uintptr_t)(addr + i);
        name[i] = c;
        if (c == 0)
            break;
    }
    int start = 1;
    for (uint32_t i = 0; name[i]; ++i) {
        char c = name[i];
        int lower = c >= 'a' && c <= 'z';
        if (start ? !lower : !(lower || (c >= '0' && c <= '9') || c == '_' || c == '.'))
            return FND_E_BAD_ARG;
        if (c == '.' && name[i + 1u] == 0)
            return FND_E_BAD_ARG;
        start = c == '.';
    }
    return name[0] ? FND_OK : FND_E_BAD_ARG;
}

static int name_equal(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return *a == *b;
}

int32_t sys_svc_register(struct fnd_proc *proc, uint32_t name_addr, uint32_t h) {
    char name[SERVICE_NAME_MAX + 1u];
    if (!(proc->caps & FND_CAP_SVC))
        return FND_E_ACCESS;
    int32_t status = read_name(proc, name_addr, name);
    if (status != FND_OK)
        return status;
    struct fnd_handle_entry *e = entry(proc, h);
    if (!e || e->kind != FND_OBJ_PORT)
        return FND_E_BAD_HANDLE;
    int free_slot = -1;
    for (unsigned i = 0; i < MAX_SERVICES; ++i) {
        if (!services[i].port) {
            if (free_slot < 0)
                free_slot = (int)i;
        } else if (name_equal(services[i].name, name)) {
            return FND_E_EXISTS;
        }
    }
    if (free_slot < 0)
        return FND_E_LIMIT;
    for (unsigned i = 0; i <= SERVICE_NAME_MAX; ++i)
        services[free_slot].name[i] = name[i];
    services[free_slot].port = e->object;
    return FND_OK;
}

int32_t sys_svc_lookup(struct fnd_proc *proc, uint32_t name_addr) {
    char name[SERVICE_NAME_MAX + 1u];
    int32_t status = read_name(proc, name_addr, name);
    if (status != FND_OK)
        return status;
    for (unsigned i = 0; i < MAX_SERVICES; ++i) {
        if (services[i].port && name_equal(services[i].name, name)) {
            next_lookup_badge = (next_lookup_badge + 1u) & 0x7FFFFFFFu;
            return new_channel(proc, services[i].port, LOOKUP_BADGE | next_lookup_badge);
        }
    }
    return FND_E_NOT_FOUND;
}

/* ---- processes ---- */

static struct fnd_proc *process_handle(struct fnd_proc *proc, uint32_t h) {
    struct fnd_handle_entry *e = entry(proc, h);
    return e && e->kind == FND_OBJ_PROCESS ? e->object : 0;
}

int32_t sys_wait(struct fnd_thread *thread, uint32_t h, uint32_t timeout, uint32_t code_ptr) {
    struct fnd_proc *target = process_handle(thread->proc, h);
    if (!target)
        return FND_E_BAD_HANDLE; /* THREAD handles come with thread_create */
    if (code_ptr) {
        int32_t status = user_struct(thread->proc, code_ptr, 4u, CPT32_PERM_W);
        if (status != FND_OK)
            return status;
    }
    if (target->state != FND_PROC_LIVE) {
        if (code_ptr)
            *(volatile int32_t *)(uintptr_t)code_ptr = target->exit_code;
        return FND_OK;
    }
    if (timeout == 0)
        return FND_E_WOULD_BLOCK;
    thread->wait_proc = target;
    thread->code_ptr = code_ptr;
    sched_block(thread, B_WAIT, timeout);
    return FND_OK;
}

void ipc_proc_exited(struct fnd_proc *proc) {
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        struct fnd_thread *t = &fnd_threads[i];
        if (t->state == T_BLOCKED && t->reason == B_WAIT && t->wait_proc == proc) {
            if (t->code_ptr)
                *(volatile int32_t *)(uintptr_t)t->code_ptr = proc->exit_code;
            sched_wake(t, FND_OK);
        }
    }
}

int32_t sys_proc_kill(struct fnd_proc *proc, uint32_t h) {
    struct fnd_proc *target = process_handle(proc, h);
    if (!target)
        return FND_E_BAD_HANDLE;
    proc_exit(target, FND_E_KILLED, 0);
    return FND_OK;
}
