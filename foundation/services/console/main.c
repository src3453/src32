/* The console service (doc/spec_Foundation_API.md §5, §8, §9): the PCG
 * screen and the PeC keyboard behind the stream and console protocols.
 *
 * It registers as "console" and "dev.con". A channel from svc_lookup only
 * takes DEV_OPEN, which makes a console stream: a channel of the service's
 * port whose badge names a stream object. Reads and CON_READ_KEY wait in one
 * FIFO (§8.3) and are answered when keys arrive; CON_READ_KEY with
 * FND_CON_NOWAIT answers at once. The keyboard still uses polling whenever
 * ipc_recv times out. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

#include "con_pcg.h"

#define POLL_TICKS 1u /* minimum wait: 10 ms at 100 Hz */
#define STREAMS 16u
#define PENDING 16u
#define KEYS 256u /* type-ahead; as deep as the PeC event FIFO */
#define LINE_MAX 1023u
#define LOOKUP_BADGE 0x80000000u

struct stream {
    uint32_t badge; /* 0: free */
    uint32_t flags; /* FND_OPEN_READ | FND_OPEN_WRITE */
};

/* §8.2: the mode is shared by all handles of a stream; the editor and the
 * key queue belong to the one screen. */
static uint32_t mode;
static struct stream streams[STREAMS];
static uint32_t next_badge;

struct pending {
    uint32_t token, badge;
    uint32_t op;  /* FND_ST_READ or FND_CON_READ_KEY */
    uint32_t max; /* ST_READ: bytes asked for */
};
static struct pending pending[PENDING];
static uint32_t pending_count;
static uint32_t break_token, break_badge; /* CON_BREAK_WAIT (§8.4) */

static fnd_key_t keys[KEYS];
static uint32_t key_head, key_count;

/* LINE mode: the line being edited, then the finished line still to be
 * read (§8.2: a long line comes back over several ST_READs). */
static char edit[LINE_MAX + 1u];
static uint32_t edit_len;
static char ready[LINE_MAX + 1u];
static uint32_t ready_len, ready_pos;
static int ready_eof;

static uint8_t data[FND_IPC_MAX_DATA];
static uint8_t out[FND_IPC_MAX_DATA];

static void reply(uint32_t token, int32_t result, uint32_t a0, uint32_t a1, uint32_t a2, const void *buf,
                  uint32_t len) {
    fnd_msg msg = {(uint32_t)result, 0, a0, a1, a2, 0, (uint32_t)(uintptr_t)buf, len};
    fnd_sys_ipc_reply(token, &msg); /* a client that is gone needs no answer */
}

static struct stream *find_stream(uint32_t badge) {
    for (uint32_t i = 0; i < STREAMS; ++i) {
        if (streams[i].badge && streams[i].badge == badge)
            return &streams[i];
    }
    return 0;
}

static void write_text(const void *text, uint32_t len) { con_pcg_write((const uint8_t *)text, len); }

/* ---- input ---- */

static uint32_t encode(uint32_t cp, char *buf) {
    if (cp < 0x80u) {
        buf[0] = (char)cp;
        return 1;
    }
    if (cp < 0x800u) {
        buf[0] = (char)(0xC0u | cp >> 6);
        buf[1] = (char)(0x80u | (cp & 0x3Fu));
        return 2;
    }
    if (cp < 0x10000u) {
        buf[0] = (char)(0xE0u | cp >> 12);
        buf[1] = (char)(0x80u | ((cp >> 6) & 0x3Fu));
        buf[2] = (char)(0x80u | (cp & 0x3Fu));
        return 3;
    }
    buf[0] = (char)(0xF0u | cp >> 18);
    buf[1] = (char)(0x80u | ((cp >> 12) & 0x3Fu));
    buf[2] = (char)(0x80u | ((cp >> 6) & 0x3Fu));
    buf[3] = (char)(0x80u | (cp & 0x3Fu));
    return 4;
}

static void drop_pending(uint32_t i) {
    for (; i + 1u < pending_count; ++i)
        pending[i] = pending[i + 1u];
    --pending_count;
}

/* Answers the oldest ST_READ from the finished line. */
static int serve_ready(void) {
    if (!pending_count || pending[0].op != FND_ST_READ || !(ready_len || ready_eof))
        return 0;
    struct pending *p = &pending[0];
    uint32_t n = ready_len - ready_pos;
    if (n > p->max)
        n = p->max;
    reply(p->token, FND_OK, 0, 0, 0, ready + ready_pos, n);
    ready_pos += n;
    if (ready_pos == ready_len) {
        ready_len = ready_pos = 0;
        ready_eof = 0;
    }
    drop_pending(0);
    return 1;
}

/* LINE mode editing (§8.2): characters at the end and Backspace, echoed;
 * Enter finishes the line with LF; Ctrl+D on an empty line is EOF. */
static void line_key(const fnd_key_t *k) {
    if (k->code == FND_KEY_ENTER) {
        edit[edit_len++] = '\n';
        for (uint32_t i = 0; i < edit_len; ++i)
            ready[i] = edit[i];
        ready_len = edit_len;
        ready_pos = 0;
        edit_len = 0;
        write_text("\n", 1);
    } else if (k->code == FND_KEY_BACKSPACE) {
        if (edit_len) {
            do
                --edit_len;
            while (edit_len && ((uint8_t)edit[edit_len] & 0xC0u) == 0x80u);
            write_text("\b \b", 3);
        }
    } else if (k->code == FND_KEY_CHAR && (k->mods & FND_MOD_CTRL)) {
        if (k->ch == 'd' && edit_len == 0)
            ready_eof = 1;
        else if (k->ch == 'c')
            edit_len = 0; /* §8.4 without a break waiter: drop the line */
    } else if (k->code == FND_KEY_CHAR) {
        char buf[4];
        uint32_t n = encode(k->ch, buf);
        if (edit_len + n < LINE_MAX) {
            for (uint32_t i = 0; i < n; ++i)
                edit[edit_len++] = buf[i];
            write_text(buf, n);
        }
    }
}

/* Gives queued keys to the waiting requests, oldest first. */
static void dispatch(void) {
    for (;;) {
        if (serve_ready())
            continue;
        if (!pending_count || !key_count)
            return;
        fnd_key_t k = keys[key_head];
        key_head = (key_head + 1u) % KEYS;
        --key_count;
        struct pending *p = &pending[0];
        if (p->op == FND_CON_READ_KEY) {
            reply(p->token, FND_OK, k.code, k.ch, k.mods, 0, 0);
            drop_pending(0);
        } else if (mode == FND_CON_RAW) {
            char buf[4];
            if (k.code == FND_KEY_CHAR && !(k.mods & FND_MOD_CTRL)) {
                reply(p->token, FND_OK, 0, 0, 0, buf, encode(k.ch, buf));
                drop_pending(0);
            }
        } else {
            line_key(&k);
        }
    }
}

/* §8.4: with a break waiter, Ctrl+C answers it, fails every pending read
 * and drops the line; it never reaches the queue. */
static void take_break(void) {
    reply(break_token, FND_OK, 0, 0, 0, 0, 0);
    break_token = 0;
    for (uint32_t i = 0; i < pending_count; ++i)
        reply(pending[i].token, FND_E_INTERRUPTED, 0, 0, 0, 0, 0);
    pending_count = 0;
    edit_len = 0;
    key_count = 0;
}

static void poll_keys(void) {
    fnd_key_t k;
    while (con_pcg_poll_key(&k)) {
        if (break_token && k.code == FND_KEY_CHAR && k.ch == 'c' && (k.mods & FND_MOD_CTRL)) {
            take_break();
            continue;
        }
        if (key_count < KEYS) {
            keys[(key_head + key_count) % KEYS] = k;
            ++key_count;
        }
    }
    dispatch();
}

/* ---- requests ---- */

static int32_t dev_open(fnd_handle port, uint32_t token, uint32_t flags) {
    if (!flags || (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE)))
        return FND_E_BAD_ARG;
    for (uint32_t i = 0; i < STREAMS; ++i) {
        if (streams[i].badge)
            continue;
        /* §4.4: a badge is not reused right after it is freed. */
        next_badge = next_badge >= 0x7FFFFFFFu ? 1u : next_badge + 1u;
        int32_t ch = fnd_sys_chan_create(port, next_badge);
        if (ch < 0)
            return ch;
        streams[i].badge = next_badge;
        streams[i].flags = flags;
        fnd_msg msg = {FND_OK, FND_MSG_XFER_HANDLE, 0, 0, 0, (uint32_t)ch, 0, 0};
        if (fnd_sys_ipc_reply(token, &msg) != FND_OK) {
            fnd_sys_handle_close((uint32_t)ch);
            streams[i].badge = 0;
        }
        return FND_OK; /* answered */
    }
    return FND_E_LIMIT;
}

static int32_t queue(const fnd_recv *r, uint32_t op, uint32_t max) {
    if (pending_count == PENDING)
        return FND_E_LIMIT;
    pending[pending_count++] = (struct pending){r->token, r->badge, op, max};
    dispatch();
    return FND_OK; /* answered later */
}

/* Handles one request; returns 1 if it was answered (or will be later),
 * else 0 with *result set for an immediate reply. */
static int handle(fnd_handle port, const fnd_recv *r, int32_t *result, uint32_t a[3]) {
    const fnd_msg *m = &r->msg;
    if (r->badge & LOOKUP_BADGE) {
        if (m->op != FND_DEV_OPEN) {
            *result = FND_E_UNSUPPORTED;
            return 0;
        }
        *result = dev_open(port, r->token, m->a0);
        return *result == FND_OK;
    }
    struct stream *s = find_stream(r->badge);
    if (!s) {
        *result = FND_E_BAD_HANDLE;
        return 0;
    }
    *result = FND_OK;
    switch (m->op) {
    case FND_CON_KEY_STATE: {
        if (!(s->flags & FND_OPEN_READ)) { *result = FND_E_ACCESS; return 0; }
        if (r->reply_cap < sizeof(fnd_keyboard)) { *result = FND_E_LIMIT; return 0; }
        poll_keys();
        fnd_keyboard state;
        con_pcg_key_state(&state);
        reply(r->token, FND_OK, 0, 0, 0, &state, sizeof state);
        return 1;
    }
    case FND_ST_READ:
        if (!(s->flags & FND_OPEN_READ))
            *result = FND_E_ACCESS;
        else if (m->a0 == 0 || m->a0 > FND_IPC_MAX_DATA)
            *result = FND_E_BAD_ARG;
        else
            return (*result = queue(r, FND_ST_READ, m->a0 < r->reply_cap ? m->a0 : r->reply_cap)) == FND_OK;
        return 0;
    case FND_CON_READ_KEY:
        if (!(m->a0 & FND_CON_NOWAIT))
            return (*result = queue(r, FND_CON_READ_KEY, 0)) == FND_OK;
        /* A queued key now, if no earlier request waits for it; else END. */
        poll_keys();
        if (pending_count || !key_count) {
            *result = FND_END;
        } else {
            a[0] = keys[key_head].code;
            a[1] = keys[key_head].ch;
            a[2] = keys[key_head].mods;
            key_head = (key_head + 1u) % KEYS;
            --key_count;
        }
        return 0;
    case FND_ST_WRITE:
        if (!(s->flags & FND_OPEN_WRITE)) {
            *result = FND_E_ACCESS;
        } else {
            write_text(data, m->len);
            a[0] = m->len;
        }
        return 0;
    case FND_ST_STAT: {
        fnd_stat_t st = {FND_TYPE_CONSOLE};
        for (uint32_t i = 0; i < sizeof st; ++i)
            out[i] = ((const uint8_t *)&st)[i];
        reply(r->token, FND_OK, 0, 0, 0, out, sizeof st);
        return 1;
    }
    case FND_ST_SYNC:
        return 0;
    case FND_CON_SET_MODE:
        if (m->a0 > FND_CON_RAW)
            *result = FND_E_BAD_ARG;
        else
            mode = m->a0;
        return 0;
    case FND_CON_BREAK_WAIT:
        if (break_token)
            *result = FND_E_BUSY;
        else
            break_token = r->token, break_badge = r->badge;
        return break_token == r->token;
    case FND_CON_CLEAR:
        con_pcg_clear();
        return 0;
    case FND_CON_GET_SIZE:
        a[0] = con_pcg_columns();
        a[1] = CON_PCG_ROWS;
        return 0;
    case FND_CON_SET_CURSOR:
        if (m->a0 >= con_pcg_columns() || m->a1 >= CON_PCG_ROWS)
            *result = FND_E_BAD_ARG;
        else
            con_pcg_set_cursor(m->a0, m->a1);
        return 0;
    case FND_CON_GET_CURSOR:
        con_pcg_get_cursor(&a[0], &a[1]);
        return 0;
    case FND_CON_SET_COLOR:
        if (m->a0 > 15 || m->a1 > 15)
            *result = FND_E_BAD_ARG;
        else
            con_pcg_set_color(m->a0, m->a1);
        return 0;
    case FND_CON_SET_DISPLAY:
        *result = con_pcg_set_display(m->a0);
        return 0;
    case FND_CON_GET_DISPLAY:
        a[0] = con_pcg_display();
        return 0;
    default:
        *result = FND_E_UNSUPPORTED; /* ST_SEEK, ST_TRUNCATE */
        return 0;
    }
}

/* CLIENT_GONE (§4.3): the last handle of a stream closed. */
static void client_gone(uint32_t badge) {
    struct stream *s = find_stream(badge);
    if (s)
        s->badge = 0;
    if (break_token && break_badge == badge)
        break_token = 0;
}

int main(void) {
    fnd_init();
    con_pcg_init();
    int32_t port = fnd_sys_port_create();
    if (port < 0 || fnd_sys_svc_register("console", port) != FND_OK ||
        fnd_sys_svc_register("dev.con", port) != FND_OK)
        fnd_exit(FND_E_BAD_HANDLE);
    fnd_recv r;
    for (;;) {
        r = (fnd_recv){{0}, (uint32_t)(uintptr_t)data, sizeof data, 0, 0, 0, 0, POLL_TICKS, 0};
        int32_t status = fnd_sys_ipc_recv((uint32_t)port, &r);
        if (status == FND_OK && r.token == 0) {
            if (r.msg.op == FND_NOTIFY_CLIENT_GONE)
                client_gone(r.badge);
        } else if (status == FND_OK) {
            int32_t result;
            uint32_t a[3] = {0, 0, 0};
            r.msg.buf = 0; /* the data is in data[] */
            if (!handle((fnd_handle)port, &r, &result, a))
                reply(r.token, result, a[0], a[1], a[2], 0, 0);
        }
        poll_keys();
    }
}
