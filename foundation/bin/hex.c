/* ANSI console TUI hex editor; see doc/spec_Foundation_shell.md section 10.3. */
#include "cmd.h"

#define MAX_FILE 65536u
#define MAX_WORDS 16u
#define WORD_CAP 32u
#define LINE_CAP 120u

static uint8_t *data;
static uint32_t data_len, pos, dirty, quit;
static uint32_t cursor, nibble, ascii_mode, per_row, page_size, confirm_quit;
static fnd_handle console;
static char output[FND_IPC_MAX_DATA];
static uint32_t output_len;
static const char *path;
static char message[48];
static char words[MAX_WORDS][WORD_CAP];

static char upper(char c) { return c >= 'a' && c <= 'z' ? (char)(c - 32) : c; }

static int str_eq(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return *a == *b;
}

static void set_message(const char *prefix, const char *text, uint32_t number, int with_number) {
    uint32_t n = 0;
    for (; *prefix && n < sizeof message - 1u; ++prefix)
        message[n++] = *prefix;
    for (; *text && n < sizeof message - 1u; ++text)
        message[n++] = *text;
    if (with_number) {
        char digits[10];
        uint32_t count = 0;
        do {
            digits[count++] = (char)('0' + number % 10u);
            number /= 10u;
        } while (number);
        while (count && n < sizeof message - 1u)
            message[n++] = digits[--count];
        const char *tail = " BYTES";
        for (; *tail && n < sizeof message - 1u; ++tail)
            message[n++] = *tail;
    }
    message[n] = 0;
}

static void msg_err(const char *text) { set_message("ERROR: ", text, 0, 0); }

/* Keep UTF-8 glyph encodings intact at IPC boundaries. ANSI may be split. */
static void flush(void) {
    if (output_len) (void)fnd_write(console, output, output_len, 0);
    output_len = 0;
}
static void out(const char *s) {
    uint32_t len = cmd_len(s);
    if (output_len + len > FND_IPC_MAX_DATA) flush();
    for (uint32_t i = 0; i < len; ++i) output[output_len++] = s[i];
}
static void decimal(uint32_t value) {
    char text[11];
    uint32_t n = 0;
    do { text[n++] = (char)('0' + value % 10u); value /= 10u; } while (value);
    while (n) { char c[2] = {text[--n], 0}; out(c); }
}
static void at(uint32_t row, uint32_t col) {
    out("\033["); decimal(row); out(";"); decimal(col); out("H");
}
static void out_hex(uint32_t value, uint32_t digits) {
    char text[9];
    for (uint32_t i = 0; i < digits; ++i)
        text[i] = "0123456789ABCDEF"[(value >> ((digits - 1u - i) * 4u)) & 15u];
    text[digits] = 0;
    out(text);
}
static void glyph(uint8_t b) {
    /* U+E000 + glyph number, including NUL, LF, ESC and FF. */
    char text[4] = {(char)0xEE, (char)(0x80u | (b >> 6)),
                    (char)(0x80u | (b & 63u)), 0};
    out(text);
}
static void selected_color(uint32_t offset) {
    if (offset == cursor) out("\033[30;106m");
    else if (data[offset] == 0) out("\033[90;40m");
    else out("\033[97;40m");
}
static void show(void) {
    pos = cursor / page_size * page_size;
    at(1, 1); out("\033[0;96m\033[2KHEX ");
    /* Bound the title to the display, even for long/UTF-8 paths. */
    for (uint32_t i = 0; path[i] && i < per_row * 2u; ++i) {
        char c[2] = {path[i] >= 32 && path[i] <= 126 ? path[i] : '?', 0}; out(c);
    }
    at(2, 1); out("\033[2K"); decimal(data_len); out(" bytes  @"); out_hex(cursor, 4);
    out(dirty ? " * " : "   "); out(ascii_mode ? "ASCII" : "HEX");
    at(3, 1); out("\033[2K     ");
    for (uint32_t col = 0; col < per_row; ++col) { out_hex(col, 2); out(" "); }
    for (uint32_t row = 0; row < 16u; ++row) {
        uint32_t address = pos + row * per_row;
        at(4u + row, 1); out("\033[0;36m\033[2K"); out_hex(address, 4); out(" ");
        for (uint32_t col = 0; col < per_row; ++col) {
            uint32_t offset = address + col;
            if (offset < data_len) { selected_color(offset); out_hex(data[offset], 2); }
            else out("  ");
            out("\033[0m ");
        }
        out("|");
        for (uint32_t col = 0; col < per_row; ++col) {
            uint32_t offset = address + col;
            if (offset < data_len) { selected_color(offset); glyph(data[offset]); }
            else out(" ");
            out("\033[0m");
        }
        out("|");
    }
    at(21, 1); out("\033[0;96m\033[2KArrows:move Tab:HEX/ASCII Home/End");
    at(22, 1); out("\033[2KCtrl+S:save Ctrl+Q:quit Esc:HEX");
    at(23, 1); out("\033[2KHEX: N/P:page G:goto W:save Q:quit");
    at(24, 1); out("\033[2K: DOS commands (E/G/N/P/W/Q)");
    at(26, 1); out("\033[0;93m\033[2K"); out(message);
    out("\033[0m");
    at(27, 1); out("\033[2K");
    uint32_t cell = cursor - pos;
    at(4u + cell / per_row, ascii_mode ? 7u + 3u * per_row + cell % per_row :
                                         6u + 3u * (cell % per_row) + nibble);
    flush();
}

/* All of `word` as hex digits, or -1. */
static int32_t parse_hex(const char *word) {
    uint32_t value = 0, digits = 0;
    for (; *word; ++word, ++digits) {
        char c = upper(*word);
        uint32_t d;
        if (c >= '0' && c <= '9')
            d = (uint32_t)(c - '0');
        else if (c >= 'A' && c <= 'F')
            d = (uint32_t)(c - 'A') + 10u;
        else
            return -1;
        value = (value << 4) + d;
    }
    return digits && digits <= 8u && value <= 0x7FFFFFFFu ? (int32_t)value : -1;
}

static uint32_t split(const char *line, uint32_t len) {
    uint32_t count = 0, chars = 0;
    int started = 0;
    for (uint32_t i = 0; i < len; ++i) {
        char c = line[i];
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
            if (started) {
                words[count++][chars] = 0;
                chars = 0;
                started = 0;
            }
        } else if (count < MAX_WORDS && chars < WORD_CAP - 1u) {
            words[count][chars++] = upper(c);
            started = 1;
        }
    }
    if (started)
        words[count++][chars] = 0;
    return count;
}

static void save(void) {
    fnd_handle h;
    fnd_status status = fnd_file_open(path, FND_OPEN_WRITE | FND_OPEN_TRUNC, FND_MODE_DEFAULT, &h);
    if (status != FND_OK) {
        msg_err("SAVE FAILED");
        return;
    }
    for (uint32_t done = 0; done < data_len && status == FND_OK;) {
        uint32_t chunk = data_len - done, put = 0;
        if (chunk > 1024u)
            chunk = 1024u;
        status = fnd_write(h, data + done, chunk, &put);
        if (status == FND_OK && put == 0)
            status = FND_E_IO;
        done += put;
    }
    if (status == FND_OK)
        status = fnd_sync(h);
    fnd_close(h);
    if (status != FND_OK) {
        msg_err("SAVE FAILED");
        return;
    }
    set_message("SAVED ", "", data_len, 1);
    dirty = 0;
}

static void dispatch(uint32_t count) {
    const char *command = words[0];
    int32_t address, value;
    if (!count || str_eq(command, "N") || str_eq(command, "NEXT")) {
        cursor = cursor + page_size < data_len ? cursor + page_size : data_len - 1u;
    } else if (str_eq(command, "P") || str_eq(command, "PREV")) {
        cursor = cursor < page_size ? 0 : cursor - page_size;
    } else if (str_eq(command, "Q") || str_eq(command, "QUIT")) {
        if (dirty) { confirm_quit = 1; set_message("Discard changes? Y/N", "", 0, 0); }
        else quit = 1;
    } else if (str_eq(command, "W") || str_eq(command, "SAVE")) {
        save();
    } else if (str_eq(command, "G") || str_eq(command, "GOTO")) {
        if (count < 2u) {
            msg_err(command[1] ? "USAGE: GOTO ADDRESS" : "USAGE: G ADDRESS");
        } else if ((address = parse_hex(words[1])) < 0) {
            msg_err("BAD ADDRESS");
        } else if ((uint32_t)address >= data_len) {
            msg_err("ADDRESS OUT OF RANGE");
        } else {
            cursor = (uint32_t)address; nibble = 0;
        }
    } else if (str_eq(command, "E") || str_eq(command, "SET")) {
        if (count < 3u) {
            msg_err(command[1] ? "USAGE: SET ADDRESS V1 [V2 ...]" : "USAGE: E ADDRESS V1 [V2 ...]");
            return;
        }
        address = parse_hex(words[1]);
        if (address < 0) {
            msg_err("BAD ADDRESS");
            return;
        }
        for (uint32_t i = 2; i < count; ++i) {
            value = parse_hex(words[i]);
            if (value < 0) {
                msg_err("BAD VALUE");
                return;
            }
            if (value > 255) {
                msg_err("VALUE MUST BE A BYTE");
                return;
            }
            if ((uint32_t)address >= data_len) {
                msg_err("WRITE PAST END OF FILE");
                return;
            }
            data[address++] = (uint8_t)value;
            dirty = 1;
        }
    } else {
        msg_err("UNKNOWN HEX COMMAND");
    }
}

static int32_t load(void) {
    fnd_stat_t st;
    fnd_status status = fnd_path_stat(path, &st);
    if (status != FND_OK)
        return cmd_report("hex", path, status);
    if (st.type == FND_TYPE_DIR)
        return cmd_report("hex", path, FND_E_IS_DIR);
    if (st.size_hi || st.size_lo > MAX_FILE) {
        cmd_err("hex: ");
        cmd_err(path);
        cmd_err(": file too large (limit 65536 bytes)\n");
        return FND_E_LIMIT;
    }
    if (st.size_lo == 0) {
        cmd_err("hex: ");
        cmd_err(path);
        cmd_err(": empty file has nothing to edit\n");
        return FND_E_BAD_ARG;
    }
    data_len = st.size_lo;
    data = fnd_mem_alloc(data_len);
    if (!data)
        return cmd_report("hex", path, FND_E_NO_MEMORY);
    fnd_handle h = 0;
    status = fnd_file_open(path, FND_OPEN_READ, FND_MODE_DEFAULT, &h);
    for (uint32_t done = 0; status == FND_OK && done < data_len;) {
        uint32_t got = 0;
        status = fnd_read(h, data + done, data_len - done, &got);
        if (status == FND_OK && got == 0)
            status = FND_E_IO;
        done += got;
    }
    if (h) fnd_close(h);
    if (status != FND_OK)
        return cmd_report("hex", path, status);
    return FND_OK;
}

/* Commands use the same key API as byte editing; ESC cancels the prompt
 * without leaving an unfinished console LINE-mode edit buffer. */
static void command(int go) {
    char line[LINE_CAP];
    uint32_t len = 0;
    if (go) { line[len++] = 'G'; line[len++] = ' '; }
    for (;;) {
        at(27, 1); out("\033[0m\033[2K:");
        line[len] = 0; out(line); flush();
        fnd_key_t key;
        if (fnd_con_getkey(&key) != FND_OK) { quit = 1; return; }
        if (key.code == FND_KEY_ESC) return;
        if (key.code == FND_KEY_ENTER) { dispatch(split(line, len)); return; }
        if (key.code == FND_KEY_BACKSPACE && len) --len;
        if (key.code == FND_KEY_CHAR && !(key.mods & (FND_MOD_CTRL | FND_MOD_ALT)) &&
            key.ch >= 32u && key.ch <= 126u && len < (per_row == 8u ? 37u : 77u))
            line[len++] = (char)key.ch;
    }
}
static void move_to(uint32_t value) {
    cursor = value < data_len ? value : data_len - 1u;
    nibble = 0;
}
static void keypress(fnd_key_t key) {
    char ch = key.ch < 128u ? upper((char)key.ch) : 0;
    if (confirm_quit) {
        if (key.code == FND_KEY_CHAR && ch == 'Y') quit = 1;
        else if (key.code == FND_KEY_ESC || (key.code == FND_KEY_CHAR && ch == 'N')) {
            confirm_quit = 0; message[0] = 0;
        }
        return;
    }
    message[0] = 0;
    if (key.code == FND_KEY_CHAR && (key.mods & FND_MOD_CTRL)) {
        if (ch == 'S') save();
        else if (ch == 'Q') { words[0][0] = 'Q'; words[0][1] = 0; dispatch(1); }
        return;
    }
    if (key.mods & FND_MOD_ALT) return;
    switch (key.code) {
    case FND_KEY_LEFT: move_to(cursor ? cursor - 1u : 0); return;
    case FND_KEY_RIGHT: move_to(cursor + 1u); return;
    case FND_KEY_UP: move_to(cursor >= per_row ? cursor - per_row : cursor); return;
    case FND_KEY_DOWN: move_to(cursor + per_row < data_len ? cursor + per_row : cursor); return;
    case FND_KEY_HOME: move_to(key.mods & FND_MOD_CTRL ? 0 : cursor / per_row * per_row); return;
    case FND_KEY_END: move_to(key.mods & FND_MOD_CTRL ? data_len - 1u : cursor / per_row * per_row + per_row - 1u); return;
    case FND_KEY_TAB: ascii_mode ^= 1u; nibble = 0; return;
    case FND_KEY_ESC: ascii_mode = nibble = 0; return;
    default: break;
    }
    if (key.code != FND_KEY_CHAR) return;
    if (ascii_mode) {
        if (key.ch >= 32u && key.ch <= 126u) {
            if (data[cursor] != key.ch) { data[cursor] = (uint8_t)key.ch; dirty = 1; }
            move_to(cursor + 1u);
        }
        return;
    }
    char digit[2] = {(char)key.ch, 0};
    int32_t value = key.ch < 128u ? parse_hex(digit) : -1;
    if (value >= 0) {
        uint8_t b = nibble ? (uint8_t)((data[cursor] & 0xF0u) | (uint32_t)value) :
                            (uint8_t)((data[cursor] & 15u) | ((uint32_t)value << 4));
        if (b != data[cursor]) { data[cursor] = b; dirty = 1; }
        if (nibble) move_to(cursor + 1u);
        else nibble = 1;
    } else if (ch == ':' || ch == 'G') command(ch == 'G');
    else if (ch == 'W' || ch == 'Q' || ch == 'N' || ch == 'P') {
        words[0][0] = ch; words[0][1] = 0; dispatch(1); nibble = 0;
    }
}
static int32_t hex(uint32_t argc) {
    if (argc != 2u) return cmd_usage("hex path");
    path = fnd_argv(1);
    int32_t result = load();
    if (result != FND_OK) return result;
    console = fnd_console();
    uint32_t cols, rows;
    result = fnd_con_size(&cols, &rows);
    if (result != FND_OK) { fnd_mem_free(data); return cmd_report("hex", 0, result); }
    per_row = cols < 80u ? 8u : 16u;
    page_size = per_row * 16u;
    out("\033[0m\033[2J\033[H\033[?25h"); flush();
    while (!quit) {
        show();
        fnd_key_t key;
        result = fnd_con_getkey(&key);
        if (result != FND_OK) break;
        keypress(key);
    }
    at(28, 1); out("\033[0m\033[?25h\033[K");
    if (dirty) out("UNSAVED CHANGES DISCARDED\n");
    flush();
    fnd_mem_free(data);
    return result;
}

CMD_MAIN(hex)
