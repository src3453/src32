#ifndef FOUNDATION_LIBCNET_H
#define FOUNDATION_LIBCNET_H
#include <foundation/cnet.h>
#include <foundation/fnd.h>

/* libcnet: the WAL "cnet" 1.0 (/lib/cnet.1.wal), datagram sockets over
 * /dev/cnet (doc/spec_Foundation_API.md §9.6, doc/spec_NET.md).
 *
 * A socket is a /dev/cnet stream bound to one 8-bit CNET socket number;
 * close it with cnet_close (or fnd_close). Addresses are NET << 16 | NODE
 * (FND_CNET_ADDR); NET 0 means "this network" and NODE 0xFFFF broadcasts.
 * Datagrams carry up to FND_CNET_MTU bytes and may be lost: there is no
 * retransmission. The network card acquires its node number after the
 * first open, which takes about 100 ms; cnet_wait_ready waits for it.
 *
 * WAL calls carry only 32-bit scalars and pointers. */

typedef struct {
    uint32_t addr;   /* sender NET << 16 | NODE */
    uint32_t socket; /* sender socket */
    uint32_t proto;  /* PROTO byte */
    uint32_t flags;  /* FND_CNET_BROADCAST */
} cnet_peer;

typedef struct {
    uint32_t status; /* FND_CNET_LINK, _PROBING, _READY, _CONFLICT */
    uint32_t addr;   /* own address, 0 until READY */
    uint32_t socket; /* the socket this stream is bound to */
} cnet_info;

/* Opens /dev/cnet and binds socket (1..254 except 4, or 0 for a dynamic
 * one from 128..254). FND_E_NO_MEDIA without a CNET card, FND_E_EXISTS if
 * the socket is taken, FND_E_LIMIT when no socket or card handle is free. */
int32_t cnet_open(uint32_t socket, fnd_handle *out);
int32_t cnet_close(fnd_handle h);
int32_t cnet_info_get(fnd_handle h, cnet_info *info);
/* Sleeps one tick at a time until the card has an address. FND_OK,
 * FND_E_NO_MEDIA (no link), FND_E_ADDRESS_CONFLICT, or FND_E_TIMEOUT. */
int32_t cnet_wait_ready(fnd_handle h, uint32_t ticks);
/* FND_E_BUSY while the node is being acquired, FND_E_NOT_FOUND for another
 * network (no routers yet), FND_E_LIMIT above the MTU. */
int32_t cnet_sendto(fnd_handle h, uint32_t addr, uint32_t socket, uint32_t proto, const void *data,
                    uint32_t len);
/* Receives one datagram into buf and returns its length (>= 0). Blocks
 * unless flags has FND_CNET_NONBLOCK (then FND_E_WOULD_BLOCK when empty).
 * FND_E_LIMIT if cap is too small; the datagram then stays queued.
 * from may be 0. */
int32_t cnet_recvfrom(fnd_handle h, void *buf, uint32_t cap, uint32_t flags, cnet_peer *from);
/* Parses "NET.NODE" (hex) into *addr. A node on the local network may be
 * written ".NODE" or "NODE"; both give NET 0 (this network).
 * FND_E_BAD_ARG otherwise. */
int32_t cnet_parse_addr(const char *text, uint32_t *addr);
/* Writes "NNNN:NNNN" and a NUL into out[10]. */
void cnet_format_addr(uint32_t addr, char *out);

#endif
