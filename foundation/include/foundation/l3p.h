#ifndef FOUNDATION_L3P_H
#define FOUNDATION_L3P_H
#include <foundation/libcnet.h>

/* libl3p: the WAL "l3p" 1.0 (/lib/l3p.1.wal), the L3P transport over CNET
 * (doc/spec_NET_L3P.md). It imports fnd only: libcnet's code is linked in,
 * because a program importing fnd, cnet and l3p would need more than the 8
 * regions of a process. Programs that also want the libcnet address helpers
 * link lib/cnet/cnet.c in too, rather than importing cnet.
 *
 * This version implements the opening side: transaction associations
 * (exactly-once, ordered requests, §5), stream associations (§6) and
 * connectionless queries (UREQ, §5.5). Each association owns one dynamic
 * CNET socket. Encryption (§7) and listening are not implemented yet:
 * L3P_OPT_ENC gives FND_E_UNSUPPORTED.
 *
 * The caller provides the l3p_assoc storage (about 9 KiB; static or on a
 * large stack) and drives it from one thread. Retransmission and timeouts
 * run inside the calls: the blocking ones (open, txn_wait/call, close) poll
 * and sleep one tick at a time, and an event loop calls l3p_poll often
 * (every tick) between the nonblocking l3p_read / l3p_write.
 *
 * Errors: FND_E_TIMEOUT when the peer stopped answering (the association
 * is then broken; later calls give FND_E_STALE), FND_E_IO when the peer
 * sent RESET (l3p_info has the reason and message), FND_E_PEER_GONE after
 * the peer closed the association, and the libcnet errors from opening. */

#define L3P_KIND_TXN 1u
#define L3P_KIND_STREAM 2u
#define L3P_OPT_ENC 1u

#define L3P_MAX_PLAIN 1004u
#define L3P_TXN_WINDOW 4u
#define L3P_STREAM_BUF 4096u

/* l3p_info.state */
#define L3P_OPENING 1u
#define L3P_OPEN 2u
#define L3P_CLOSING 3u
#define L3P_CLOSED 4u
#define L3P_BROKEN 5u

/* RESET reasons (§4.2) */
#define L3P_R_NO_LISTENER 1u
#define L3P_R_BUSY 2u
#define L3P_R_REFUSED 3u
#define L3P_R_ENC_REQUIRED 4u
#define L3P_R_ENC_UNSUPPORTED 5u
#define L3P_R_AUTH_FAILED 6u
#define L3P_R_UNKNOWN_CONN 7u
#define L3P_R_TIMEOUT 8u
#define L3P_R_ABORT 9u

typedef struct {
    uint32_t state;        /* L3P_OPENING .. L3P_BROKEN */
    uint32_t peer;         /* NET << 16 | NODE */
    uint32_t peer_socket;
    uint32_t conn;
    uint32_t encrypted;    /* 0 in this version */
    uint32_t eof;          /* stream: the peer's FIN was received and all data read */
    uint32_t sent_all;     /* stream: our FIN was acknowledged */
    uint32_t send_space;   /* stream: bytes l3p_write would take now */
    uint32_t recv_ready;   /* stream: bytes l3p_read would return now */
    uint32_t reset_reason; /* L3P_R_* of a RESET from the peer, else 0 */
    char reset_message[132];
} l3p_info_t;

/* Internal state. Keep the layout opaque: it may change with the WAL
 * minor version, but its size only grows. */
typedef struct {
    uint32_t handle, peer, peer_socket, kind, state, conn, nonce, options, idle;
    uint32_t last_rx, last_tx;
    int32_t error;
    uint32_t reset_reason;
    char reset_message[132];
    uint32_t ctrl_type, ctrl_first, ctrl_next, ctrl_interval, ctrl_window;
    /* transactions */
    uint32_t window, next_xid;
    uint32_t slot_xid[4], slot_state[4], slot_len[4], slot_first[4], slot_next[4], slot_interval[4];
    /* streams */
    uint32_t snd_una, snd_nxt, slen, inflight[4], ninflight, fin_queued, fin_sent, fin_acked;
    uint32_t rcv_nxt, rcv_fin, rhead, rlen, peer_window, rto, rto_next, progress, need_ack;
    uint32_t mem[2u * L3P_STREAM_BUF / 4u + 8u]; /* txn: 4 requests + 4 responses (8032 bytes); stream: two 4 KiB rings */
    uint32_t packet[1024u / 4u];
} l3p_assoc;

/* Opens an association to addr:socket and waits (up to 15 s) until it is
 * established. Opens and binds a dynamic CNET socket and waits for the
 * card's address first. */
int32_t l3p_open(uint32_t addr, uint32_t socket, uint32_t kind, uint32_t opts, l3p_assoc *a);
/* Handles waiting datagrams and timers without blocking. FND_OK while the
 * association is usable (or closing), else its error. */
int32_t l3p_poll(l3p_assoc *a);
uint32_t l3p_mtu(l3p_assoc *a);
int32_t l3p_info(l3p_assoc *a, l3p_info_t *info);

/* Transactions. txn_send queues a request without waiting
 * (FND_E_WOULD_BLOCK while the window is full) and gives its XID;
 * txn_wait waits for that XID's response, which must fit in cap
 * (FND_E_LIMIT otherwise; the response is dropped). */
int32_t l3p_txn_send(l3p_assoc *a, const void *req, uint32_t len, uint32_t *xid);
int32_t l3p_txn_wait(l3p_assoc *a, uint32_t xid, void *rsp, uint32_t cap, uint32_t *rlen);
int32_t l3p_txn_call(l3p_assoc *a, const void *req, uint32_t len, void *rsp, uint32_t cap, uint32_t *rlen);

/* Streams, nonblocking. l3p_write takes as many bytes as fit (*written may
 * be less than len, or 0). l3p_read returns FND_OK with *got > 0,
 * FND_E_WOULD_BLOCK when nothing is ready, or FND_END at the end of the
 * peer's data. l3p_shutdown sends our FIN after the queued data. */
int32_t l3p_write(l3p_assoc *a, const void *data, uint32_t len, uint32_t *written);
int32_t l3p_read(l3p_assoc *a, void *buf, uint32_t cap, uint32_t *got);
int32_t l3p_shutdown(l3p_assoc *a);

/* Ends the association: waits for outstanding requests (transactions) or
 * for both FINs (streams, up to 5 s), sends CLOSE and waits for CLOSED (up
 * to 2 s), then closes the CNET socket. l3p_abort sends RESET(ABORT) and
 * closes at once. Both may be called on a broken association. */
int32_t l3p_close(l3p_assoc *a);
int32_t l3p_abort(l3p_assoc *a);

/* Connectionless query (§5.5) from a fresh dynamic socket. Unicast returns
 * after the first answer, broadcast (NODE 0xFFFF) collects answers for
 * ticks. Each answer is stored in out as addr u32, socket u32, len u32 and
 * the data padded to 4 bytes (host byte order words); answers that do not
 * fit are dropped. *count is the number stored. FND_E_TIMEOUT if a unicast
 * query got no answer. */
int32_t l3p_ucall(uint32_t addr, uint32_t socket, const void *req, uint32_t len, void *out, uint32_t cap,
                  uint32_t ticks, uint32_t *count);

#endif
