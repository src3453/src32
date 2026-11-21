#ifndef FOUNDATION_CNET_H
#define FOUNDATION_CNET_H
#include <stdint.h>

/* /dev/cnet, the kernel CNET driver (doc/spec_Foundation_API.md §9.6).
 * One stream is one socket on the CNET card (doc/spec_NET.md). The wire
 * protocol lives on the card; the driver only moves datagrams. */

#define FND_CNET_BIND 0x1400u     /* a0 = socket (0: dynamic) -> a0 = socket */
#define FND_CNET_SENDTO 0x1401u   /* a0 = address, a1 = socket | proto << 8, data = payload */
#define FND_CNET_RECVFROM 0x1402u /* a0 = FND_CNET_NONBLOCK? -> a0 = address, a1 = socket | proto << 8 | flags << 16 */
#define FND_CNET_INFO 0x1403u     /* -> a0 = status, a1 = own address, a2 = bound socket */

#define FND_CNET_NONBLOCK 1u

/* Status bits (the card's STATUS register). */
#define FND_CNET_LINK 0x1u
#define FND_CNET_PROBING 0x2u
#define FND_CNET_READY 0x4u
#define FND_CNET_CONFLICT 0x8u

/* Received-datagram flags. */
#define FND_CNET_BROADCAST 0x1u

#define FND_CNET_MTU 1024u
#define FND_CNET_ECHO_SOCKET 4u
#define FND_CNET_PROTO_ECHO 0x02u
#define FND_CNET_BROADCAST_NODE 0xFFFFu

/* Addresses are NET << 16 | NODE. */
#define FND_CNET_ADDR(net, node) (((uint32_t)(net) << 16) | ((uint32_t)(node) & 0xFFFFu))
#define FND_CNET_NET(addr) ((uint32_t)(addr) >> 16)
#define FND_CNET_NODE(addr) ((uint32_t)(addr) & 0xFFFFu)

#endif
