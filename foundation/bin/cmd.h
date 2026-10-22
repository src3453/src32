#ifndef FOUNDATION_BIN_CMD_H
#define FOUNDATION_BIN_CMD_H
#include <foundation/fnd.h>
#include <foundation/sys.h>

/* Output helpers shared by the standard commands in /bin
 * (doc/spec_Foundation_shell.md §10.2, §11.1). Linked into each command
 * until libfnd is a WAL. */

uint32_t cmd_len(const char *s);
void cmd_out(const char *s);
void cmd_err(const char *s);
void cmd_out_dec(int32_t value, uint32_t width);
/* "<command>: <target>: <message> (<code>)", target optional; returns code. */
int32_t cmd_report(const char *command, const char *target, int32_t code);
/* "usage: <text>" on STDERR; returns FND_E_BAD_ARG. */
int32_t cmd_usage(const char *text);
/* fnd_init, then the command's main body with argc and argv, then exit. */
#define CMD_MAIN(body)                                                                        \
    int main(void) {                                                                          \
        fnd_init();                                                                           \
        fnd_exit(body(fnd_argc()));                                                           \
    }

#endif
