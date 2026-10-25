/* ネットワークはない（doc/spec_pkcl.md §6.8）。BusyBox がリンクできるように、失敗する定義だけを置く。 */
#define _GNU_SOURCE 1
#include <arpa/inet.h>
#include <errno.h>
#include <netdb.h>
#include <sys/socket.h>

int h_errno;

static int unsupported(void) {
    errno = ENOSYS;
    return -1;
}
int socket(int d, int t, int p) { (void)d, (void)t, (void)p; return unsupported(); }
int bind(int fd, const struct sockaddr *a, socklen_t n) { (void)fd, (void)a, (void)n; return unsupported(); }
int connect(int fd, const struct sockaddr *a, socklen_t n) { (void)fd, (void)a, (void)n; return unsupported(); }
int listen(int fd, int n) { (void)fd, (void)n; return unsupported(); }
int getsockname(int fd, struct sockaddr *a, socklen_t *n) { (void)fd, (void)a, (void)n; return unsupported(); }
int getpeername(int fd, struct sockaddr *a, socklen_t *n) { (void)fd, (void)a, (void)n; return unsupported(); }
int setsockopt(int fd, int l, int o, const void *v, socklen_t n) { (void)fd, (void)l, (void)o, (void)v, (void)n; return unsupported(); }
ssize_t sendto(int fd, const void *b, size_t n, int f, const struct sockaddr *a, socklen_t l) {
    (void)fd, (void)b, (void)n, (void)f, (void)a, (void)l;
    return unsupported();
}

struct hostent *gethostbyname(const char *name) { (void)name; h_errno = HOST_NOT_FOUND; return 0; }
struct servent *getservbyname(const char *name, const char *proto) { (void)name, (void)proto; return 0; }
int getaddrinfo(const char *n, const char *s, const struct addrinfo *h, struct addrinfo **r) {
    (void)n, (void)s, (void)h, (void)r;
    return EAI_FAIL;
}
void freeaddrinfo(struct addrinfo *r) { (void)r; }
int getnameinfo(const struct sockaddr *a, socklen_t al, char *h, socklen_t hl, char *s, socklen_t sl, int f) {
    (void)a, (void)al, (void)h, (void)hl, (void)s, (void)sl, (void)f;
    return EAI_FAIL;
}
const char *gai_strerror(int code) { (void)code; return "name resolution is not available"; }
const char *hstrerror(int code) { (void)code; return "host not found"; }

int inet_aton(const char *text, struct in_addr *out) {
    uint32_t value = 0;
    for (int part = 0; part < 4; ++part) {
        if (*text < '0' || *text > '9')
            return 0;
        unsigned number = 0;
        while (*text >= '0' && *text <= '9')
            number = number * 10 + (unsigned)(*text++ - '0');
        if (number > 255 || (part < 3 && *text++ != '.'))
            return 0;
        value = value << 8 | number;
    }
    if (*text)
        return 0;
    out->s_addr = htonl(value);
    return 1;
}

char *inet_ntoa(struct in_addr in) {
    static char text[16];
    uint32_t value = ntohl(in.s_addr);
    char *p = text;
    for (int shift = 24; shift >= 0; shift -= 8) {
        unsigned byte = (value >> shift) & 255u;
        if (byte >= 100)
            *p++ = (char)('0' + byte / 100);
        if (byte >= 10)
            *p++ = (char)('0' + byte / 10 % 10);
        *p++ = (char)('0' + byte % 10);
        if (shift)
            *p++ = '.';
    }
    *p = 0;
    return text;
}
