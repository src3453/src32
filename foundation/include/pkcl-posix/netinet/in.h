#ifndef PKCL_NETINET_IN_H
#define PKCL_NETINET_IN_H
#include <stdint.h>
#include <sys/socket.h>

typedef uint16_t in_port_t;
typedef uint32_t in_addr_t;

struct in_addr {
    in_addr_t s_addr;
};

struct in6_addr {
    uint8_t s6_addr[16];
};

struct sockaddr_in {
    sa_family_t sin_family;
    in_port_t sin_port;
    struct in_addr sin_addr;
    unsigned char sin_zero[8];
};

struct sockaddr_in6 {
    sa_family_t sin6_family;
    in_port_t sin6_port;
    uint32_t sin6_flowinfo;
    struct in6_addr sin6_addr;
    uint32_t sin6_scope_id;
};

#define INADDR_ANY ((in_addr_t)0)
#define INADDR_LOOPBACK ((in_addr_t)0x7f000001)
#define IPPROTO_TCP 6
#define IPPROTO_UDP 17
#define INET_ADDRSTRLEN 16
#define INET6_ADDRSTRLEN 46

/* SRC32 is big endian: network order is host order. */
#define htons(x) ((uint16_t)(x))
#define ntohs(x) ((uint16_t)(x))
#define htonl(x) ((uint32_t)(x))
#define ntohl(x) ((uint32_t)(x))

#endif
