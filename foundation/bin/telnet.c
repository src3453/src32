/* telnet [-b ADDR[:SOCK]] [-t TERM] HOST [PORT]
 * ssh [-b ADDR[:SOCK]] [-t TERM] [-p PORT] [USER@]HOST   (ssh.c includes this file)
 *
 * A telnet client over the CNET TCP bridge (doc/spec_NET_bridge.md). The
 * bridge, a host program at CNET address ADDR:SOCK (-b, default .FF01:17,
 * i.e. socket 23), opens telnet://HOST:PORT or ssh://USER@HOST:PORT for us:
 * we open an L3P stream (WAL l3p, doc/spec_NET_L3P.md §6), send the target
 * as the first line and wait for the bridge's "OK" line; this side always
 * speaks telnet. Ctrl+C belongs to the shell (it ends this program), so the
 * escape key Ctrl+] followed by c sends Ctrl+C and q closes the connection. */
#include <foundation/l3p.h>
#include <foundation/sys.h>

#include "cmd.h"

#ifndef TELNET_SSH
#define TELNET_SSH 0
#endif
#if TELNET_SSH
#define NAME "ssh"
#define USAGE "ssh [-b ADDR[:SOCK]] [-t TERM] [-p PORT] [USER@]HOST"
#else
#define NAME "telnet"
#define USAGE "telnet [-b ADDR[:SOCK]] [-t TERM] HOST [PORT]"
#endif

#define DEFAULT_BRIDGE ".FF01"
#define BRIDGE_SOCKET 23u
#define MAX_DATA L3P_MAX_PLAIN
#define ANSWER_TICKS 3000u
#define CLOSE_TICKS 500u

#define IAC 255u
#define DONT 254u
#define DO 253u
#define WONT 252u
#define WILL 251u
#define SB 250u
#define SE 240u
#define OPT_BINARY 0u
#define OPT_ECHO 1u
#define OPT_SGA 3u
#define OPT_TTYPE 24u
#define OPT_NAWS 31u

static l3p_assoc assoc;
static uint32_t bridge, bridge_socket = BRIDGE_SOCKET;
static const char *term = "ansi";

static uint32_t now(void) { return (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO); }

static int same(const char *a, const char *b) {
    while (*a && *a == *b) ++a, ++b;
    return *a == *b;
}

static uint32_t send_space(void) {
    l3p_info_t info;
    l3p_info(&assoc, &info);
    return info.send_space;
}

/* Queues bytes for the peer; 0 if they do not fit (nothing is queued). */
static int stream_write(const void *data, uint32_t len) {
    uint32_t put;
    if (send_space() < len) return 0;
    return l3p_write(&assoc, data, len, &put) == FND_OK && put == len;
}

/* ---- telnet (RFC 854; NAWS 1073, TTYPE 1091) ---- */

static uint8_t remote_on[256 / 8], local_on[256 / 8];
static uint32_t tn_state, tn_verb, sb_len;
static uint8_t sb[64];
static uint8_t out[2 * MAX_DATA + 8];
static uint32_t out_len;

static int bit(const uint8_t *set, uint32_t opt) { return set[opt >> 3] >> (opt & 7u) & 1u; }
static void set_bit(uint8_t *set, uint32_t opt, int on) {
    if (on) set[opt >> 3] |= (uint8_t)(1u << (opt & 7u));
    else set[opt >> 3] &= (uint8_t)~(1u << (opt & 7u));
}

static void reply3(uint32_t verb, uint32_t opt) {
    uint8_t r[3] = {IAC, (uint8_t)verb, (uint8_t)opt};
    stream_write(r, 3);
}

static void send_naws(void) {
    uint32_t cols = 80, rows = 30;
    fnd_con_size(&cols, &rows);
    uint8_t r[9] = {IAC, SB, OPT_NAWS, (uint8_t)(cols >> 8), (uint8_t)cols, (uint8_t)(rows >> 8), (uint8_t)rows,
                    IAC, SE};
    stream_write(r, 9);
}

static void send_ttype(void) {
    uint8_t r[4 + 40 + 2] = {IAC, SB, OPT_TTYPE, 0};
    uint32_t n = 4;
    for (const char *p = term; *p && n < 44u; ++p) {
        char c = *p;
        r[n++] = (uint8_t)(c >= 'a' && c <= 'z' ? c - 32 : c); /* names are sent in upper case */
    }
    r[n++] = IAC;
    r[n++] = SE;
    stream_write(r, n);
}

static void option(uint32_t verb, uint32_t opt) {
    if (verb == WILL) {
        if (opt == OPT_ECHO || opt == OPT_SGA || opt == OPT_BINARY) {
            if (!bit(remote_on, opt)) {
                set_bit(remote_on, opt, 1);
                reply3(DO, opt);
            }
        } else {
            reply3(DONT, opt);
        }
    } else if (verb == WONT) {
        if (bit(remote_on, opt)) {
            set_bit(remote_on, opt, 0);
            reply3(DONT, opt);
        }
    } else if (verb == DO) {
        if (opt == OPT_NAWS || opt == OPT_TTYPE || opt == OPT_BINARY) {
            if (!bit(local_on, opt)) {
                set_bit(local_on, opt, 1);
                reply3(WILL, opt);
                if (opt == OPT_NAWS) send_naws();
            }
        } else {
            reply3(WONT, opt);
        }
    } else if (bit(local_on, opt)) { /* DONT */
        set_bit(local_on, opt, 0);
        reply3(WONT, opt);
    }
}

/* Writes the complete UTF-8 characters in out[] to the screen and keeps
 * a split one for the next datagram (the console wants whole characters). */
static void flush_out(void) {
    uint32_t keep = 0;
    for (uint32_t back = 1; back <= 3u && back <= out_len; ++back) {
        uint8_t c = out[out_len - back];
        if ((c & 0xC0u) == 0x80u) continue;
        uint32_t need = c < 0xC0u ? 1u : c < 0xE0u ? 2u : c < 0xF0u ? 3u : 4u;
        if (need > back) keep = back;
        break;
    }
    uint32_t n = out_len - keep, put;
    if (n) fnd_write(fnd_std_out(), out, n, &put);
    for (uint32_t i = 0; i < keep; ++i) out[i] = out[n + i];
    out_len = keep;
}

static void telnet_input(const uint8_t *data, uint32_t len) {
    for (uint32_t i = 0; i < len; ++i) {
        uint8_t c = data[i];
        switch (tn_state) {
        case 0:
            if (c == IAC) tn_state = 1;
            else if (c) out[out_len++] = c;
            break;
        case 1:
            if (c == IAC) {
                out[out_len++] = c;
                tn_state = 0;
            } else if (c >= WILL && c <= DONT) {
                tn_verb = c;
                tn_state = 2;
            } else if (c == SB) {
                sb_len = 0;
                tn_state = 3;
            } else {
                tn_state = 0;
            }
            break;
        case 2:
            option(tn_verb, c);
            tn_state = 0;
            break;
        case 3:
            if (c == IAC) tn_state = 4;
            else if (sb_len < sizeof sb) sb[sb_len++] = c;
            break;
        default: /* SB IAC */
            if (c == SE) {
                if (sb_len >= 2 && sb[0] == OPT_TTYPE && sb[1] == 1u && bit(local_on, OPT_TTYPE)) send_ttype();
                tn_state = 0;
            } else {
                if (c == IAC && sb_len < sizeof sb) sb[sb_len++] = c;
                tn_state = 3;
            }
        }
    }
    flush_out();
}

/* ---- data from the bridge ---- */

static int peer_eof;

/* Runs the association and hands what arrived to telnet; 1 if there was data. */
static int receive(void) {
    static uint8_t buf[MAX_DATA];
    int any = 0;
    l3p_poll(&assoc);
    for (;;) {
        uint32_t got;
        int32_t st = l3p_read(&assoc, buf, sizeof buf, &got);
        if (st == FND_END) peer_eof = 1;
        if (st != FND_OK) break;
        telnet_input(buf, got);
        any = 1;
    }
    return any;
}

/* ---- keys ---- */

static int escape_pending;

static void local_echo(const char *text) {
    if (!bit(remote_on, OPT_ECHO)) fnd_con_write(text);
}

/* Turns one key into bytes for the peer; 0 to quit. */
static int key_input(const fnd_key_t *k) {
    static const char *const special[] = {
        [FND_KEY_DELETE] = "\033[3~", [FND_KEY_LEFT] = "\033[D", [FND_KEY_RIGHT] = "\033[C",
        [FND_KEY_UP] = "\033[A",      [FND_KEY_DOWN] = "\033[B", [FND_KEY_HOME] = "\033[H",
        [FND_KEY_END] = "\033[F",     [FND_KEY_TAB] = "\t",      [FND_KEY_ESC] = "\033",
    };
    uint8_t b[4];
    uint32_t n = 0;
    int ctrl = (k->mods & FND_MOD_CTRL) != 0;
    if (escape_pending) {
        escape_pending = 0;
        if (k->code != FND_KEY_CHAR) return 1;
        if (k->ch == 'q' || k->ch == 'Q') return 0;
        if (k->ch == 'c' || k->ch == 'C') b[n++] = 3u;
        else if (k->ch == ']') b[n++] = 0x1Du;
        else return 1;
    } else if (k->code == FND_KEY_ENTER) {
        b[n++] = '\r';
        if (!bit(local_on, OPT_BINARY)) b[n++] = 0; /* NVT: CR NUL */
        local_echo("\n");
    } else if (k->code == FND_KEY_BACKSPACE) {
        b[n++] = 0x7Fu;
        local_echo("\b \b");
    } else if (k->code == FND_KEY_CHAR && ctrl) {
        uint32_t ch = k->ch;
        if (ch == ']') {
            escape_pending = 1;
            return 1;
        }
        if (ch >= 'a' && ch <= 'z') b[n++] = (uint8_t)(ch - 'a' + 1u);
        else if (ch >= '@' && ch <= '_') b[n++] = (uint8_t)(ch - '@');
        else if (ch == ' ') b[n++] = 0;
        else return 1;
    } else if (k->code == FND_KEY_CHAR) {
        uint32_t ch = k->ch;
        if (ch < 0x80u) {
            b[n++] = (uint8_t)ch;
        } else if (ch < 0x800u) {
            b[n++] = (uint8_t)(0xC0u | ch >> 6);
            b[n++] = (uint8_t)(0x80u | (ch & 0x3Fu));
        } else if (ch < 0x10000u) {
            b[n++] = (uint8_t)(0xE0u | ch >> 12);
            b[n++] = (uint8_t)(0x80u | (ch >> 6 & 0x3Fu));
            b[n++] = (uint8_t)(0x80u | (ch & 0x3Fu));
        } else {
            b[n++] = (uint8_t)(0xF0u | ch >> 18);
            b[n++] = (uint8_t)(0x80u | (ch >> 12 & 0x3Fu));
            b[n++] = (uint8_t)(0x80u | (ch >> 6 & 0x3Fu));
            b[n++] = (uint8_t)(0x80u | (ch & 0x3Fu));
        }
        if (!bit(remote_on, OPT_ECHO) && n == 1u && ch >= 0x20u) {
            char echo[2] = {(char)ch, 0};
            fnd_con_write(echo);
        }
    } else if (k->code < sizeof special / sizeof special[0] && special[k->code]) {
        const char *s = special[k->code];
        return stream_write(s, cmd_len(s)), 1;
    } else {
        return 1;
    }
    stream_write(b, n);
    return 1;
}

/* ---- the command ---- */

static uint32_t append(char *dst, uint32_t at, uint32_t cap, const char *src) {
    while (*src && at + 1u < cap) dst[at++] = *src++;
    dst[at] = 0;
    return at;
}

static int32_t parse_port(const char *text, uint32_t *port) {
    uint32_t value = 0;
    if (!*text) return FND_E_BAD_ARG;
    for (; *text; ++text) {
        if (*text < '0' || *text > '9') return FND_E_BAD_ARG;
        value = value * 10u + (uint32_t)(*text - '0');
        if (value > 65535u) return FND_E_BAD_ARG;
    }
    if (!value) return FND_E_BAD_ARG;
    *port = value;
    return FND_OK;
}

/* Builds "telnet://HOST:PORT" or "ssh://[USER@]HOST:PORT" from the arguments. */
static int32_t parse_args(uint32_t argc, char *target, uint32_t cap, const char **bridge_text) {
    const char *host = 0, *port_text = 0;
    uint32_t port = TELNET_SSH ? 22u : 23u;
    *bridge_text = DEFAULT_BRIDGE;
    for (uint32_t i = 1; i < argc; ++i) {
        const char *a = fnd_argv(i);
        if ((same(a, "-b") || same(a, "-t") || (TELNET_SSH && same(a, "-p"))) && i + 1u < argc) {
            const char *v = fnd_argv(++i);
            if (a[1] == 'b') *bridge_text = v;
            else if (a[1] == 't') term = v;
            else port_text = v;
        } else if (a[0] == '-') {
            return FND_E_BAD_ARG;
        } else if (!host) {
            host = a;
        } else if (!TELNET_SSH && !port_text) {
            port_text = a;
        } else {
            return FND_E_BAD_ARG;
        }
    }
    if (!host || !*host || (port_text && parse_port(port_text, &port) != FND_OK)) return FND_E_BAD_ARG;
    const char *at_sign = 0;
    for (const char *p = host; *p; ++p) {
        if (*p == '@') at_sign = p;
        if (*p == ' ' || *p == '/') return FND_E_BAD_ARG;
    }
    if (at_sign && (!TELNET_SSH || at_sign == host || !at_sign[1])) return FND_E_BAD_ARG;
    uint32_t colons = 0;
    for (const char *p = at_sign ? at_sign + 1 : host; *p; ++p) colons += *p == ':';
    uint32_t n = append(target, 0, cap, TELNET_SSH ? "ssh://" : "telnet://");
    if (colons) { /* IPv6: [host] */
        char user[64];
        uint32_t u = 0;
        if (at_sign)
            for (const char *p = host; p <= at_sign && u + 1u < sizeof user; ++p) user[u++] = *p;
        user[u] = 0;
        n = append(target, n, cap, user);
        n = append(target, n, cap, "[");
        n = append(target, n, cap, at_sign ? at_sign + 1 : host);
        n = append(target, n, cap, "]");
    } else {
        n = append(target, n, cap, host);
    }
    char digits[8];
    uint32_t d = sizeof digits;
    digits[--d] = 0;
    do digits[--d] = (char)('0' + port % 10u); while (port /= 10u);
    n = append(target, n, cap, ":");
    n = append(target, n, cap, digits + d);
    return n + 1u < cap ? FND_OK : FND_E_BAD_ARG;
}

static int32_t fail(const char *what, int32_t code) { return cmd_report(NAME, what, code); }

static void print_reset(const char *lead) {
    l3p_info_t info;
    l3p_info(&assoc, &info);
    cmd_err(lead);
    cmd_err(NAME ": ");
    cmd_err(info.reset_message[0] ? info.reset_message : "connection refused");
    cmd_err("\n");
}

/* Waits for the bridge's answer line; leftover bytes go to telnet. */
static int32_t wait_answer(void) {
    static uint8_t line[MAX_DATA + 8];
    uint32_t len = 0, start = now();
    for (;;) {
        int32_t st = l3p_poll(&assoc);
        if (st != FND_OK) return st;
        uint32_t got;
        st = l3p_read(&assoc, line + len, sizeof line - 8u - len, &got);
        if (st == FND_END) return FND_E_IO;
        if (st == FND_OK) {
            len += got;
            for (uint32_t i = 0; i < len; ++i) {
                if (line[i] != '\n') continue;
                if (i != 2u || line[0] != 'O' || line[1] != 'K') return FND_E_FORMAT;
                cmd_out("Connected. Escape character is Ctrl+] (then q: quit, c: send Ctrl+C).\n");
                if (len > i + 1u) telnet_input(line + i + 1u, len - i - 1u);
                return FND_OK;
            }
            if (len >= sizeof line - 8u) return FND_E_FORMAT;
            continue;
        }
        if (now() - start > ANSWER_TICKS) return FND_E_TIMEOUT;
        fnd_sys_sleep(1);
    }
}

static int32_t telnet(uint32_t argc) {
    char target[256];
    const char *bridge_text;
    if (parse_args(argc, target, sizeof target, &bridge_text) != FND_OK) return cmd_usage(USAGE);
    if (cnet_parse_endpoint(bridge_text, &bridge, &bridge_socket) != FND_OK) return fail(bridge_text, FND_E_BAD_ARG);
    char addr_text[14];
    cnet_format_endpoint(bridge, bridge_socket, addr_text);
    cmd_out("Trying ");
    cmd_out(target);
    cmd_out(" via ");
    cmd_out(addr_text);
    cmd_out("...\n");

    /* §3: an L3P stream, the target line, then the bridge's OK. */
    int32_t st = l3p_open(bridge, bridge_socket, L3P_KIND_STREAM, 0, &assoc);
    if (st == FND_E_IO) {
        print_reset("");
        return FND_E_IO;
    }
    if (st != FND_OK) return fail(addr_text, st);
    uint32_t n = cmd_len(target);
    target[n] = '\n';
    stream_write(target, n + 1u);
    target[n] = 0;
    st = wait_answer();
    if (st != FND_OK) {
        if (st == FND_E_IO) print_reset("");
        else fail(addr_text, st);
        l3p_abort(&assoc);
        return st == FND_E_IO ? FND_E_IO : st;
    }
    fnd_con_set_mode(FND_CON_RAW);

    int quit = 0, shut = 0;
    uint32_t closing_since = 0;
    l3p_info_t info;
    for (;;) {
        int busy = receive();
        l3p_info(&assoc, &info);
        if (info.state != L3P_OPEN) break;
        fnd_key_t key;
        while (!quit && !shut && send_space() >= 16u && fnd_con_pollkey(&key) == FND_OK) {
            busy = 1;
            if (!key_input(&key)) quit = 1;
        }
        if ((quit || peer_eof) && !shut) {
            l3p_shutdown(&assoc);
            shut = 1;
            closing_since = now();
        }
        if (shut) {
            l3p_info(&assoc, &info);
            if ((info.sent_all && (peer_eof || quit)) || now() - closing_since > CLOSE_TICKS) break;
        }
        if (!busy) fnd_sys_sleep(1);
    }
    fnd_con_set_mode(FND_CON_LINE);
    fnd_con_write("\033[0m\033[?25h");
    l3p_info(&assoc, &info);
    uint32_t state = info.state;
    if (state == L3P_OPEN) l3p_close(&assoc);
    else l3p_abort(&assoc);
    if (state == L3P_BROKEN && info.reset_reason) {
        print_reset("\n");
        return FND_E_IO;
    }
    if (state == L3P_BROKEN) {
        cmd_err("\n" NAME ": connection lost\n");
        return FND_E_TIMEOUT;
    }
    cmd_out(peer_eof ? "\nConnection closed by foreign host.\n" : "\nConnection closed.\n");
    return FND_OK;
}

CMD_MAIN(telnet)
