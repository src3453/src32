/* /dev/cnet: datagram sockets on the CNET expansion card (doc/spec_NET.md,
 * spec_Foundation_API.md §9.6). The card runs the network stack; the kernel
 * finds it by probing the expansion slots, and each stream owns one of the
 * card's 16 socket handles. Calls run with IRQs disabled. An empty blocking
 * receive returns CNET_PARK, and the scheduler tick retries it like the PeC
 * reads. User data always passes through mm_copy_*. */
#include "internal.h"
#include <foundation/cnet.h>
#include <foundation/fnd.h>

#ifndef CNET_READ8
#define CNET_READ8(a) (*(volatile uint8_t *)(uintptr_t)(a))
#define CNET_WRITE8(a, v) (*(volatile uint8_t *)(uintptr_t)(a) = (uint8_t)(v))
#endif

#define EXPANSION_BASE 0xA0000000u
#define SLOT_SIZE 0x02000000u
#define SLOTS 8u
#define HANDLES 16u

#define R_STATUS 0x06u
#define R_CONTROL 0x08u
#define R_NET 0x0Cu
#define R_NODE 0x0Eu
#define H_BASE 0x0100u
#define H_STRIDE 0x20u
#define H_SOCKET 0x00u
#define H_STATE 0x01u
#define H_CMD 0x02u
#define H_RESULT 0x03u
#define H_RX_COUNT 0x04u
#define H_TX_NET 0x08u
#define H_TX_NODE 0x0Au
#define H_TX_SOCKET 0x0Cu
#define H_TX_PROTO 0x0Du
#define H_TX_LEN 0x0Eu
#define H_RX_NET 0x10u
#define H_RX_NODE 0x12u
#define H_RX_SOCKET 0x14u
#define H_RX_PROTO 0x15u
#define H_RX_LEN 0x16u
#define H_RX_FLAGS 0x18u
#define TX_BASE 0x10000u
#define RX_BASE 0x20000u
#define BUF_STRIDE 0x400u

#define CMD_OPEN 1u
#define CMD_CLOSE 2u
#define CMD_SEND 3u
#define CMD_RX_POP 4u

/* Stream badge: open rights in bits 0-1, card handle + 1 in bits 8-12. */
#define BADGE_RIGHTS 0x3u
#define BADGE_HANDLE(b) (((b) >> 8) & 0x1Fu)

static uint32_t card;   /* slot base of the card, 0 before a successful probe */
static uint32_t in_use; /* bit h: card handle h belongs to a stream */

static uint32_t rd8(uint32_t off) { return CNET_READ8(card + off); }
static void wr8(uint32_t off, uint32_t v) { CNET_WRITE8(card + off, v); }
static uint32_t rd16(uint32_t off) { return rd8(off) << 8 | rd8(off + 1u); }
static void wr16(uint32_t off, uint32_t v) {
    wr8(off, v >> 8);
    wr8(off + 1u, v);
}

/* Empty slots read 0 without a bus error (spec_WASM_expansion_card.md §6). */
static int probe(void) {
    if (card) return 1;
    for (uint32_t slot = 0; slot < SLOTS; ++slot) {
        uint32_t base = EXPANSION_BASE + slot * SLOT_SIZE;
        if (CNET_READ8(base) == 'C' && CNET_READ8(base + 1u) == 'N' && CNET_READ8(base + 2u) == 'E' &&
            CNET_READ8(base + 3u) == 'T' && CNET_READ8(base + 4u) == 0 && CNET_READ8(base + 5u) == 1u) {
            card = base;
            return 1;
        }
    }
    return 0;
}

int32_t cnet_open(uint32_t flags) {
    if (!flags || (flags & ~(FND_OPEN_READ | FND_OPEN_WRITE))) return FND_E_BAD_ARG;
    if (!probe()) return FND_E_NO_MEDIA;
    /* The first open turns the link on; the card then acquires its node. */
    if (!(rd8(R_CONTROL + 1u) & 1u)) wr16(R_CONTROL, 1u);
    return FND_OK;
}

void cnet_close(uint32_t badge) {
    uint32_t h = BADGE_HANDLE(badge);
    if (!h || !card) return;
    --h;
    wr8(H_BASE + h * H_STRIDE + H_CMD, CMD_CLOSE);
    in_use &= ~(1u << h);
}

static int32_t send_error(uint32_t result) {
    switch (result) {
    case 0: return FND_OK;
    case 4: return FND_E_LIMIT;       /* BAD_LEN */
    case 5: return FND_E_BAD_ARG;     /* BAD_DEST */
    case 6:                           /* NO_ADDR */
        return (rd16(R_STATUS) & FND_CNET_CONFLICT) ? FND_E_ADDRESS_CONFLICT : FND_E_BUSY;
    case 7: return FND_E_NO_MEDIA;    /* NO_LINK */
    case 8: return FND_E_WOULD_BLOCK; /* DROPPED */
    case 9: return FND_E_NOT_FOUND;   /* NO_ROUTE */
    default: return FND_E_IO;
    }
}

static int32_t bind(uint32_t *badge, const fnd_msg *m, fnd_msg *r) {
    if (BADGE_HANDLE(*badge)) return FND_E_BUSY;
    if (m->a0 > 255u) return FND_E_BAD_ARG;
    uint32_t h = 0;
    while (h < HANDLES && (in_use & (1u << h))) ++h;
    if (h == HANDLES) return FND_E_LIMIT;
    uint32_t regs = H_BASE + h * H_STRIDE;
    if (rd8(regs + H_STATE)) wr8(regs + H_CMD, CMD_CLOSE); /* left open by an earlier kernel */
    wr8(regs + H_SOCKET, m->a0);
    wr8(regs + H_CMD, CMD_OPEN);
    switch (rd8(regs + H_RESULT)) {
    case 0: break;
    case 2: return FND_E_EXISTS;  /* IN_USE or reserved */
    case 3: return FND_E_LIMIT;   /* NO_SOCKET */
    default: return FND_E_IO;
    }
    in_use |= 1u << h;
    *badge = (*badge & BADGE_RIGHTS) | (h + 1u) << 8;
    r->a0 = rd8(regs + H_SOCKET);
    return FND_OK;
}

/* The stream's open card handle, or -1 when unbound or closed underneath. */
static int32_t bound(uint32_t badge) {
    uint32_t h = BADGE_HANDLE(badge);
    if (!h) return -1;
    --h;
    if (!(in_use & (1u << h)) || !rd8(H_BASE + h * H_STRIDE + H_STATE)) return -1;
    return (int32_t)h;
}

int32_t cnet_call(struct fnd_proc *proc, uint32_t *badge, const fnd_msg *m, fnd_msg *r, uint32_t cap) {
    uint32_t rights = *badge & BADGE_RIGHTS;
    switch (m->op) {
    case FND_CNET_BIND:
        return bind(badge, m, r);
    case FND_CNET_INFO: {
        r->a0 = rd16(R_STATUS);
        r->a1 = (r->a0 & FND_CNET_READY) ? FND_CNET_ADDR(rd16(R_NET), rd16(R_NODE)) : 0;
        int32_t h = bound(*badge);
        r->a2 = h < 0 ? 0 : rd8(H_BASE + (uint32_t)h * H_STRIDE + H_SOCKET);
        return FND_OK;
    }
    case FND_CNET_SENDTO: {
        uint8_t data[FND_CNET_MTU];
        if (!(rights & FND_OPEN_WRITE)) return FND_E_ACCESS;
        int32_t h = bound(*badge);
        if (h < 0) return FND_E_BAD_ARG;
        if (m->len > FND_CNET_MTU) return FND_E_LIMIT;
        if (m->a1 > 0xFFFFu) return FND_E_BAD_ARG;
        mm_copy_from_user(data, proc, m->buf, m->len);
        uint32_t regs = H_BASE + (uint32_t)h * H_STRIDE;
        uint32_t tx = TX_BASE + (uint32_t)h * BUF_STRIDE;
        for (uint32_t i = 0; i < m->len; ++i) wr8(tx + i, data[i]);
        wr16(regs + H_TX_NET, FND_CNET_NET(m->a0));
        wr16(regs + H_TX_NODE, FND_CNET_NODE(m->a0));
        wr8(regs + H_TX_SOCKET, m->a1 & 0xFFu);
        wr8(regs + H_TX_PROTO, m->a1 >> 8);
        wr16(regs + H_TX_LEN, m->len);
        wr8(regs + H_CMD, CMD_SEND);
        return send_error(rd8(regs + H_RESULT));
    }
    case FND_CNET_RECVFROM: {
        uint8_t data[FND_CNET_MTU];
        if (!(rights & FND_OPEN_READ)) return FND_E_ACCESS;
        if (m->a0 & ~FND_CNET_NONBLOCK) return FND_E_BAD_ARG;
        int32_t h = bound(*badge);
        if (h < 0) return FND_E_BAD_ARG;
        uint32_t regs = H_BASE + (uint32_t)h * H_STRIDE;
        if (!rd8(regs + H_RX_COUNT)) return (m->a0 & FND_CNET_NONBLOCK) ? FND_E_WOULD_BLOCK : CNET_PARK;
        uint32_t len = rd16(regs + H_RX_LEN);
        if (len > cap) return FND_E_LIMIT; /* the datagram stays queued */
        uint32_t rx = RX_BASE + (uint32_t)h * BUF_STRIDE;
        for (uint32_t i = 0; i < len; ++i) data[i] = (uint8_t)rd8(rx + i);
        r->a0 = FND_CNET_ADDR(rd16(regs + H_RX_NET), rd16(regs + H_RX_NODE));
        r->a1 = rd8(regs + H_RX_SOCKET) | rd8(regs + H_RX_PROTO) << 8 | rd8(regs + H_RX_FLAGS) << 16;
        wr8(regs + H_CMD, CMD_RX_POP);
        mm_copy_to_user(proc, r->buf, data, len);
        r->len = len;
        return FND_OK;
    }
    case FND_ST_STAT: {
        if (cap < sizeof(fnd_stat_t)) return FND_E_LIMIT;
        fnd_stat_t st = {0};
        st.type = FND_TYPE_DEVICE;
        st.mode = 0x36u;
        mm_copy_to_user(proc, r->buf, &st, sizeof st);
        r->len = sizeof st;
        return FND_OK;
    }
    case FND_ST_SYNC:
        return FND_OK;
    default:
        return FND_E_UNSUPPORTED;
    }
}
