/* libl3p, the WAL "l3p" 1.0 (foundation/include/foundation/l3p.h): the
 * opening side of L3P (doc/spec_NET_L3P.md) over libcnet. */
#include <foundation/l3p.h>
#include <foundation/sys.h>

#define PROTO_L3P 0x04u
#define HDR 20u
#define T_OPEN 0x01u
#define T_ACCEPT 0x02u
#define T_RESET 0x03u
#define T_CLOSE 0x04u
#define T_CLOSED 0x05u
#define T_ACKONLY 0x06u
#define T_REQ 0x10u
#define T_RSP 0x11u
#define T_UREQ 0x18u
#define T_URSP 0x19u
#define T_DATA 0x20u
#define F_ENC 0x01u
#define F_FIN 0x02u

/* §4.3 in ticks of 10 ms */
#define RTO_FIRST 30u
#define RTO_MAX 240u
#define TXN_GIVEUP 1500u
#define STREAM_GIVEUP 6000u
#define OPEN_GIVEUP 1500u
#define CLOSE_WAIT 200u
#define FIN_WAIT 500u
#define READY_TICKS 300u
#define INFLIGHT 4u

#define S_FREE 0u
#define S_SENT 1u
#define S_DONE 2u

static uint32_t now(void) { return (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO); }

static void put32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24);
    p[1] = (uint8_t)(v >> 16);
    p[2] = (uint8_t)(v >> 8);
    p[3] = (uint8_t)v;
}

static uint32_t get32(const uint8_t *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}

static uint32_t get16(const uint8_t *p) { return (uint32_t)p[0] << 8 | p[1]; }

/* Byte copy with a word fast path (llvm-src32 byte loops are slow). */
static void copy(void *dst, const void *src, uint32_t n) {
    uint8_t *d = dst;
    const uint8_t *s = src;
    if ((((uint32_t)d | (uint32_t)s) & 3u) == 0) {
        uint32_t *dw = (uint32_t *)d;
        const uint32_t *sw = (const uint32_t *)s;
        while (n >= 4u) {
            *dw++ = *sw++;
            n -= 4u;
        }
        d = (uint8_t *)dw;
        s = (const uint8_t *)sw;
    }
    while (n--) *d++ = *s++;
}

static uint8_t *pkt(l3p_assoc *a) { return (uint8_t *)a->packet; }
static uint8_t *req_buf(l3p_assoc *a, uint32_t i) { return (uint8_t *)a->mem + i * L3P_MAX_PLAIN; }
static uint8_t *rsp_buf(l3p_assoc *a, uint32_t i) { return (uint8_t *)a->mem + (4u + i) * L3P_MAX_PLAIN; }
static uint8_t *sbuf(l3p_assoc *a) { return (uint8_t *)a->mem; }
static uint8_t *rbuf(l3p_assoc *a) { return (uint8_t *)a->mem + L3P_STREAM_BUF; }

static void header(uint8_t *p, uint32_t kind, uint32_t type, uint32_t flags, uint32_t conn, uint32_t seq,
                   uint32_t ack, uint32_t window) {
    p[0] = (uint8_t)(0x10u | kind);
    p[1] = (uint8_t)type;
    p[2] = (uint8_t)flags;
    p[3] = 0;
    put32(p + 4, conn);
    put32(p + 8, seq);
    put32(p + 12, ack);
    p[16] = (uint8_t)(window >> 8);
    p[17] = (uint8_t)window;
    p[18] = p[19] = 0;
}

/* Sends one packet; data may already be in place at pkt + HDR (pass 0). */
static void send(l3p_assoc *a, uint32_t type, uint32_t flags, uint32_t conn, uint32_t seq, uint32_t ack,
                 uint32_t window, const void *data, uint32_t len) {
    uint8_t *p = pkt(a);
    header(p, a->kind, type, flags, conn, seq, ack, window);
    if (data) copy(p + HDR, data, len);
    cnet_sendto(a->handle, a->peer, a->peer_socket, PROTO_L3P, p, HDR + len);
    a->last_tx = now();
}

static uint32_t rcv_window(l3p_assoc *a) { return L3P_STREAM_BUF - a->rlen; }

static uint32_t min_unacked(l3p_assoc *a) {
    uint32_t best = a->next_xid;
    for (uint32_t i = 0; i < 4u; ++i)
        if (a->slot_state[i] == S_SENT && a->slot_xid[i] < best) best = a->slot_xid[i];
    return best;
}

static void fail(l3p_assoc *a, int32_t error, int send_reset) {
    if (a->state == L3P_BROKEN || a->state == L3P_CLOSED) return;
    if (send_reset && a->conn) {
        uint8_t r[4] = {0, (uint8_t)L3P_R_TIMEOUT, 0, 0};
        send(a, T_RESET, 0, a->conn, 0, 0, 0, r, 4);
    }
    a->state = L3P_BROKEN;
    a->error = error;
}

static int32_t status(l3p_assoc *a) {
    if (a->state == L3P_BROKEN || a->state == L3P_CLOSED) {
        int32_t e = a->error;
        if (e == FND_E_TIMEOUT) a->error = FND_E_STALE; /* reported once */
        return e;
    }
    return FND_OK;
}

static int from_peer(l3p_assoc *a, const cnet_peer *from) {
    uint32_t want = FND_CNET_NET(a->peer), got = FND_CNET_NET(from->addr);
    return FND_CNET_NODE(from->addr) == FND_CNET_NODE(a->peer) && from->socket == a->peer_socket &&
           from->proto == PROTO_L3P && (!want || !got || want == got);
}

/* ---- streams ---- */

static void stream_pump(l3p_assoc *a) {
    if (a->state != L3P_OPEN) return;
    uint32_t end = a->snd_una + a->slen;
    while (a->snd_nxt != end && a->snd_nxt - a->snd_una < a->peer_window && a->ninflight < INFLIGHT) {
        uint32_t off = a->snd_nxt - a->snd_una, n = end - a->snd_nxt;
        if (n > L3P_MAX_PLAIN) n = L3P_MAX_PLAIN;
        if (n > a->peer_window - off) n = a->peer_window - off;
        if (a->snd_nxt == a->snd_una) {
            a->progress = now();
            a->rto_next = a->progress + a->rto;
        }
        uint8_t *p = pkt(a) + HDR;
        for (uint32_t i = 0; i < n; ++i) p[i] = sbuf(a)[(a->snd_una + off + i) % L3P_STREAM_BUF];
        send(a, T_DATA, 0, a->conn, a->snd_nxt, a->rcv_nxt, rcv_window(a), 0, n);
        a->snd_nxt += n;
        a->inflight[a->ninflight++] = a->snd_nxt;
        a->need_ack = 0;
    }
    if (a->fin_queued && !a->fin_sent && a->snd_nxt == end && a->ninflight < INFLIGHT) {
        if (a->snd_nxt == a->snd_una) {
            a->progress = now();
            a->rto_next = a->progress + a->rto;
        }
        send(a, T_DATA, F_FIN, a->conn, end, a->rcv_nxt, rcv_window(a), 0, 0);
        a->fin_sent = 1;
        a->snd_nxt = end + 1u;
        a->inflight[a->ninflight++] = a->snd_nxt;
        a->need_ack = 0;
    }
}

static void stream_packet(l3p_assoc *a, uint32_t type, uint32_t flags, uint32_t seq, uint32_t ack, uint32_t window,
                          const uint8_t *data, uint32_t len) {
    uint32_t delta = ack - a->snd_una;
    if (delta && delta <= a->snd_nxt - a->snd_una) {
        uint32_t taken = delta < a->slen ? delta : a->slen;
        a->slen -= taken;
        a->snd_una = ack;
        uint32_t k = 0;
        for (uint32_t i = 0; i < a->ninflight; ++i)
            if ((int32_t)(a->inflight[i] - a->snd_una) > 0) a->inflight[k++] = a->inflight[i];
        a->ninflight = k;
        if (a->fin_sent && a->snd_una == a->snd_nxt) a->fin_acked = 1;
        a->rto = RTO_FIRST;
        a->progress = now();
        a->rto_next = a->progress + a->rto;
    }
    a->peer_window = window;
    if (type != T_DATA) return;
    if (seq == a->rcv_nxt && !a->rcv_fin && len <= rcv_window(a)) {
        uint32_t tail = (a->rhead + a->rlen) % L3P_STREAM_BUF;
        for (uint32_t i = 0; i < len; ++i) rbuf(a)[(tail + i) % L3P_STREAM_BUF] = data[i];
        a->rlen += len;
        a->rcv_nxt += len;
        if (flags & F_FIN) {
            a->rcv_nxt += 1u;
            a->rcv_fin = 1;
        }
    }
    a->need_ack = 1;
}

static void stream_tick(l3p_assoc *a, uint32_t t) {
    int pending = a->snd_nxt != a->snd_una || a->slen || (a->fin_queued && !a->fin_sent);
    if (!pending) {
        a->progress = t;
        return;
    }
    if (a->snd_nxt == a->snd_una) {
        if (a->peer_window == 0) { /* zero window probe, not counted for give-up */
            if ((int32_t)(t - a->rto_next) >= 0) {
                a->rto = a->rto * 2u > RTO_MAX ? RTO_MAX : a->rto * 2u;
                a->rto_next = t + a->rto;
                send(a, T_DATA, 0, a->conn, a->snd_nxt, a->rcv_nxt, rcv_window(a), 0, 0);
            }
            a->progress = t;
        }
        return;
    }
    if (t - a->progress > STREAM_GIVEUP) {
        fail(a, FND_E_TIMEOUT, 1);
        return;
    }
    if ((int32_t)(t - a->rto_next) >= 0) {
        a->rto = a->rto * 2u > RTO_MAX ? RTO_MAX : a->rto * 2u;
        a->rto_next = t + a->rto;
        a->snd_nxt = a->snd_una; /* go-back-N */
        a->ninflight = 0;
        if (a->fin_sent && !a->fin_acked) a->fin_sent = 0;
    }
}

/* ---- transactions ---- */

static void txn_packet(l3p_assoc *a, uint32_t type, uint32_t seq, const uint8_t *data, uint32_t len) {
    if (type != T_RSP) return;
    for (uint32_t i = 0; i < 4u; ++i) {
        if (a->slot_state[i] == S_SENT && a->slot_xid[i] == seq) {
            if (len > L3P_MAX_PLAIN) return;
            copy(rsp_buf(a, i), data, len);
            a->slot_len[i] = len;
            a->slot_state[i] = S_DONE;
            return;
        }
    }
}

static void txn_tick(l3p_assoc *a, uint32_t t) {
    for (uint32_t xid = min_unacked(a); xid < a->next_xid; ++xid) {
        uint32_t i = xid % 4u;
        if (a->slot_state[i] != S_SENT || a->slot_xid[i] != xid) continue;
        if (t - a->slot_first[i] > TXN_GIVEUP) {
            fail(a, FND_E_TIMEOUT, 1);
            return;
        }
        if ((int32_t)(t - a->slot_next[i]) >= 0) {
            a->slot_interval[i] = a->slot_interval[i] * 2u > RTO_MAX ? RTO_MAX : a->slot_interval[i] * 2u;
            a->slot_next[i] = t + a->slot_interval[i];
            send(a, T_REQ, 0, a->conn, xid, min_unacked(a), 0, req_buf(a, i), a->slot_len[i]);
        }
    }
}

/* ---- receiving and timers ---- */

static void receive(l3p_assoc *a) {
    for (;;) {
        cnet_peer from;
        uint8_t *buf = pkt(a);
        int32_t n = cnet_recvfrom(a->handle, buf, FND_CNET_MTU, FND_CNET_NONBLOCK, &from);
        if (n < 0) {
            if (n != FND_E_WOULD_BLOCK && n != FND_E_LIMIT) fail(a, n, 0);
            return;
        }
        if (!from_peer(a, &from) || (uint32_t)n < HDR) continue;
        uint32_t kind = buf[0] & 15u, type = buf[1], flags = buf[2];
        uint32_t conn = get32(buf + 4), seq = get32(buf + 8), ack = get32(buf + 12), window = get16(buf + 16);
        uint32_t len = (uint32_t)n - HDR;
        uint8_t *data = buf + HDR;
        if (buf[0] >> 4 != 1u || kind != a->kind || buf[3] || buf[18] || buf[19] || (flags & F_ENC)) continue;
        if (a->state == L3P_OPENING) {
            if (type == T_ACCEPT && conn && len >= 12u && get32(data) == a->nonce) {
                if (get32(data + 4)) { /* options we did not ask for */
                    a->conn = conn;
                    fail(a, FND_E_UNSUPPORTED, 1);
                    return;
                }
                a->conn = conn;
                a->idle = get16(data + 8) ? get16(data + 8) * 100u : 12000u;
                a->state = L3P_OPEN;
                a->ctrl_type = 0;
                a->last_rx = now();
                if (a->kind == L3P_KIND_TXN) {
                    if (window && window < a->window) a->window = window;
                } else {
                    a->peer_window = window;
                }
            } else if (type == T_RESET && !conn && seq == a->nonce) {
                a->reset_reason = len >= 2u ? get16(data) : 0;
                uint32_t m = len > 4u ? len - 4u : 0;
                if (m > sizeof a->reset_message - 1u) m = sizeof a->reset_message - 1u;
                copy(a->reset_message, data + 4, m);
                a->reset_message[m] = 0;
                a->state = L3P_BROKEN;
                a->error = FND_E_IO;
                return;
            }
            continue;
        }
        if (conn != a->conn || a->state == L3P_BROKEN || a->state == L3P_CLOSED) continue;
        a->last_rx = now();
        if (type == T_RESET) {
            if (a->state == L3P_CLOSING) {
                a->state = L3P_CLOSED;
                a->error = FND_E_PEER_GONE;
                return;
            }
            a->reset_reason = len >= 2u ? get16(data) : 0;
            uint32_t m = len > 4u ? len - 4u : 0;
            if (m > sizeof a->reset_message - 1u) m = sizeof a->reset_message - 1u;
            copy(a->reset_message, data + 4, m);
            a->reset_message[m] = 0;
            a->state = L3P_BROKEN;
            a->error = FND_E_IO;
            return;
        }
        if (type == T_CLOSE) {
            send(a, T_CLOSED, 0, a->conn, 0, 0, 0, 0, 0);
            a->state = L3P_CLOSED;
            a->error = FND_E_PEER_GONE;
            return;
        }
        if (type == T_CLOSED) {
            if (a->state == L3P_CLOSING) {
                a->state = L3P_CLOSED;
                a->error = FND_E_PEER_GONE;
            }
            continue;
        }
        if (a->kind == L3P_KIND_TXN) txn_packet(a, type, seq, data, len);
        else if (type == T_DATA || type == T_ACKONLY) stream_packet(a, type, flags, seq, ack, window, data, len);
    }
}

int32_t l3p_poll(l3p_assoc *a) {
    if (!a) return FND_E_BAD_ARG;
    if (a->state == L3P_BROKEN || a->state == L3P_CLOSED) return status(a);
    receive(a);
    uint32_t t = now();
    if (a->ctrl_type) {
        if (t - a->ctrl_first > (a->ctrl_type == T_CLOSE ? CLOSE_WAIT : OPEN_GIVEUP)) {
            if (a->ctrl_type == T_CLOSE) {
                a->state = L3P_CLOSED;
                a->error = FND_E_PEER_GONE;
            } else {
                fail(a, FND_E_TIMEOUT, 0);
            }
            a->ctrl_type = 0;
            return status(a);
        }
        if ((int32_t)(t - a->ctrl_next) >= 0) {
            a->ctrl_interval = a->ctrl_interval * 2u > RTO_MAX ? RTO_MAX : a->ctrl_interval * 2u;
            a->ctrl_next = t + a->ctrl_interval;
            if (a->ctrl_type == T_OPEN) {
                uint8_t open[8];
                put32(open, a->nonce);
                put32(open + 4, 0);
                send(a, T_OPEN, 0, 0, 0, 0, a->ctrl_window, open, 8);
            } else {
                send(a, T_CLOSE, 0, a->conn, 0, 0, 0, 0, 0);
            }
        }
    }
    if (a->state == L3P_OPEN) {
        if (t - a->last_rx > a->idle) {
            a->state = L3P_BROKEN;
            a->error = FND_E_TIMEOUT;
            return status(a);
        }
        if (a->kind == L3P_KIND_TXN) {
            txn_tick(a, t);
        } else {
            stream_tick(a, t);
            stream_pump(a);
            if (a->need_ack && a->state == L3P_OPEN) {
                send(a, T_ACKONLY, 0, a->conn, 0, a->rcv_nxt, rcv_window(a), 0, 0);
                a->need_ack = 0;
            }
        }
        if (a->state == L3P_OPEN && t - a->last_tx >= a->idle / 4u) {
            if (a->kind == L3P_KIND_TXN) send(a, T_ACKONLY, 0, a->conn, 0, min_unacked(a), 0, 0, 0);
            else send(a, T_ACKONLY, 0, a->conn, 0, a->rcv_nxt, rcv_window(a), 0, 0);
        }
    }
    return status(a);
}

/* ---- public calls ---- */

/* §4.1: NONCEs (and UREQ sequence numbers) that a restarted node does not
 * repeat. The tick count after boot repeats (exactly so on the emulator,
 * whose RTC also starts from 0 unless set to real time), and a responder
 * answers a known (address, socket, NONCE) with the old association's
 * ACCEPT, whose XIDs are already used. So each process takes a seed from
 * /dev/trng once, or the RTC seconds where there is no TRNG. */
static uint32_t entropy(void) {
    static uint32_t seed, seeded;
    if (!seeded) {
        fnd_handle h;
        uint32_t got = 0;
        seeded = 1;
        if (fnd_file_open("/dev/trng", FND_OPEN_READ, 0, &h) == FND_OK) {
            if (fnd_read(h, &seed, sizeof seed, &got) != FND_OK || got != sizeof seed) seed = 0;
            fnd_close(h);
        }
        seed ^= (uint32_t)fnd_sys_time_get(FND_TIME_RTC_LO) * 0x85EBCA6Bu;
    }
    return seed;
}

int32_t l3p_open(uint32_t addr, uint32_t socket, uint32_t kind, uint32_t opts, l3p_assoc *a) {
    static uint32_t counter;
    if (!a || (kind != L3P_KIND_TXN && kind != L3P_KIND_STREAM) || !socket || socket > 254u ||
        FND_CNET_NODE(addr) == 0xFFFFu)
        return FND_E_BAD_ARG;
    if (opts & ~L3P_OPT_ENC) return FND_E_BAD_ARG;
    if (opts & L3P_OPT_ENC) return FND_E_UNSUPPORTED;
    uint32_t *w = (uint32_t *)a;
    for (uint32_t i = 0; i < sizeof *a / 4u; ++i) w[i] = 0;
    int32_t st = cnet_open(0, &a->handle);
    if (st != FND_OK) return st;
    st = cnet_wait_ready(a->handle, READY_TICKS);
    if (st != FND_OK) {
        cnet_close(a->handle);
        return st;
    }
    cnet_info info;
    cnet_info_get(a->handle, &info);
    a->peer = addr;
    a->peer_socket = socket;
    a->kind = kind;
    a->state = L3P_OPENING;
    a->window = L3P_TXN_WINDOW;
    a->next_xid = 1;
    a->rto = RTO_FIRST;
    a->peer_window = INFLIGHT * L3P_MAX_PLAIN;
    a->idle = 12000u;
    a->nonce = (now() * 2654435761u) ^ (info.addr * 40503u) ^ (info.socket << 24) ^ (++counter * 0x9E3779B9u) ^
               entropy();
    if (!a->nonce) a->nonce = 1;
    a->ctrl_window = kind == L3P_KIND_TXN ? L3P_TXN_WINDOW : L3P_STREAM_BUF;
    uint32_t t = now();
    a->ctrl_type = T_OPEN;
    a->ctrl_first = t;
    a->ctrl_interval = RTO_FIRST;
    a->ctrl_next = t + RTO_FIRST;
    uint8_t open[8];
    put32(open, a->nonce);
    put32(open + 4, 0);
    send(a, T_OPEN, 0, 0, 0, 0, a->ctrl_window, open, 8);
    while (a->state == L3P_OPENING) {
        l3p_poll(a);
        if (a->state == L3P_OPENING) fnd_sys_sleep(1);
    }
    if (a->state != L3P_OPEN) {
        st = a->error;
        cnet_close(a->handle);
        a->handle = 0;
        return st;
    }
    return FND_OK;
}

uint32_t l3p_mtu(l3p_assoc *a) {
    (void)a;
    return L3P_MAX_PLAIN;
}

int32_t l3p_info(l3p_assoc *a, l3p_info_t *info) {
    if (!a || !info) return FND_E_BAD_ARG;
    info->state = a->state;
    info->peer = a->peer;
    info->peer_socket = a->peer_socket;
    info->conn = a->conn;
    info->encrypted = 0;
    info->eof = a->kind == L3P_KIND_STREAM && a->rcv_fin && !a->rlen;
    info->sent_all = a->fin_acked;
    info->send_space = a->kind == L3P_KIND_STREAM && !a->fin_queued ? L3P_STREAM_BUF - a->slen : 0;
    info->recv_ready = a->kind == L3P_KIND_STREAM ? a->rlen : 0;
    info->reset_reason = a->reset_reason;
    copy(info->reset_message, a->reset_message, sizeof info->reset_message);
    return FND_OK;
}

int32_t l3p_txn_send(l3p_assoc *a, const void *req, uint32_t len, uint32_t *xid) {
    if (!a || a->kind != L3P_KIND_TXN || (len && !req) || !xid) return FND_E_BAD_ARG;
    if (len > L3P_MAX_PLAIN) return FND_E_LIMIT;
    int32_t st = l3p_poll(a);
    if (st != FND_OK) return st;
    if (a->state != L3P_OPEN) return FND_E_BAD_HANDLE;
    uint32_t x = a->next_xid, i = x % 4u;
    if (x >= min_unacked(a) + a->window || a->slot_state[i] != S_FREE) return FND_E_WOULD_BLOCK;
    copy(req_buf(a, i), req, len);
    uint32_t t = now();
    a->slot_xid[i] = x;
    a->slot_len[i] = len;
    a->slot_state[i] = S_SENT;
    a->slot_first[i] = t;
    a->slot_interval[i] = RTO_FIRST;
    a->slot_next[i] = t + RTO_FIRST;
    a->next_xid = x + 1u;
    send(a, T_REQ, 0, a->conn, x, min_unacked(a), 0, req_buf(a, i), len);
    *xid = x;
    return FND_OK;
}

int32_t l3p_txn_wait(l3p_assoc *a, uint32_t xid, void *rsp, uint32_t cap, uint32_t *rlen) {
    if (!a || a->kind != L3P_KIND_TXN || !rlen || (cap && !rsp)) return FND_E_BAD_ARG;
    uint32_t i = xid % 4u;
    if (a->slot_xid[i] != xid || a->slot_state[i] == S_FREE) return FND_E_BAD_ARG;
    while (a->slot_state[i] != S_DONE) {
        int32_t st = l3p_poll(a);
        if (st != FND_OK) return st;
        if (a->slot_state[i] != S_DONE) fnd_sys_sleep(1);
    }
    a->slot_state[i] = S_FREE;
    if (a->slot_len[i] > cap) return FND_E_LIMIT;
    copy(rsp, rsp_buf(a, i), a->slot_len[i]);
    *rlen = a->slot_len[i];
    return FND_OK;
}

int32_t l3p_txn_call(l3p_assoc *a, const void *req, uint32_t len, void *rsp, uint32_t cap, uint32_t *rlen) {
    uint32_t xid;
    for (;;) {
        int32_t st = l3p_txn_send(a, req, len, &xid);
        if (st == FND_OK) break;
        if (st != FND_E_WOULD_BLOCK) return st;
        fnd_sys_sleep(1); /* another request holds the window */
    }
    return l3p_txn_wait(a, xid, rsp, cap, rlen);
}

int32_t l3p_write(l3p_assoc *a, const void *data, uint32_t len, uint32_t *written) {
    if (!a || a->kind != L3P_KIND_STREAM || (len && !data) || !written) return FND_E_BAD_ARG;
    *written = 0;
    int32_t st = status(a);
    if (st != FND_OK) return st;
    if (a->state != L3P_OPEN || a->fin_queued) return FND_E_BAD_HANDLE;
    uint32_t room = L3P_STREAM_BUF - a->slen, n = len < room ? len : room;
    uint32_t tail = (a->snd_una + a->slen) % L3P_STREAM_BUF;
    const uint8_t *p = data;
    for (uint32_t i = 0; i < n; ++i) sbuf(a)[(tail + i) % L3P_STREAM_BUF] = p[i];
    a->slen += n;
    *written = n;
    stream_pump(a);
    return FND_OK;
}

int32_t l3p_read(l3p_assoc *a, void *buf, uint32_t cap, uint32_t *got) {
    if (!a || a->kind != L3P_KIND_STREAM || (cap && !buf) || !got) return FND_E_BAD_ARG;
    *got = 0;
    if (!a->rlen) {
        if (a->rcv_fin) return FND_END;
        int32_t st = status(a);
        return st != FND_OK ? st : FND_E_WOULD_BLOCK;
    }
    uint32_t n = a->rlen < cap ? a->rlen : cap;
    uint8_t *p = buf;
    for (uint32_t i = 0; i < n; ++i) p[i] = rbuf(a)[(a->rhead + i) % L3P_STREAM_BUF];
    uint32_t before = rcv_window(a);
    a->rhead = (a->rhead + n) % L3P_STREAM_BUF;
    a->rlen -= n;
    if (before < L3P_MAX_PLAIN && rcv_window(a) >= L3P_MAX_PLAIN && a->state == L3P_OPEN)
        send(a, T_ACKONLY, 0, a->conn, 0, a->rcv_nxt, rcv_window(a), 0, 0); /* window update */
    *got = n;
    return FND_OK;
}

int32_t l3p_shutdown(l3p_assoc *a) {
    if (!a || a->kind != L3P_KIND_STREAM) return FND_E_BAD_ARG;
    int32_t st = status(a);
    if (st != FND_OK) return st;
    if (!a->fin_queued) {
        a->fin_queued = 1;
        stream_pump(a);
    }
    return FND_OK;
}

int32_t l3p_close(l3p_assoc *a) {
    if (!a) return FND_E_BAD_ARG;
    if (a->state == L3P_OPEN) {
        uint32_t start = now();
        if (a->kind == L3P_KIND_TXN) {
            while (a->state == L3P_OPEN && min_unacked(a) != a->next_xid) {
                l3p_poll(a);
                fnd_sys_sleep(1);
            }
        } else {
            l3p_shutdown(a);
            while (a->state == L3P_OPEN && !(a->fin_acked && a->rcv_fin) && now() - start < FIN_WAIT) {
                l3p_poll(a);
                fnd_sys_sleep(1);
            }
        }
    }
    if (a->state == L3P_OPEN) {
        uint32_t t = now();
        a->state = L3P_CLOSING;
        a->ctrl_type = T_CLOSE;
        a->ctrl_first = t;
        a->ctrl_interval = RTO_FIRST;
        a->ctrl_next = t + RTO_FIRST;
        send(a, T_CLOSE, 0, a->conn, 0, 0, 0, 0, 0);
        while (a->state == L3P_CLOSING) { /* l3p_poll resends CLOSE and gives up after 2 s */
            l3p_poll(a);
            if (a->state == L3P_CLOSING) fnd_sys_sleep(1);
        }
    }
    if (a->handle) cnet_close(a->handle);
    a->handle = 0;
    if (a->state != L3P_BROKEN) {
        a->state = L3P_CLOSED;
        a->error = FND_E_STALE;
    }
    return FND_OK;
}

int32_t l3p_abort(l3p_assoc *a) {
    if (!a) return FND_E_BAD_ARG;
    if ((a->state == L3P_OPEN || a->state == L3P_CLOSING) && a->conn && a->handle) {
        uint8_t r[4] = {0, (uint8_t)L3P_R_ABORT, 0, 0};
        send(a, T_RESET, 0, a->conn, 0, 0, 0, r, 4);
    }
    if (a->handle) cnet_close(a->handle);
    a->handle = 0;
    a->state = L3P_BROKEN;
    a->error = FND_E_STALE;
    return FND_OK;
}

int32_t l3p_ucall(uint32_t addr, uint32_t socket, const void *req, uint32_t len, void *out, uint32_t cap,
                  uint32_t ticks, uint32_t *count) {
    static uint32_t counter;
    static uint32_t buf[FND_CNET_MTU / 4u];
    if (!socket || socket > 254u || (len && !req) || !count || (cap && !out)) return FND_E_BAD_ARG;
    if (len > L3P_MAX_PLAIN) return FND_E_LIMIT;
    *count = 0;
    fnd_handle h;
    int32_t st = cnet_open(0, &h);
    if (st != FND_OK) return st;
    st = cnet_wait_ready(h, READY_TICKS);
    if (st != FND_OK) {
        cnet_close(h);
        return st;
    }
    cnet_info info;
    cnet_info_get(h, &info);
    uint32_t seq = (now() * 2654435761u) ^ (info.addr * 40503u) ^ (++counter * 0x9E3779B9u) ^ entropy();
    if (!seq) seq = 1;
    uint8_t *p = (uint8_t *)buf;
    int broadcast = FND_CNET_NODE(addr) == 0xFFFFu;
    uint32_t used = 0, start = now(), next = start, interval = RTO_FIRST;
    uint8_t *o = out;
    for (;;) {
        uint32_t t = now();
        if (t - start >= ticks || (!broadcast && *count)) break;
        if ((int32_t)(t - next) >= 0) {
            header(p, 1u, T_UREQ, 0, 0, seq, 0, 0);
            copy(p + HDR, req, len);
            cnet_sendto(h, addr, socket, PROTO_L3P, p, HDR + len);
            next = t + interval;
            interval = interval * 2u > RTO_MAX ? RTO_MAX : interval * 2u;
        }
        int any = 0;
        for (;;) {
            cnet_peer from;
            int32_t n = cnet_recvfrom(h, p, FND_CNET_MTU, FND_CNET_NONBLOCK, &from);
            if (n < 0) break;
            any = 1;
            if (from.proto != PROTO_L3P || (uint32_t)n < HDR || p[1] != T_URSP || get32(p + 8) != seq ||
                p[0] != 0x11u)
                continue;
            if (!broadcast && (FND_CNET_NODE(from.addr) != FND_CNET_NODE(addr) || from.socket != socket)) continue;
            int dup = 0;
            for (uint32_t k = 0, off = 0; k < *count; ++k) {
                uint32_t *rec = (uint32_t *)(o + off);
                if (FND_CNET_NODE(rec[0]) == FND_CNET_NODE(from.addr) && rec[1] == from.socket) dup = 1;
                off += 12u + ((rec[2] + 3u) & ~3u);
            }
            uint32_t dlen = (uint32_t)n - HDR, need = 12u + ((dlen + 3u) & ~3u);
            if (dup || used + need > cap) continue;
            uint32_t *rec = (uint32_t *)(o + used);
            rec[0] = from.addr;
            rec[1] = from.socket;
            rec[2] = dlen;
            copy(o + used + 12u, p + HDR, dlen);
            used += need;
            ++*count;
        }
        if (!any) fnd_sys_sleep(1);
    }
    cnet_close(h);
    return broadcast || *count ? FND_OK : FND_E_TIMEOUT;
}
