/* libcnet, the WAL "cnet" 1.0 (foundation/include/foundation/libcnet.h). */
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

static int32_t hex_value(const char *text, uint32_t *out) {
    uint32_t value = 0, digits = 0;
    for (; *text; ++text, ++digits) {
        char c = *text;
        uint32_t d;
        if (c >= '0' && c <= '9') d = (uint32_t)(c - '0');
        else if (c >= 'a' && c <= 'f') d = (uint32_t)(c - 'a' + 10);
        else if (c >= 'A' && c <= 'F') d = (uint32_t)(c - 'A' + 10);
        else return FND_E_BAD_ARG;
        if (digits == 4u) return FND_E_BAD_ARG;
        value = value << 4 | d;
    }
    if (!digits) return FND_E_BAD_ARG;
    *out = value;
    return FND_OK;
}

int32_t cnet_parse_addr(const char *text, uint32_t *addr) {
    if (!text || !addr) return FND_E_BAD_ARG;
    char net[5];
    uint32_t i = 0;
    const char *colon = text;
    while (*colon && *colon != ':') ++colon;
    uint32_t n = 0, node;
    if (*colon == ':' && colon == text) {
        /* ":NODE" is this network, the same as "NODE". */
        if (hex_value(colon + 1, &node) != FND_OK) return FND_E_BAD_ARG;
    } else if (*colon) {
        if (colon - text > 4) return FND_E_BAD_ARG;
        for (i = 0; text + i < colon; ++i) net[i] = text[i];
        net[i] = 0;
        if (hex_value(net, &n) != FND_OK || hex_value(colon + 1, &node) != FND_OK) return FND_E_BAD_ARG;
    } else if (hex_value(text, &node) != FND_OK) {
        return FND_E_BAD_ARG;
    }
    *addr = FND_CNET_ADDR(n, node);
    return FND_OK;
}

void cnet_format_addr(uint32_t addr, char *out) {
    static const char digits[] = "0123456789ABCDEF";
    for (uint32_t i = 0; i < 8u; ++i) {
        uint32_t nibble = (addr >> (28u - 4u * i)) & 0xFu;
        out[i < 4u ? i : i + 1u] = digits[nibble];
    }
    out[4] = ':';
    out[9] = 0;
}
