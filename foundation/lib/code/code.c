/* 文字コード変換（include/foundation/code.h）。コードポイントを 1 つずつ読んで書く。
 * heap・libfnd・作業領域は使わず、読み書きは byte 単位なので整列を問わない。 */
#include <foundation/code.h>

#define REPLACEMENT 0xFFFDu
#define BOM 0xFEFFu
#define KNOWN_FLAGS (CODE_REPLACE | CODE_STRIP_BOM | CODE_ADD_BOM)

/* 次のコードポイントを読む。戻り値は CONDE_OK、CONDE_DATA（不正。*i は maximal subpart だけ進む）、
 * CONDE_INPUT_EOF（末尾で途中切れ。*i は n まで進む）。 */
static int32_t next_utf8(const uint8_t *s, uint32_t n, uint32_t *i, uint32_t *cp) {
    uint32_t p = *i, b = s[p++], lo = 0x80, hi = 0xBF, need;
    if (b < 0x80) {
        *i = p;
        *cp = b;
        return CONDE_OK;
    }
    if (b >= 0xC2 && b <= 0xDF) {
        need = 1;
        *cp = b & 0x1F;
    } else if (b >= 0xE0 && b <= 0xEF) {
        need = 2;
        *cp = b & 0x0F;
        if (b == 0xE0)
            lo = 0xA0;
        else if (b == 0xED)
            hi = 0x9F;
    } else if (b >= 0xF0 && b <= 0xF4) {
        need = 3;
        *cp = b & 0x07;
        if (b == 0xF0)
            lo = 0x90;
        else if (b == 0xF4)
            hi = 0x8F;
    } else {
        *i = p;
        return CONDE_DATA;
    }
    for (; need; --need, lo = 0x80, hi = 0xBF) {
        if (p >= n) {
            *i = p;
            return CONDE_INPUT_EOF;
        }
        b = s[p];
        if (b < lo || b > hi) {
            *i = p;
            return CONDE_DATA;
        }
        *cp = *cp << 6 | (b & 0x3F);
        ++p;
    }
    *i = p;
    return CONDE_OK;
}

static uint32_t unit16(const uint8_t *s, int big) {
    return big ? (uint32_t)s[0] << 8 | s[1] : (uint32_t)s[1] << 8 | s[0];
}

static int32_t next_utf16(const uint8_t *s, uint32_t n, uint32_t *i, uint32_t *cp, int big) {
    uint32_t p = *i, u, v;
    if (n - p < 2) {
        *i = n;
        return CONDE_INPUT_EOF;
    }
    u = unit16(s + p, big);
    p += 2;
    if (u < 0xD800 || u > 0xDFFF) {
        *i = p;
        *cp = u;
        return CONDE_OK;
    }
    if (u >= 0xDC00) {  /* 下位サロゲートだけ */
        *i = p;
        return CONDE_DATA;
    }
    if (n - p < 2) {  /* 上位サロゲートの後に続きがない（1 byte の端数を含む） */
        *i = n;
        return CONDE_INPUT_EOF;
    }
    v = unit16(s + p, big);
    if (v < 0xDC00 || v > 0xDFFF) {
        *i = p;
        return CONDE_DATA;
    }
    *i = p + 2;
    *cp = 0x10000u + ((u - 0xD800) << 10) + (v - 0xDC00);
    return CONDE_OK;
}

static int32_t next_utf32(const uint8_t *s, uint32_t n, uint32_t *i, uint32_t *cp, int big) {
    uint32_t p = *i, v;
    if (n - p < 4) {
        *i = n;
        return CONDE_INPUT_EOF;
    }
    v = big ? (uint32_t)s[p] << 24 | (uint32_t)s[p + 1] << 16 | (uint32_t)s[p + 2] << 8 | s[p + 3]
            : (uint32_t)s[p + 3] << 24 | (uint32_t)s[p + 2] << 16 | (uint32_t)s[p + 1] << 8 | s[p];
    *i = p + 4;
    if (v > 0x10FFFF || (v >= 0xD800 && v <= 0xDFFF))
        return CONDE_DATA;
    *cp = v;
    return CONDE_OK;
}

static int32_t next(uint32_t enc, const uint8_t *s, uint32_t n, uint32_t *i, uint32_t *cp) {
    switch (enc) {
    case CODE_UTF16LE: return next_utf16(s, n, i, cp, 0);
    case CODE_UTF16BE: return next_utf16(s, n, i, cp, 1);
    case CODE_UTF32LE: return next_utf32(s, n, i, cp, 0);
    case CODE_UTF32BE: return next_utf32(s, n, i, cp, 1);
    default: return next_utf8(s, n, i, cp);
    }
}

static void put16(uint8_t *d, uint32_t u, int big) {
    d[big ? 0 : 1] = (uint8_t)(u >> 8);
    d[big ? 1 : 0] = (uint8_t)u;
}

/* cp を書く（cp は有効なスカラー値）。容量不足なら CONDE_OUTPUT_EOF。 */
static int32_t put(uint8_t *d, uint32_t cap, uint32_t *o, uint32_t enc, uint32_t cp) {
    uint32_t p = *o, len, k;
    if (enc == CODE_UTF8 || enc == CODE_UTF8_BOM) {
        len = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
        if (cap - p < len)
            return CONDE_OUTPUT_EOF;
        if (len == 1) {
            d[p] = (uint8_t)cp;
        } else {
            static const uint8_t lead[5] = {0, 0, 0xC0, 0xE0, 0xF0};
            d[p] = (uint8_t)(lead[len] | cp >> 6 * (len - 1));
            for (k = 1; k < len; ++k)
                d[p + k] = (uint8_t)(0x80 | (cp >> 6 * (len - 1 - k) & 0x3F));
        }
    } else if (enc == CODE_UTF16LE || enc == CODE_UTF16BE) {
        int big = enc == CODE_UTF16BE;
        len = cp < 0x10000 ? 2 : 4;
        if (cap - p < len)
            return CONDE_OUTPUT_EOF;
        if (len == 2) {
            put16(d + p, cp, big);
        } else {
            cp -= 0x10000u;
            put16(d + p, 0xD800 + (cp >> 10), big);
            put16(d + p + 2, 0xDC00 + (cp & 0x3FF), big);
        }
    } else {
        int big = enc == CODE_UTF32BE;
        len = 4;
        if (cap - p < 4)
            return CONDE_OUTPUT_EOF;
        for (k = 0; k < 4; ++k)
            d[p + k] = (uint8_t)(cp >> 8 * (big ? 3 - k : k));
    }
    *o = p + len;
    return CONDE_OK;
}

int32_t cpt_code_detect(const uint8_t *s, uint32_t n, uint32_t *bom_size) {
    int32_t enc = CODE_UTF8;
    uint32_t len = 0;
    if (!s && n)
        return -1;
    if (n >= 4 && s[0] == 0xFF && s[1] == 0xFE && !s[2] && !s[3]) {
        enc = CODE_UTF32LE;
        len = 4;
    } else if (n >= 4 && !s[0] && !s[1] && s[2] == 0xFE && s[3] == 0xFF) {
        enc = CODE_UTF32BE;
        len = 4;
    } else if (n >= 3 && s[0] == 0xEF && s[1] == 0xBB && s[2] == 0xBF) {
        enc = CODE_UTF8_BOM;
        len = 3;
    } else if (n >= 2 && s[0] == 0xFF && s[1] == 0xFE) {
        enc = CODE_UTF16LE;
        len = 2;
    } else if (n >= 2 && s[0] == 0xFE && s[1] == 0xFF) {
        enc = CODE_UTF16BE;
        len = 2;
    }
    if (bom_size)
        *bom_size = len;
    return enc;
}

/* 入力 1 単位（UTF-8 は 1 byte、UTF-16 は 2 byte、UTF-32 は 4 byte。端数は 1 単位に数える）あたりの
 * 最悪出力 byte 数。不正な単位が U+FFFD（UTF-8 で 3 byte）になる場合を含む。
 * UTF-16 入力の対は 2 単位で 4 byte なので、単位あたりでは BMP 1 単位の方が大きい。 */
static uint32_t bound_for(uint32_t src_size, uint32_t from, uint32_t to) {
    uint32_t unit = from == CODE_UTF16LE || from == CODE_UTF16BE ? 2 : from == CODE_UTF32LE || from == CODE_UTF32BE ? 4 : 1;
    uint32_t units = src_size / unit + (src_size % unit != 0);
    uint32_t per = to == CODE_UTF16LE || to == CODE_UTF16BE ? 2 : to == CODE_UTF32LE || to == CODE_UTF32BE ? 4 : 3;
    if (unit == 4 && per < 4)
        per = 4;  /* U+10000 以上は UTF-8 も UTF-16 も 4 byte */
    if (units > 0xFFFFFFFFu / per)
        return 0xFFFFFFFFu;
    return units * per;
}

uint32_t cpt_code_bound(uint32_t src_size, uint32_t from, uint32_t to) {
    uint32_t bom, size;
    uint64_t total;
    if (to > CODE_UTF32BE || from > CODE_UTF32BE)
        return 0xFFFFFFFFu;
    bom = to == CODE_UTF16LE || to == CODE_UTF16BE ? 2 : to == CODE_UTF32LE || to == CODE_UTF32BE ? 4 : 3;
    size = bound_for(src_size, from, to);
    total = (uint64_t)size + bom;
    return total >= 0xFFFFFFFFu ? 0xFFFFFFFFu : (uint32_t)total;
}

static int32_t convert(uint8_t *dst, uint32_t *dst_size,
                         const uint8_t *src, uint32_t src_size,
                         uint32_t from, uint32_t to, uint32_t flags) {
    uint32_t cap, i = 0, o = 0, cp, first = 1;
    int32_t status = CONDE_OK;
    if (!dst_size)
        return CONDE_PARAMETER;
    cap = *dst_size;
    *dst_size = 0;
    if ((src_size && !src) || (cap && !dst) || to > CODE_UTF32BE ||
        from > CODE_UTF32BE || (flags & ~KNOWN_FLAGS))
        return CONDE_PARAMETER;
    if (from == CODE_UTF8_BOM) {
        if (src_size >= 3 && src[0] == 0xEF && src[1] == 0xBB && src[2] == 0xBF)
            i = 3;
    }
    if (from == CODE_UTF8_BOM)
        from = CODE_UTF8;
    if (to == CODE_UTF8_BOM || (flags & CODE_ADD_BOM)) {
        status = put(dst, cap, &o, to, BOM);
        if (status)
            goto fail;
    }
    while (i < src_size) {
        int32_t r = next(from, src, src_size, &i, &cp);
        if (r) {
            if (!(flags & CODE_REPLACE)) {
                status = r;
                goto fail;
            }
            cp = REPLACEMENT;
        }
        if (first && cp == BOM && !r && (flags & CODE_STRIP_BOM)) {
            first = 0;
            continue;
        }
        first = 0;
        status = put(dst, cap, &o, to, cp);
        if (status)
            goto fail;
    }
    *dst_size = o;
    return CONDE_OK;
fail:
    return status;
}

#define WRAP(name, enc)                                                                                  \
    int32_t cpt_code_##name##_to_utf32be(uint8_t *dst, uint32_t *dst_size, const uint8_t *src,           \
                                         uint32_t src_size, uint32_t flags) {                            \
        return convert(dst, dst_size, src, src_size, enc, CODE_UTF32BE, flags);                          \
    }                                                                                                    \
    int32_t cpt_code_utf32be_to_##name(uint8_t *dst, uint32_t *dst_size, const uint8_t *src,             \
                                       uint32_t src_size, uint32_t flags) {                              \
        return convert(dst, dst_size, src, src_size, CODE_UTF32BE, enc, flags);                          \
    }

WRAP(utf8, CODE_UTF8)
WRAP(utf8bom, CODE_UTF8_BOM)
WRAP(utf16le, CODE_UTF16LE)
WRAP(utf16be, CODE_UTF16BE)
WRAP(utf32le, CODE_UTF32LE)
