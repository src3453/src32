#ifndef PKCL_NETDB_H
#define PKCL_NETDB_H
#include <sys/socket.h>

/* No resolver (doc/spec_pkcl.md §6.8): declarations only. */
struct hostent {
    char *h_name;
    char **h_aliases;
    int h_addrtype;
    int h_length;
    char **h_addr_list;
};

struct addrinfo {
    int ai_flags;
    int ai_family;
    int ai_socktype;
    int ai_protocol;
    socklen_t ai_addrlen;
    struct sockaddr *ai_addr;
    char *ai_canonname;
    struct addrinfo *ai_next;
};

#define AI_PASSIVE 1
#define AI_CANONNAME 2
#define AI_NUMERICHOST 4
#define NI_NUMERICHOST 1
#define NI_NUMERICSERV 2
#define EAI_NONAME (-2)
#define EAI_FAIL (-4)
#define EAI_SYSTEM (-11)

struct hostent *gethostbyname(const char *name);
int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **res);
void freeaddrinfo(struct addrinfo *res);
const char *gai_strerror(int code);
int getnameinfo(const struct sockaddr *addr, socklen_t len, char *host, socklen_t hostlen,
                char *service, socklen_t servicelen, int flags);

#endif
