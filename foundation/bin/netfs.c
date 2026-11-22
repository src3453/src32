/* netfs                                   list the mounts
 * netfs exports [ADDR]                    list a server's exports (broadcast without ADDR)
 * netfs mount ADDR EXPORT NAME [-r] [-e]  mount EXPORT of the CFP server at ADDR on /net/NAME
 * netfs umount NAME                       remove /net/NAME
 *
 * The command of the CNET Filing Protocol client (doc/spec_NET_FP.md
 * §11.4). Mounts belong to the netfs service (services/netfs, a boot
 * module); this asks it with FND_NET_MOUNT, FND_NET_UMOUNT and
 * FND_NET_LIST. "exports" asks the servers itself with an L3P UREQ (§5). */
#include <foundation/l3p.h>
#include <foundation/sys.h>

#include "cmd.h"

#define CFP_SOCKET 9u
#define T_EXPORTS 0x02u
#define MOUNTS 8u
#define EXPORTS_TICKS 100u

static uint32_t rd16(const uint8_t *p) { return (uint32_t)p[0] << 8 | p[1]; }
static void copy(void *dst, const void *src, uint32_t n) {
    uint8_t *d = (uint8_t *)dst;
    const uint8_t *s = (const uint8_t *)src;
    while (n--) *d++ = *s++;
}
static int same(const char *a, const char *b) {
    while (*a && *a == *b) ++a, ++b;
    return *a == *b;
}

/* Mount and export names: ASCII lower case, digits, '_' and '-' (§5). */
static int good_name(const uint8_t *s, uint32_t n) {
    if (n == 0 || n > FND_NET_NAME_MAX) return 0;
    for (uint32_t i = 0; i < n; ++i) {
        uint8_t c = s[i];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-')) return 0;
    }
    return 1;
}

#define USAGE "netfs [exports [ADDR] | mount ADDR EXPORT NAME [-r] [-e] | umount NAME]"

static int32_t find_service(fnd_handle *out) {
    int32_t h = fnd_sys_svc_lookup("netfs");
    if (h < 0) return h;
    *out = (fnd_handle)h;
    return FND_OK;
}

static void out_padded(const char *s, uint32_t width) {
    cmd_out(s);
    for (uint32_t n = cmd_len(s); n < width; ++n) cmd_out(" ");
}

static int32_t cmd_list(void) {
    fnd_handle svc;
    if (find_service(&svc) != FND_OK) return FND_OK; /* nothing mounted */
    static uint8_t buf[MOUNTS * FND_NET_LIST_RECORD];
    fnd_msg rep;
    fnd_status st = fnd_call(svc, FND_NET_LIST, 0, 0, 0, buf, sizeof buf, &rep);
    fnd_sys_handle_close(svc);
    if (st != FND_OK) return cmd_report("netfs", 0, st);
    for (uint32_t i = 0; i < rep.a0 && (i + 1u) * FND_NET_LIST_RECORD <= rep.len; ++i) {
        const uint8_t *r = buf + i * FND_NET_LIST_RECORD;
        uint32_t w[3];
        copy(w, r, 12);
        char addr[10], name[33], export_name[33];
        copy(name, r + 12, 32);
        copy(export_name, r + 44, 32);
        name[32] = export_name[32] = 0;
        cnet_format_addr(w[0], addr);
        cmd_out("/net/");
        out_padded(name, 12);
        out_padded(addr, 11);
        out_padded(export_name, 12);
        cmd_out(w[2] ? "up" : "down");
        if (w[1] & FND_NET_MOUNT_RO) cmd_out(" ro");
        cmd_out("\n");
    }
    return FND_OK;
}

static int32_t cmd_exports(uint32_t argc) {
    uint32_t addr = 0x0000FFFFu; /* broadcast on this network */
    const char *target = 0;
    if (argc > 3u) return cmd_usage(USAGE);
    if (argc == 3u) {
        target = fnd_argv(2);
        if (cnet_parse_addr(target, &addr) != FND_OK) return cmd_report("netfs", target, FND_E_BAD_ARG);
    }
    static uint8_t out[2048] __attribute__((aligned(4)));
    uint8_t req[4] = {T_EXPORTS, 0, 0, 0};
    uint32_t count = 0;
    fnd_status st = l3p_ucall(addr, CFP_SOCKET, req, 4, out, sizeof out, EXPORTS_TICKS, &count);
    if (st != FND_OK) return cmd_report("netfs", target, st);
    uint32_t at = 0, shown = 0;
    for (uint32_t i = 0; i < count; ++i) {
        uint32_t w[3];
        copy(w, out + at, 12);
        const uint8_t *m = out + at + 12;
        uint32_t n = w[2];
        at += 12u + ((n + 3u) & ~3u);
        if (n < 8u || m[0] != (T_EXPORTS | 0x80u)) continue;
        char from[10];
        cnet_format_addr(w[0], from);
        uint32_t entries = rd16(m + 4), off = 8;
        for (uint32_t e = 0; e < entries && off + 4u <= n; ++e) {
            uint32_t flags = rd16(m + off), nl = rd16(m + off + 2);
            if (off + 4u + nl > n || nl > FND_NET_NAME_MAX) break;
            char name[FND_NET_NAME_MAX + 1];
            copy(name, m + off + 4, nl);
            name[nl] = 0;
            off += (4u + nl + 3u) & ~3u;
            out_padded(from, 11);
            cmd_out(name);
            if (flags & 1u) cmd_out(" ro");
            if (flags & 2u) cmd_out(" enc");
            cmd_out("\n");
            ++shown;
        }
    }
    if (!shown) cmd_out(target ? "netfs: no exports\n" : "netfs: no servers answered\n");
    return FND_OK;
}

static int32_t cmd_mount(uint32_t argc) {
    const char *pos[3] = {0, 0, 0};
    uint32_t npos = 0, flags = 0;
    for (uint32_t i = 2; i < argc; ++i) {
        const char *a = fnd_argv(i);
        if (same(a, "-r")) flags |= FND_NET_MOUNT_RO;
        else if (same(a, "-e")) flags |= FND_NET_MOUNT_ENC;
        else if (a[0] == '-' || npos == 3u) return cmd_usage(USAGE);
        else pos[npos++] = a;
    }
    if (npos != 3u) return cmd_usage(USAGE);
    uint32_t addr;
    if (cnet_parse_addr(pos[0], &addr) != FND_OK) return cmd_report("netfs", pos[0], FND_E_BAD_ARG);
    uint32_t el = cmd_len(pos[1]), nl = cmd_len(pos[2]);
    if (!good_name((const uint8_t *)pos[1], el)) return cmd_report("netfs", pos[1], FND_E_BAD_ARG);
    if (!good_name((const uint8_t *)pos[2], nl)) return cmd_report("netfs", pos[2], FND_E_BAD_ARG);
    uint8_t data[2u * FND_NET_NAME_MAX];
    copy(data, pos[1], el);
    copy(data + el, pos[2], nl);
    fnd_handle svc;
    fnd_status st = find_service(&svc);
    if (st != FND_OK) return cmd_report("netfs", "service", st);
    uint32_t args[4] = {addr, flags, el, 0};
    st = fnd_call(svc, FND_NET_MOUNT, args, data, el + nl, 0, 0, 0);
    fnd_sys_handle_close(svc);
    if (st != FND_OK) return cmd_report("netfs", pos[2], st);
    char text[10];
    cnet_format_addr(addr, text);
    cmd_out("/net/");
    cmd_out(pos[2]);
    cmd_out(": ");
    cmd_out(pos[1]);
    cmd_out(" on ");
    cmd_out(text);
    cmd_out(flags & FND_NET_MOUNT_RO ? " (read-only)\n" : "\n");
    return FND_OK;
}

static int32_t cmd_umount(uint32_t argc) {
    if (argc != 3u) return cmd_usage(USAGE);
    const char *name = fnd_argv(2);
    fnd_handle svc;
    if (find_service(&svc) != FND_OK) return cmd_report("netfs", name, FND_E_NOT_FOUND);
    fnd_status st = fnd_call(svc, FND_NET_UMOUNT, 0, name, cmd_len(name), 0, 0, 0);
    fnd_sys_handle_close(svc);
    return st != FND_OK ? cmd_report("netfs", name, st) : FND_OK;
}

static int32_t netfs_main(uint32_t argc) {
    if (argc == 1u) return cmd_list();
    const char *verb = fnd_argv(1);
    if (same(verb, "exports")) return cmd_exports(argc);
    if (same(verb, "mount")) return cmd_mount(argc);
    if (same(verb, "umount")) return cmd_umount(argc);
    return cmd_usage(USAGE);
}

CMD_MAIN(netfs_main)
