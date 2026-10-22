/* hex path (doc/spec_Foundation_shell.md §10.3). The interactive hex editor
 * of the DOS guest (dos.sol, examples/dos-c): 16 rows of 16 bytes with an
 * ASCII column, and the commands N/P/G/E/W/Q read from STDIN a line at a
 * time (the console's LINE mode). Command words and hex digits are case
 * insensitive and the messages keep the DOS wording. The whole file is held
 * in memory; W writes it back with its size unchanged. */
#include "cmd.h"

#define MAX_FILE 65536u
#define MAX_WORDS 16u
#define WORD_CAP 32u
#define LINE_CAP 120u

static uint8_t *data;
static uint32_t data_len, pos, dirty, quit;
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

static void out_hex(uint32_t value, uint32_t digits) {
    char text[9];
    for (uint32_t i = 0; i < digits; ++i)
        text[i] = "0123456789ABCDEF"[(value >> ((digits - 1u - i) * 4u)) & 15u];
    text[digits] = 0;
    cmd_out(text);
}

static void show(void) {
    (void)fnd_con_clear();
    cmd_out("HEX EDITOR  ");
    for (const char *p = path; *p; ++p) {
        char c[2] = {upper(*p), 0};
        cmd_out(c);
    }
    cmd_out("  ");
    cmd_out_dec((int32_t)data_len, 0);
    cmd_out(" BYTES  OFFSET ");
    out_hex(pos, 4);
    cmd_out("\nN=next P=prev G addr E addr v.. W=save Q=quit\n");
    if (message[0]) {
        cmd_out(message);
        cmd_out("\n");
        message[0] = 0;
    }
    cmd_out("      00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F\n");
    for (uint32_t row = 0; row < 16u; ++row) {
        uint32_t address = pos + row * 16u;
        char ascii[19];
        out_hex(address, 4);
        cmd_out(": ");
        ascii[0] = '|';
        for (uint32_t col = 0; col < 16u; ++col) {
            uint32_t offset = address + col;
            if (offset < data_len) {
                uint8_t b = data[offset];
                out_hex(b, 2);
                ascii[1u + col] = b >= 32u && b <= 126u ? (char)b : '.';
            } else {
                cmd_out("  ");
                ascii[1u + col] = ' ';
            }
            cmd_out(" ");
        }
        ascii[17] = '|';
        ascii[18] = 0;
        cmd_out(ascii);
        cmd_out("\n");
    }
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
    return digits ? (int32_t)value : -1;
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
        pos += 256u;
    } else if (str_eq(command, "P") || str_eq(command, "PREV")) {
        pos = pos < 256u ? 0 : pos - 256u;
    } else if (str_eq(command, "Q") || str_eq(command, "QUIT")) {
        quit = 1;
    } else if (str_eq(command, "W") || str_eq(command, "SAVE")) {
        save();
    } else if (str_eq(command, "G") || str_eq(command, "GOTO")) {
        if (count < 2u) {
            msg_err(command[1] ? "USAGE: GOTO ADDRESS" : "USAGE: G ADDRESS");
        } else if ((address = parse_hex(words[1])) < 0) {
            msg_err("BAD ADDRESS");
        } else if ((uint32_t)address > data_len) {
            msg_err("ADDRESS OUT OF RANGE");
        } else {
            pos = (uint32_t)address & ~255u;
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
    fnd_handle h;
    status = fnd_file_open(path, FND_OPEN_READ, FND_MODE_DEFAULT, &h);
    for (uint32_t done = 0; status == FND_OK && done < data_len;) {
        uint32_t got = 0;
        status = fnd_read(h, data + done, data_len - done, &got);
        if (status == FND_OK && got == 0)
            status = FND_E_IO;
        done += got;
    }
    if (status != FND_OK)
        return cmd_report("hex", path, status);
    fnd_close(h);
    return FND_OK;
}

static int32_t hex(uint32_t argc) {
    if (argc != 2u)
        return cmd_usage("hex path");
    path = fnd_argv(1);
    int32_t result = load();
    if (result != FND_OK)
        return result;
    char line[LINE_CAP];
    while (!quit) {
        show();
        cmd_out(":");
        uint32_t got = 0;
        fnd_status status = fnd_read(fnd_std_in(), line, LINE_CAP, &got);
        if (status != FND_OK || got == 0) /* EOF (Ctrl+D) ends the session */
            break;
        dispatch(split(line, got));
    }
    if (dirty)
        cmd_out("UNSAVED CHANGES DISCARDED (W TO SAVE)\n");
    return FND_OK;
}

CMD_MAIN(hex)
