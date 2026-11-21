/* chat ADDR [socket] -- a two-window CNET chat for Foundation (libcnet).
 *
 * Every line typed is sent as one datagram (PROTO 0x80) to ADDR's socket
 * (default 77); datagrams arriving on our socket are printed above the input
 * line with the sender's address. Esc quits. Run it in two emulators, each
 * pointing at the other's node (examples/cnet-chat/run_demo.py). */
#include <foundation/libcnet.h>
#include <foundation/sys.h>

#define DEFAULT_SOCKET 77u
#define TEXT_PROTO 0x80u
#define LINE_MAX 200u

static void out(const char *text) { fnd_con_write(text); }

static void out_addr(uint32_t addr) {
    char text[10];
    cnet_format_addr(addr, text);
    out(text);
}

static int32_t fail(const char *what, int32_t st) {
    out("chat: ");
    out(what);
    out(": ");
    out(fnd_strerror(st));
    out("\n");
    return st;
}

static int32_t parse_socket(const char *text, uint32_t *out_socket) {
    uint32_t value = 0;
    if (!*text) return FND_E_BAD_ARG;
    for (; *text; ++text) {
        if (*text < '0' || *text > '9') return FND_E_BAD_ARG;
        value = value * 10u + (uint32_t)(*text - '0');
        if (value > 254u) return FND_E_BAD_ARG;
    }
    if (!value || value == FND_CNET_ECHO_SOCKET) return FND_E_BAD_ARG;
    *out_socket = value;
    return FND_OK;
}

/* Redraw the prompt and the line being typed. */
static void prompt(const char *line, uint32_t len) {
    char copy[LINE_MAX + 1u];
    for (uint32_t i = 0; i < len; ++i) copy[i] = line[i];
    copy[len] = 0;
    out("> ");
    out(copy);
}

static void show_message(const char *line, uint32_t len, const cnet_peer *from, char *data, int32_t n) {
    for (int32_t i = 0; i < n; ++i)
        if ((uint8_t)data[i] < 0x20u || (uint8_t)data[i] > 0x7Eu) data[i] = '.';
    data[n] = 0;
    out("\r\033[K<");
    out_addr(from->addr);
    out("> ");
    out(data);
    out("\n");
    prompt(line, len);
}

static int32_t chat(void) {
    uint32_t peer, socket = DEFAULT_SOCKET;
    uint32_t argc = fnd_argc();
    if (argc < 2u || argc > 3u || cnet_parse_addr(fnd_argv(1), &peer) != FND_OK ||
        (argc == 3u && parse_socket(fnd_argv(2), &socket) != FND_OK)) {
        out("usage: chat ADDR [socket]   (ADDR = NET:NODE or NODE in hex)\n");
        return FND_E_BAD_ARG;
    }
    fnd_handle h;
    int32_t st = cnet_open(socket, &h);
    if (st != FND_OK) return fail("/dev/cnet", st);
    st = cnet_wait_ready(h, 300u);
    if (st != FND_OK) {
        cnet_close(h);
        return fail("address", st);
    }
    cnet_info info;
    cnet_info_get(h, &info);
    if (!FND_CNET_NET(peer)) peer = FND_CNET_ADDR(FND_CNET_NET(info.addr), FND_CNET_NODE(peer));
    out("CNET chat: ");
    out_addr(info.addr);
    out(" -> ");
    out_addr(peer);
    out(". Enter sends, Esc quits.\n");

    char line[LINE_MAX];
    uint32_t len = 0;
    fnd_con_set_mode(FND_CON_RAW);
    prompt(line, len);
    for (;;) {
        int busy = 0;
        fnd_key_t key;
        if (fnd_con_pollkey(&key) == FND_OK) {
            busy = 1;
            if (key.code == FND_KEY_ESC) break;
            if (key.code == FND_KEY_ENTER) {
                out("\n");
                if (len) {
                    st = cnet_sendto(h, peer, socket, TEXT_PROTO, line, len);
                    if (st != FND_OK) fail("send", st);
                }
                len = 0;
                prompt(line, len);
            } else if (key.code == FND_KEY_BACKSPACE) {
                if (len) {
                    --len;
                    out("\b \b");
                }
            } else if (key.code == FND_KEY_CHAR && key.ch >= 0x20u && key.ch <= 0x7Eu && len < LINE_MAX) {
                char echo[2] = {(char)key.ch, 0};
                line[len++] = (char)key.ch;
                out(echo);
            }
        }
        char data[FND_CNET_MTU + 1u];
        cnet_peer from;
        int32_t n = cnet_recvfrom(h, data, FND_CNET_MTU, FND_CNET_NONBLOCK, &from);
        if (n >= 0) {
            busy = 1;
            show_message(line, len, &from, data, n);
        } else if (n != FND_E_WOULD_BLOCK) {
            fail("receive", n);
            break;
        }
        if (!busy) fnd_sys_sleep(1);
    }
    fnd_con_set_mode(FND_CON_LINE);
    out("\nbye\n");
    cnet_close(h);
    return FND_OK;
}

int main(void) {
    fnd_init();
    fnd_exit(chat());
}
