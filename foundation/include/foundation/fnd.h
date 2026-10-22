#ifndef FOUNDATION_FND_H
#define FOUNDATION_FND_H
#include <stdint.h>

#include <foundation/api.h>

/* libfnd (doc/spec_Foundation_API.md §11), the part that exists so far.
 *
 * Provisional: the kernel has no IPC and no handles yet, so there is no
 * console service. Until there is, the console is lib/fnd/con_pcg.c, linked
 * into the program, which drives the PCG and the PeC input itself through
 * MMIO grants. fnd_init then gives every console role (STDIN, STDOUT,
 * STDERR, CONSOLE) the handle FND_LOCAL_CONSOLE, as init would pass one
 * console stream for all four (§12); there is no FS role. */

typedef int32_t fnd_status;
typedef uint32_t fnd_handle;

#define FND_LOCAL_CONSOLE 1u

typedef struct {
    uint32_t code; /* FND_KEY_* */
    uint32_t ch;   /* Unicode scalar for FND_KEY_CHAR, else 0 */
    uint32_t mods; /* FND_MOD_* */
} fnd_key_t;

/* §11.2 */
fnd_status fnd_init(void);
uint32_t fnd_argc(void);
const char *fnd_argv(uint32_t i);
fnd_handle fnd_std_in(void);
fnd_handle fnd_std_out(void);
fnd_handle fnd_std_err(void);
fnd_handle fnd_console(void);
fnd_handle fnd_fs(void);
void fnd_exit(int32_t code) __attribute__((noreturn));

/* §11.3 */
fnd_status fnd_write(fnd_handle h, const void *buf, uint32_t len, uint32_t *put);

/* §11.5 */
fnd_status fnd_con_write(const char *text);
fnd_status fnd_con_getkey(fnd_key_t *key);
fnd_status fnd_con_clear(void);
fnd_status fnd_con_size(uint32_t *cols, uint32_t *rows);

/* §11.7 (lib/fnd/name.c) */
fnd_status fnd_name_fold(const char *name, char *out, uint32_t cap, uint32_t *len);
int fnd_name_equal(const char *a, const char *b);
int fnd_name_match(const char *pattern, const char *name);

/* §11.9 */
const char *fnd_strerror(fnd_status code);

#endif
