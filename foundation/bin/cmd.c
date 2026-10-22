/* Output helpers of the standard commands (cmd.h). */
#include "cmd.h"

uint32_t cmd_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

static void put(fnd_handle h, const char *s) {
    uint32_t n = cmd_len(s);
    if (n)
        (void)fnd_write(h, s, n, 0);
}

void cmd_out(const char *s) { put(fnd_std_out(), s); }
void cmd_err(const char *s) { put(fnd_std_err(), s); }

static uint32_t format_i32(int32_t value, char *out) {
    char digits[11];
    uint32_t n = 0, len = 0, v = value < 0 ? 0u - (uint32_t)value : (uint32_t)value;
    do {
        digits[n++] = (char)('0' + v % 10u);
        v /= 10u;
    } while (v);
    if (value < 0)
        out[len++] = '-';
    while (n)
        out[len++] = digits[--n];
    out[len] = 0;
    return len;
}

void cmd_out_dec(int32_t value, uint32_t width) {
    char text[12];
    uint32_t n = format_i32(value, text);
    while (n < width--)
        cmd_out(" ");
    cmd_out(text);
}

int32_t cmd_report(const char *command, const char *target, int32_t code) {
    char text[12];
    cmd_err(command);
    cmd_err(": ");
    if (target) {
        cmd_err(target);
        cmd_err(": ");
    }
    cmd_err(fnd_strerror(code));
    cmd_err(" (");
    format_i32(code, text);
    cmd_err(text);
    cmd_err(")\n");
    return code;
}

int32_t cmd_usage(const char *text) {
    cmd_err("usage: ");
    cmd_err(text);
    cmd_err("\n");
    return FND_E_BAD_ARG;
}
