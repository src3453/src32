/* libfnd name functions (doc/spec_Foundation_API.md §11.7), on top of the
 * folding in name.c. */
#include "name.h"

static uint32_t cstr_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

static uint32_t utf8_put(uint32_t c, uint8_t *o) {
    if (c < 0x80) {
        o[0] = (uint8_t)c;
        return 1;
    }
    if (c < 0x800) {
        o[0] = (uint8_t)(0xC0 | (c >> 6)), o[1] = (uint8_t)(0x80 | (c & 0x3F));
        return 2;
    }
    if (c < 0x10000) {
        o[0] = (uint8_t)(0xE0 | (c >> 12)), o[1] = (uint8_t)(0x80 | ((c >> 6) & 0x3F));
        o[2] = (uint8_t)(0x80 | (c & 0x3F));
        return 3;
    }
    o[0] = (uint8_t)(0xF0 | (c >> 18)), o[1] = (uint8_t)(0x80 | ((c >> 12) & 0x3F));
    o[2] = (uint8_t)(0x80 | ((c >> 6) & 0x3F)), o[3] = (uint8_t)(0x80 | (c & 0x3F));
    return 4;
}

fnd_status fnd_name_fold(const char *name, char *out, uint32_t cap, uint32_t *len) {
    const uint8_t *s = (const uint8_t *)name;
    uint32_t n = cstr_len(name), pos = 0, need = 0, c;
    fnd_status st;
    /* Validate and measure first, so that `out` is untouched on failure. */
    while ((st = fnd_utf8_next(s, n, &pos, &c)) == FND_OK) {
        uint32_t f[3], k = fnd_casefold(c, f);
        uint8_t tmp[4];
        for (uint32_t i = 0; i < k; ++i)
            need += utf8_put(f[i], tmp);
    }
    if (st != FND_END)
        return FND_E_BAD_PATH;
    if (need + 1 > cap) {
        *len = need + 1;
        return FND_E_LIMIT;
    }
    fnd_fold_iter it;
    uint32_t o = 0;
    fnd_fold_init(&it, s, n);
    while (fnd_fold_next(&it, &c))
        o += utf8_put(c, (uint8_t *)out + o);
    out[o] = 0;
    *len = o;
    return FND_OK;
}

int fnd_name_equal(const char *a, const char *b) {
    uint32_t al = cstr_len(a), bl = cstr_len(b), pos = 0, c;
    fnd_status st;
    while ((st = fnd_utf8_next((const uint8_t *)a, al, &pos, &c)) == FND_OK) {
    }
    if (st != FND_END)
        return 0;
    pos = 0;
    while ((st = fnd_utf8_next((const uint8_t *)b, bl, &pos, &c)) == FND_OK) {
    }
    if (st != FND_END)
        return 0;
    return fnd_name_key_equal((const uint8_t *)a, al, (const uint8_t *)b, bl);
}

/* Folded scalars of a pattern or a name, for fnd_name_match. Names are at
 * most 256 scalars, each folding to at most 3. */
#define MATCH_MAX 1024u

static int fold_all(const char *s, uint32_t *out, uint32_t *n) {
    uint32_t len = cstr_len(s), pos = 0, c, k = 0;
    fnd_status st;
    while ((st = fnd_utf8_next((const uint8_t *)s, len, &pos, &c)) == FND_OK) {
        uint32_t f[3], m = fnd_casefold(c, f);
        if (k + m > MATCH_MAX)
            return 0;
        for (uint32_t i = 0; i < m; ++i)
            out[k++] = f[i];
    }
    *n = k;
    return st == FND_END;
}

int fnd_name_match(const char *pattern, const char *name) {
    uint32_t p[MATCH_MAX], s[MATCH_MAX], pn, sn;
    if (!fold_all(pattern, p, &pn) || !fold_all(name, s, &sn))
        return 0;
    /* Greedy match with backtracking to the last '*'. */
    uint32_t i = 0, j = 0, star = 0xFFFFFFFFu, mark = 0;
    while (j < sn) {
        if (i < pn && (p[i] == '?' || p[i] == s[j]) && p[i] != '*') {
            ++i, ++j;
        } else if (i < pn && p[i] == '*') {
            star = i++;
            mark = j;
        } else if (star != 0xFFFFFFFFu) {
            i = star + 1;
            j = ++mark;
        } else {
            return 0;
        }
    }
    while (i < pn && p[i] == '*')
        ++i;
    return i == pn;
}
