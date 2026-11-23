/* libcnet, the WAL "cnet" 1.1 (foundation/include/foundation/libcnet.h). */
#include <foundation/libcnet.h>
#include <foundation/sys.h>

int32_t cnet_open(uint32_t socket, fnd_handle *out) {
    if (!out || socket > 255u) return FND_E_BAD_ARG;
    fnd_handle h;
    int32_t st = fnd_file_open("/dev/cnet", FND_OPEN_READ | FND_OPEN_WRITE, FND_MODE_DEFAULT, &h);
    if (st != FND_OK) return st;
    uint32_t args[4] = {socket, 0, 0, 0};
    st = fnd_call(h, FND_CNET_BIND, args, 0, 0, 0, 0, 0);
    if (st != FND_OK) {
        fnd_close(h);
        return st;
    }
    *out = h;
    return FND_OK;
}

int32_t cnet_close(fnd_handle h) { return fnd_close(h); }

int32_t cnet_info_get(fnd_handle h, cnet_info *info) {
    fnd_msg rep;
    int32_t st = fnd_call(h, FND_CNET_INFO, 0, 0, 0, 0, 0, &rep);
    if (st == FND_OK && info) {
        info->status = rep.a0;
        info->addr = rep.a1;
        info->socket = rep.a2;
    }
    return st;
}

int32_t cnet_wait_ready(fnd_handle h, uint32_t ticks) {
    for (uint32_t i = 0;; ++i) {
        cnet_info info;
        int32_t st = cnet_info_get(h, &info);
        if (st != FND_OK) return st;
        if (!(info.status & FND_CNET_LINK)) return FND_E_NO_MEDIA;
        if (info.status & FND_CNET_CONFLICT) return FND_E_ADDRESS_CONFLICT;
        if (info.status & FND_CNET_READY) return FND_OK;
        if (i >= ticks) return FND_E_TIMEOUT;
        fnd_sys_sleep(1);
    }
}

int32_t cnet_sendto(fnd_handle h, uint32_t addr, uint32_t socket, uint32_t proto, const void *data,
                    uint32_t len) {
    if (socket > 255u || proto > 255u) return FND_E_BAD_ARG;
    if (len > FND_CNET_MTU) return FND_E_LIMIT;
    uint32_t args[4] = {addr, socket | proto << 8, 0, 0};
    return fnd_call(h, FND_CNET_SENDTO, args, data, len, 0, 0, 0);
}

int32_t cnet_recvfrom(fnd_handle h, void *buf, uint32_t cap, uint32_t flags, cnet_peer *from) {
    uint32_t args[4] = {flags, 0, 0, 0};
    fnd_msg rep;
    int32_t st = fnd_call(h, FND_CNET_RECVFROM, args, 0, 0, buf, cap, &rep);
    if (st != FND_OK) return st;
    if (from) {
        from->addr = rep.a0;
        from->socket = rep.a1 & 0xFFu;
        from->proto = (rep.a1 >> 8) & 0xFFu;
        from->flags = rep.a1 >> 16;
    }
    return (int32_t)rep.len;
}

/* One number in `base` (10 or 16) with at most `max` value. Digits outside the
 * base (so A-F in a decimal string) are errors: the two notations never mix. */
static int32_t number_value(const char *text, uint32_t base, uint32_t max, uint32_t *out) {
    uint32_t value = 0, digits = 0;
    for (; *text; ++text, ++digits) {
        char c = *text;
        uint32_t d;
        if (c >= '0' && c <= '9') d = (uint32_t)(c - '0');
        else if (base == 16u && c >= 'a' && c <= 'f') d = (uint32_t)(c - 'a' + 10);
        else if (base == 16u && c >= 'A' && c <= 'F') d = (uint32_t)(c - 'A' + 10);
        else return FND_E_BAD_ARG;
        if (d >= base || digits >= 8u) return FND_E_BAD_ARG;
        value = value * base + d;
        if (value > max) return FND_E_BAD_ARG;
    }
    if (!digits) return FND_E_BAD_ARG;
    *out = value;
    return FND_OK;
}

/* Copies text into buf (cap bytes), dropping a trailing H/h. *base is 16 when
 * the suffix was there, else 10. */
static int32_t strip_hex_suffix(const char *text, char *buf, uint32_t cap, uint32_t *base) {
    uint32_t n = 0;
    while (text[n]) {
        if (n + 1u >= cap) return FND_E_BAD_ARG;
        buf[n] = text[n];
        ++n;
    }
    *base = 10u;
    if (n && (buf[n - 1u] == 'H' || buf[n - 1u] == 'h')) {
        *base = 16u;
        --n;
    }
    buf[n] = 0;
    return FND_OK;
}

static int32_t parse_addr_body(char *text, uint32_t base, uint32_t *addr) {
    char *dot = text;
    while (*dot && *dot != '.') ++dot;
    uint32_t n = 0, node;
    if (*dot == '.' && dot == text) {
        /* ".NODE" is this network, the same as "NODE". */
        if (number_value(dot + 1, base, 0xFFFFu, &node) != FND_OK) return FND_E_BAD_ARG;
    } else if (*dot) {
        *dot = 0;
        if (number_value(text, base, 0xFFFFu, &n) != FND_OK ||
            number_value(dot + 1, base, 0xFFFFu, &node) != FND_OK) return FND_E_BAD_ARG;
    } else if (number_value(text, base, 0xFFFFu, &node) != FND_OK) {
        return FND_E_BAD_ARG;
    }
    *addr = FND_CNET_ADDR(n, node);
    return FND_OK;
}

int32_t cnet_parse_addr(const char *text, uint32_t *addr) {
    if (!text || !addr) return FND_E_BAD_ARG;
    char buf[24];
    uint32_t base;
    if (strip_hex_suffix(text, buf, sizeof buf, &base) != FND_OK) return FND_E_BAD_ARG;
    return parse_addr_body(buf, base, addr);
}

int32_t cnet_parse_endpoint(const char *text, uint32_t *addr, uint32_t *socket) {
    if (!text || !addr || !socket) return FND_E_BAD_ARG;
    char buf[24];
    uint32_t base, a, s = *socket;
    if (strip_hex_suffix(text, buf, sizeof buf, &base) != FND_OK) return FND_E_BAD_ARG;
    char *colon = buf;
    while (*colon && *colon != ':') ++colon;
    char *sock_text = *colon == ':' ? colon + 1 : 0;
    *colon = 0;
    if (parse_addr_body(buf, base, &a) != FND_OK) return FND_E_BAD_ARG;
    /* One suffix covers the whole string, so SOCK uses the same base. 0..255. */
    if (sock_text && number_value(sock_text, base, 0xFFu, &s) != FND_OK) return FND_E_BAD_ARG;
    *addr = a;
    *socket = s;
    return FND_OK;
}

static uint32_t put_decimal(uint32_t value, char *out) {
    char tmp[10];
    uint32_t n = 0, len = 0;
    do {
        tmp[n++] = (char)('0' + value % 10u);
        value /= 10u;
    } while (value);
    while (n) out[len++] = tmp[--n];
    return len;
}

/* Decimal "NET.NODE" (at most 11 characters plus the terminator). */
void cnet_format_addr(uint32_t addr, char *out) {
    uint32_t len = put_decimal(addr >> 16, out);
    out[len++] = '.';
    len += put_decimal(addr & 0xFFFFu, out + len);
    out[len] = 0;
}

/* Decimal "NET.NODE:SOCK" (at most 15 characters plus the terminator). */
void cnet_format_endpoint(uint32_t addr, uint32_t socket, char *out) {
    cnet_format_addr(addr, out);
    uint32_t len = 0;
    while (out[len]) ++len;
    out[len++] = ':';
    len += put_decimal(socket & 0xFFu, out + len);
    out[len] = 0;
}
