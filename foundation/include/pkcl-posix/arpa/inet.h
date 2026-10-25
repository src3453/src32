#ifndef PKCL_ARPA_INET_H
#define PKCL_ARPA_INET_H
#include <netinet/in.h>

int inet_aton(const char *text, struct in_addr *addr);
char *inet_ntoa(struct in_addr addr);
int inet_pton(int family, const char *text, void *dst);
const char *inet_ntop(int family, const void *src, char *dst, socklen_t size);

#endif
