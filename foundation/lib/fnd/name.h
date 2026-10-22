#ifndef FOUNDATION_LIBFND_NAME_H
#define FOUNDATION_LIBFND_NAME_H
#include <stdint.h>

#include <foundation/fnd.h>

/* UTF-8 and FS2 name comparison (doc/spec_FS2.md §6), shared by libfnd
 * (§11.7 of the API spec) and the fs service so that both use the same
 * case folding table. */

/* Decodes the scalar value at s[*pos]. Only shortest forms, no surrogates,
 * at most U+10FFFF. Returns FND_OK, FND_END at the end, or FND_E_BAD_PATH. */
fnd_status fnd_utf8_next(const uint8_t *s, uint32_t len, uint32_t *pos, uint32_t *cp);

/* Unicode 16.0.0 CaseFolding status C and F. Returns 1..3 scalars. */
uint32_t fnd_casefold(uint32_t cp, uint32_t out[3]);

/* Folded scalars of a valid UTF-8 string, one at a time. */
typedef struct {
    const uint8_t *s;
    uint32_t len, pos;
    uint32_t buf[3];
    uint32_t n, i;
} fnd_fold_iter;
void fnd_fold_init(fnd_fold_iter *it, const uint8_t *s, uint32_t len);
int fnd_fold_next(fnd_fold_iter *it, uint32_t *cp); /* 0 at the end */

/* Compares the folded keys of two valid UTF-8 strings. 1 = same name. */
int fnd_name_key_equal(const uint8_t *a, uint32_t alen, const uint8_t *b, uint32_t blen);
/* A hash of the folded key; equal keys have equal hashes. */
uint32_t fnd_name_key_hash(const uint8_t *s, uint32_t len);

/* Checks one FS2 name (§6): valid UTF-8, 1..256 scalars, at most 1024
 * bytes, no '/', no U+0000-U+001F or U+007F-U+009F, not "." or "..".
 * FND_E_NAME_TOO_LONG for too many scalars or bytes, else FND_E_BAD_PATH. */
fnd_status fnd_name_check(const uint8_t *s, uint32_t len, uint32_t *chars);

#endif
