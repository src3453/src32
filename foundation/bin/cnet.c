/* cnet [info] | cnet ping ADDR [count] | cnet send ADDR:SOCK TEXT... | cnet recv SOCK [count]
 * CNET datagrams over libcnet (doc/spec_Foundation_API.md §9.6). ADDR is
 * NET.NODE in decimal (a trailing H makes the whole string hex), or .NODE /
 * NODE for the local network; SOCK is written after the address as
 * NET.NODE:SOCK (doc/spec_NET.md §2). */
#include <foundation/libcnet.h>
#include <foundation/sys.h>

#include "cmd.h"

#define READY_TICKS 300u
#define REPLY_TICKS 200u
#define TEXT_PROTO 0x80u

static int32_t parse_dec(const char *text, uint32_t max, uint32_t *out) {
    uint32_t value = 0;
    if (!*text) return FND_E_BAD_ARG;
    for (; *text; ++text) {
        if (*text < '0' || *text > '9') return FND_E_BAD_ARG;
        value = value * 10u + (uint32_t)(*text - '0');
        if (value > max) return FND_E_BAD_ARG;
    }
    *out = value;
    return FND_OK;
}

static int32_t parse_socket(const char *text, uint32_t *out) {
    uint32_t value = 0, digits = 0, base = 10u;
    uint32_t n = 0;
    if (*text == ':') ++text; /* ":SOCK" is the same socket on this node */
    while (text[n]) ++n;
    if (n && (text[n - 1u] == 'H' || text[n - 1u] == 'h')) { /* trailing H: hex */
        base = 16u;
        --n;
    }
    for (uint32_t i = 0; i < n; ++i, ++digits) {
        char c = text[i];
        uint32_t d;
        if (c >= '0' && c <= '9') d = (uint32_t)(c - '0');
        else if (base == 16u && c >= 'a' && c <= 'f') d = (uint32_t)(c - 'a' + 10);
        else if (base == 16u && c >= 'A' && c <= 'F') d = (uint32_t)(c - 'A' + 10);
        else return FND_E_BAD_ARG;
        if (digits == 3u) return FND_E_BAD_ARG;
        value = value * base + d;
    }
    if (!digits || !value || value > 0xFEu) return FND_E_BAD_ARG;
    *out = value;
    return FND_OK;
}

static int same(const char *a, const char *b) {
    while (*a && *a == *b) ++a, ++b;
    return *a == *b;
}

static void out_addr(uint32_t addr) {
    char text[16];
    cnet_format_addr(addr, text);
    cmd_out(text);
}

static void out_endpoint(uint32_t addr, uint32_t socket) {
    char text[16];
    cnet_format_endpoint(addr, socket, text);
    cmd_out(text);
}

static int32_t open_ready(uint32_t socket, fnd_handle *h) {
    int32_t st = cnet_open(socket, h);
    if (st != FND_OK) return cmd_report("cnet", "/dev/cnet", st);
    st = cnet_wait_ready(*h, READY_TICKS);
    if (st != FND_OK) {
        cnet_close(*h);
        return cmd_report("cnet", "address", st);
    }
    return FND_OK;
}

static int32_t info(void) {
    fnd_handle h;
    int32_t st = cnet_open(0, &h);
    if (st != FND_OK) return cmd_report("cnet", "/dev/cnet", st);
    cnet_wait_ready(h, READY_TICKS);
    cnet_info i;
    st = cnet_info_get(h, &i);
    cnet_close(h);
    if (st != FND_OK) return cmd_report("cnet", "info", st);
    cmd_out("link ");
    cmd_out((i.status & FND_CNET_LINK) ? "up" : "down");
    if (i.status & FND_CNET_READY) {
        cmd_out(", address ");
        out_addr(i.addr);
    } else if (i.status & FND_CNET_CONFLICT) {
        cmd_out(", address conflict");
    } else if (i.status & FND_CNET_PROBING) {
        cmd_out(", acquiring address");
    }
    cmd_out("\n");
    return FND_OK;
}

static int32_t ping(uint32_t argc) {
    uint32_t addr, count = 1;
    if (argc < 3u || argc > 4u || cnet_parse_addr(fnd_argv(2), &addr) != FND_OK ||
        (argc == 4u && parse_dec(fnd_argv(3), 100u, &count) != FND_OK))
        return cmd_usage("cnet ping ADDR [count]");
    fnd_handle h;
    int32_t st = open_ready(0, &h);
    if (st != FND_OK) return st;
    uint32_t replies = 0;
    for (uint32_t seq = 1; seq <= count; ++seq) {
        uint8_t request[8] = {1u, 'p', 'i', 'n', 'g', (uint8_t)(seq >> 8), (uint8_t)seq, 0};
        st = cnet_sendto(h, addr, FND_CNET_ECHO_SOCKET, FND_CNET_PROTO_ECHO, request, sizeof request);
        if (st != FND_OK) {
            cnet_close(h);
            return cmd_report("cnet", fnd_argv(2), st);
        }
        uint32_t waited = 0;
        for (;;) {
            uint8_t reply[FND_CNET_MTU];
            cnet_peer from;
            int32_t n = cnet_recvfrom(h, reply, sizeof reply, FND_CNET_NONBLOCK, &from);
            if (n == (int32_t)sizeof request && reply[0] == 2u && reply[5] == request[5] &&
                reply[6] == request[6] && from.socket == FND_CNET_ECHO_SOCKET) {
                cmd_out("reply from ");
                out_addr(from.addr);
                cmd_out(" seq=");
                cmd_out_dec((int32_t)seq, 0);
                cmd_out("\n");
                ++replies;
                break;
            }
            if (n >= 0) continue; /* something else: keep waiting */
            if (n != FND_E_WOULD_BLOCK || ++waited > REPLY_TICKS) {
                cmd_out("no reply seq=");
                cmd_out_dec((int32_t)seq, 0);
                cmd_out("\n");
                break;
            }
            fnd_sys_sleep(1);
        }
    }
    cnet_close(h);
    cmd_out_dec((int32_t)replies, 0);
    cmd_out("/");
    cmd_out_dec((int32_t)count, 0);
    cmd_out(" replies\n");
    return replies == count ? FND_OK : FND_E_TIMEOUT;
}

static int32_t send(uint32_t argc) {
    uint32_t addr, socket = 0;
    if (argc < 4u || cnet_parse_endpoint(fnd_argv(2), &addr, &socket) != FND_OK || !socket)
        return cmd_usage("cnet send ADDR:SOCK TEXT...");
    char text[FND_CNET_MTU];
    uint32_t len = 0;
    for (uint32_t i = 3; i < argc; ++i) {
        const char *word = fnd_argv(i);
        uint32_t n = cmd_len(word);
        if (len + n + (i > 3u) > sizeof text) return cmd_report("cnet", "text", FND_E_LIMIT);
        if (i > 3u) text[len++] = ' ';
        for (uint32_t j = 0; j < n; ++j) text[len++] = word[j];
    }
    fnd_handle h;
    int32_t st = open_ready(0, &h);
    if (st != FND_OK) return st;
    st = cnet_sendto(h, addr, socket, TEXT_PROTO, text, len);
    cnet_close(h);
    return st == FND_OK ? FND_OK : cmd_report("cnet", fnd_argv(2), st);
}

static int32_t recv(uint32_t argc) {
    uint32_t socket = 0, count = 1;
    if (argc < 3u || argc > 4u || parse_socket(fnd_argv(2), &socket) != FND_OK ||
        (argc == 4u && parse_dec(fnd_argv(3), 1000u, &count) != FND_OK))
        return cmd_usage("cnet recv SOCK [count]");
    fnd_handle h;
    int32_t st = open_ready(socket, &h);
    if (st != FND_OK) return st;
    for (uint32_t i = 0; i < count; ++i) {
        char data[FND_CNET_MTU + 1u];
        cnet_peer from;
        int32_t n = cnet_recvfrom(h, data, FND_CNET_MTU, 0, &from);
        if (n < 0) {
            cnet_close(h);
            return cmd_report("cnet", "recv", n);
        }
        data[n] = 0;
        for (int32_t j = 0; j < n; ++j)
            if ((uint8_t)data[j] < 0x20u || (uint8_t)data[j] > 0x7Eu) data[j] = '.';
        cmd_out("from ");
        out_endpoint(from.addr, from.socket);
        cmd_out(": ");
        cmd_out(data);
        cmd_out("\n");
    }
    cnet_close(h);
    return FND_OK;
}

static int32_t cnet(uint32_t argc) {
    if (argc == 1u) return info();
    const char *verb = fnd_argv(1);
    if (same("info", verb) && argc == 2u) return info();
    if (same("ping", verb)) return ping(argc);
    if (same("send", verb)) return send(argc);
    if (same("recv", verb)) return recv(argc);
    return cmd_usage("cnet [info] | ping ADDR [count] | send ADDR:SOCK TEXT... | recv SOCK [count]");
}

CMD_MAIN(cnet)
