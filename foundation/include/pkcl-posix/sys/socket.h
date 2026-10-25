#ifndef PKCL_SYS_SOCKET_H
#define PKCL_SYS_SOCKET_H
#include <sys/types.h>

/* Foundation has no network stack (doc/spec_pkcl.md §6.8). These declarations let code
 * that mentions sockets compile; every call fails with ENOSYS. */
typedef unsigned int socklen_t;
typedef unsigned short sa_family_t;

#define AF_UNSPEC 0
#define AF_UNIX 1
#define AF_INET 2
#define AF_INET6 10
#define AF_LOCAL AF_UNIX
#define PF_UNSPEC AF_UNSPEC
#define PF_UNIX AF_UNIX
#define PF_INET AF_INET
#define PF_INET6 AF_INET6
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOCK_RAW 3
#define SOCK_RDM 4
#define SOCK_SEQPACKET 5
#define SHUT_RD 0
#define SHUT_WR 1
#define SHUT_RDWR 2
#define MSG_PEEK 2
#define MSG_DONTWAIT 0x40
#define MSG_NOSIGNAL 0x4000
#define SO_ERROR 4
#define SO_BROADCAST 6
#define SO_KEEPALIVE 9
#define SO_RCVBUF 8
#define SO_SNDBUF 7
#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#define SOMAXCONN 128

struct sockaddr {
    sa_family_t sa_family;
    char sa_data[14];
};

struct sockaddr_storage {
    sa_family_t ss_family;
    char ss_pad[126];
};

int shutdown(int fd, int how);
int socket(int domain, int type, int protocol);
int connect(int fd, const struct sockaddr *addr, socklen_t len);
int bind(int fd, const struct sockaddr *addr, socklen_t len);
int listen(int fd, int backlog);
int accept(int fd, struct sockaddr *addr, socklen_t *len);
int setsockopt(int fd, int level, int name, const void *value, socklen_t len);
int getsockopt(int fd, int level, int name, void *value, socklen_t *len);
int getsockname(int fd, struct sockaddr *addr, socklen_t *len);
int getpeername(int fd, struct sockaddr *addr, socklen_t *len);
ssize_t send(int fd, const void *buf, size_t len, int flags);
ssize_t sendto(int fd, const void *buf, size_t len, int flags, const struct sockaddr *to, socklen_t tolen);
ssize_t recv(int fd, void *buf, size_t len, int flags);

#endif
