/* WAX loader core: PROTECTED WAX v2 and WAX/WAL v3 (doc/spec_WAX_v3.md,
 * doc/spec_WAX.md, doc/spec_Foundation.md §5.3, §9.2). Everything here is
 * plain C11 on byte arrays; only wax_load writes outside the launch record. */
#include "wax.h"

#define HEADER_V2 64u
#define HEADER_V3 160u
#define TYPE_WAX 1u
#define TYPE_WAL 2u
#define FLAG_DEBUG 1u
#define FLAG_PROTECTED 2u
#define PERM_R 1u
#define PERM_RW 3u
#define PERM_RX 5u
#define KIND_FUNCTION 1u
#define KIND_OBJECT 2u
#define CPU_RESERVED 0xFFFF0000u
#define NAME_MAX 127u
#define MAX_SPANS (8u + WAX_MAX_BLOCKS)

static uint32_t rd16(const uint8_t *p) { return (uint32_t)p[0] << 8 | p[1]; }

static uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}

static uint32_t word(const struct wax_image *im, uint32_t offset) { return rd32(im->data + offset); }

static int str_eq(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return *a == *b;
}

/* ASCII byte order, as exports are sorted (v3 §4.2). */
static int str_cmp(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return (int)(uint8_t)*a - (int)(uint8_t)*b;
}

/* --- strings (v3 §2.1) --- */

/* A string-table offset that starts a nonempty string, or 0. The table ends
 * with NUL (checked by parse_v3), so every string terminates inside it. */
static const char *str(const struct wax_image *im, uint32_t offset) {
    const uint8_t *table = im->data + im->strings;
    if (offset == 0 || offset >= im->string_size || table[offset - 1u] != 0 || table[offset] == 0)
        return 0;
    return (const char *)table + offset;
}

static int is_lower(char c) { return c >= 'a' && c <= 'z'; }
static int is_digit(char c) { return c >= '0' && c <= '9'; }
static int is_alpha(char c) { return is_lower(c) || (c >= 'A' && c <= 'Z'); }

/* [a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*, at most 127 bytes. */
static int is_id(const char *s) {
    uint32_t n = 0;
    if (!s)
        return 0;
    for (;;) {
        if (!is_lower(s[n]))
            return 0;
        ++n;
        while (is_lower(s[n]) || is_digit(s[n]) || s[n] == '_')
            ++n;
        if (s[n] != '.')
            break;
        ++n;
    }
    return s[n] == 0 && n <= NAME_MAX;
}

/* [A-Za-z_][A-Za-z0-9_]*, at most 127 bytes. */
static int is_symbol(const char *s) {
    uint32_t n = 0;
    if (!s || !(is_alpha(s[0]) || s[0] == '_'))
        return 0;
    while (is_alpha(s[n]) || is_digit(s[n]) || s[n] == '_')
        ++n;
    return s[n] == 0 && n <= NAME_MAX;
}

static uint32_t prefix(const char *s, const char *p) {
    uint32_t n = 0;
    while (p[n]) {
        if (s[n] != p[n])
            return 0;
        ++n;
    }
    return n;
}

/* Length of the type name at s, or 0. void only as a result (v3 §6). */
static uint32_t sig_type(const char *s, int result) {
    static const char *const types[] = {"u32", "i32", "u64", "i64", "ptr", "fnptr"};
    for (uint32_t i = 0; i < sizeof types / sizeof types[0]; ++i) {
        uint32_t n = prefix(s, types[i]);
        if (n)
            return n;
    }
    return result ? prefix(s, "void") : 0;
}

/* fn(type,...)->type with no whitespace. */
static int is_signature(const char *s) {
    uint32_t n;
    if (!s || !(n = prefix(s, "fn(")))
        return 0;
    s += n;
    if (*s != ')') {
        for (;;) {
            if (!(n = sig_type(s, 0)))
                return 0;
            s += n;
            if (*s != ',')
                break;
            ++s;
        }
    }
    if (!(n = prefix(s, ")->")))
        return 0;
    s += n;
    if (!(n = sig_type(s, 1)))
        return 0;
    return s[n] == 0;
}

/* --- blocks --- */

struct block {
    uint32_t address, file_offset, file_size, memory_size, perm, align;
};

static void get_block(const struct wax_image *im, uint32_t i, struct block *b) {
    const uint8_t *e = im->data + im->block_table + i * im->block_entry_size;
    b->address = rd32(e);
    b->file_offset = rd32(e + 4);
    b->file_size = rd32(e + 8);
    if (im->version == 2u) {
        b->memory_size = b->file_size;
        b->perm = rd32(e + 12);
        b->align = 1;
    } else {
        b->memory_size = rd32(e + 12);
        b->perm = rd32(e + 16);
        b->align = rd32(e + 20);
    }
}

enum { NEED_EXEC, NEED_DATA, NEED_RW };

/* Whether image-relative [offset, offset + len) lies in one block that has
 * the permission need wants: initialized RX bytes, R/RW, or RW. */
static int in_block(const struct wax_image *im, uint32_t offset, uint32_t len, int need) {
    if (len > 0xFFFFFFFFu - offset)
        return 0;
    for (uint32_t i = 0; i < im->block_count; ++i) {
        struct block b;
        get_block(im, i, &b);
        uint32_t rel = b.address - im->base;
        int perm_ok = need == NEED_EXEC ? b.perm == PERM_RX
                      : need == NEED_RW ? b.perm == PERM_RW
                                        : b.perm == PERM_R || b.perm == PERM_RW;
        uint32_t size = need == NEED_EXEC ? b.file_size : b.memory_size;
        if (perm_ok && rel <= offset && offset - rel <= size && len <= size - (offset - rel))
            return 1;
    }
    return 0;
}

/* Byte at image-relative offset before loading: payload, else zero tail. */
static uint32_t initial_byte(const struct wax_image *im, uint32_t offset) {
    for (uint32_t i = 0; i < im->block_count; ++i) {
        struct block b;
        get_block(im, i, &b);
        uint32_t rel = b.address - im->base;
        if (rel <= offset && offset - rel < b.file_size)
            return im->data[b.file_offset + (offset - rel)];
    }
    return 0;
}

/* --- file ranges --- */

struct span {
    uint32_t start, end;
};

struct spans {
    uint32_t count;
    struct span item[MAX_SPANS];
};

/* Records a nonempty file range, kept sorted by start. */
static int add_span(struct spans *s, uint32_t offset, uint32_t len, uint32_t size) {
    if (offset > size || len > size - offset)
        return 0;
    if (len == 0)
        return 1;
    uint32_t i = s->count++;
    while (i && s->item[i - 1u].start > offset) {
        s->item[i] = s->item[i - 1u];
        --i;
    }
    s->item[i].start = offset;
    s->item[i].end = offset + len;
    return 1;
}

/* Ranges are pairwise disjoint; with zero_gaps every byte outside them is
 * zero padding (v3 §2). */
static int spans_ok(const struct spans *s, const uint8_t *data, uint32_t size, int zero_gaps) {
    uint32_t at = 0;
    for (uint32_t i = 0; i <= s->count; ++i) {
        uint32_t start = i < s->count ? s->item[i].start : size;
        if (start < at)
            return 0;
        for (; zero_gaps && at < start; ++at) {
            if (data[at])
                return 0;
        }
        if (i < s->count)
            at = s->item[i].end;
    }
    return 1;
}

/* count entries of entry_size bytes at offset: absent tables are 0/0. */
static int add_table(struct spans *s, uint32_t offset, uint32_t count, uint32_t entry_size,
                     uint32_t size) {
    if (count == 0)
        return offset == 0;
    if ((offset & 3u) || count > size / entry_size)
        return 0;
    return add_span(s, offset, count * entry_size, size);
}

/* --- v2 PROTECTED (doc/spec_WAX.md) --- */

static enum wax_error parse_v2(struct wax_image *im) {
    const uint8_t *d = im->data;
    uint32_t size = im->size;
    struct spans spans;
    if (size < HEADER_V2)
        return WAX_E_FORMAT;
    uint32_t type = rd16(d + 6), flags = rd16(d + 10);
    if (rd16(d + 8) != 1u || type < TYPE_WAX || type > TYPE_WAL || (flags & ~3u))
        return WAX_E_UNSUPPORTED;
    if (type != TYPE_WAX || !(flags & FLAG_PROTECTED))
        return WAX_E_FORMAT; /* user programs only (Foundation §9.2) */
    im->type = type;
    im->base = word(im, 16);
    im->entry_offset = word(im, 20);
    im->block_table = word(im, 24);
    im->block_count = word(im, 28);
    im->block_entry_size = word(im, 32);
    uint32_t debug_offset = word(im, 36), debug_size = word(im, 40);
    im->stack_top = word(im, 48);
    im->stack_size = word(im, 52);
    if (word(im, 12) != HEADER_V2 || word(im, 44) != size || word(im, 56) || word(im, 60))
        return WAX_E_FORMAT;
    if (im->block_entry_size != 16u || im->block_count == 0 || im->block_count > 7u)
        return WAX_E_FORMAT;
    if ((im->block_table & 3u) || im->block_table < HEADER_V2 || im->block_table > size ||
        im->block_count * 16u > size - im->block_table)
        return WAX_E_FORMAT;
    if (!(flags & FLAG_DEBUG) != !debug_size || (!debug_size && debug_offset))
        return WAX_E_FORMAT;
    uint32_t table_end = im->block_table + im->block_count * 16u, end = 0;
    spans.count = 0;
    if (!add_span(&spans, debug_offset, debug_size, size))
        return WAX_E_FORMAT;
    for (uint32_t i = 0; i < im->block_count; ++i) {
        struct block b;
        get_block(im, i, &b);
        if (!b.file_size || !b.perm || (b.perm & ~7u) || (b.file_offset & 3u) ||
            b.file_offset < table_end || !add_span(&spans, b.file_offset, b.file_size, size))
            return WAX_E_FORMAT;
        if (b.file_size > CPU_RESERVED - b.address || (i ? b.address < end : b.address != im->base))
            return WAX_E_FORMAT;
        end = b.address + b.file_size;
    }
    if (!spans_ok(&spans, d, size, 0))
        return WAX_E_FORMAT;
    im->span = end - im->base;
    im->align = 1;
    /* Stack: nonzero multiples of 4, below the reserved area, off every block. */
    uint32_t top = im->stack_top, stack = im->stack_size;
    if (!stack || (stack & 3u) || (top & 3u) || stack > top || top > CPU_RESERVED)
        return WAX_E_FORMAT;
    for (uint32_t i = 0; i < im->block_count; ++i) {
        struct block b;
        get_block(im, i, &b);
        if (top - stack < b.address + b.file_size && b.address < top)
            return WAX_E_FORMAT;
    }
    uint32_t entry = im->base + im->entry_offset;
    if (entry < im->base)
        return WAX_E_FORMAT;
    for (uint32_t i = 0; i < im->block_count; ++i) {
        struct block b;
        get_block(im, i, &b);
        if ((b.perm & 4u) && b.address <= entry && entry - b.address < b.file_size)
            return WAX_OK;
    }
    return WAX_E_FORMAT;
}

/* --- v3 (doc/spec_WAX_v3.md §2-§4) --- */

static enum wax_error parse_v3(struct wax_image *im) {
    const uint8_t *d = im->data;
    uint32_t size = im->size;
    struct spans spans;
    if (size < HEADER_V3)
        return WAX_E_FORMAT;
    uint32_t type = rd16(d + 6), flags = rd16(d + 10);
    if (rd16(d + 8) != 1u || type < TYPE_WAX || type > TYPE_WAL || (flags & ~3u) || word(im, 20) != 1u)
        return WAX_E_UNSUPPORTED; /* arch, type, flags, call_abi */
    if (!(flags & FLAG_PROTECTED) || word(im, 12) != HEADER_V3 || word(im, 16) != size)
        return WAX_E_FORMAT;
    for (uint32_t offset = 128; offset < HEADER_V3; offset += 4u) {
        if (word(im, offset))
            return WAX_E_FORMAT;
    }
    im->type = type;
    im->base = word(im, 24);
    im->entry_offset = word(im, 28);
    im->stack_top = word(im, 32);
    im->stack_size = word(im, 36);
    im->block_table = word(im, 40);
    im->block_count = word(im, 44);
    im->block_entry_size = 24;
    im->dep_table = word(im, 52);
    im->dep_count = word(im, 56);
    im->export_table = word(im, 64);
    im->export_count = word(im, 68);
    im->import_table = word(im, 76);
    im->import_count = word(im, 80);
    im->strings = word(im, 88);
    im->string_size = word(im, 92);
    im->image_id = word(im, 96);
    im->abi_major = word(im, 100);
    im->abi_minor = word(im, 104);
    uint32_t debug_offset = word(im, 120), debug_size = word(im, 124);
    if (word(im, 48) != 24u || word(im, 60) != 16u || word(im, 72) != 24u || word(im, 84) != 24u)
        return WAX_E_FORMAT;
    if (!im->block_count || !im->string_size || !im->abi_major || !word(im, 112))
        return WAX_E_FORMAT;
    if (im->block_count > WAX_MAX_BLOCKS || im->dep_count > WAX_MAX_DEPS)
        return WAX_E_RESOURCE_LIMIT;
    if (type == TYPE_WAX ? im->export_count || !im->stack_size
                         : im->base || im->entry_offset || im->stack_top || im->stack_size ||
                               !im->export_count)
        return WAX_E_FORMAT;
    if (!(flags & FLAG_DEBUG) != !debug_size || (!debug_size && debug_offset) || (debug_offset & 3u))
        return WAX_E_FORMAT;

    /* Header, tables, strings, payloads and debug: disjoint, aligned, inside
     * the file, with zero padding between them. */
    spans.count = 0;
    if (!add_span(&spans, 0, HEADER_V3, size) ||
        !add_table(&spans, im->block_table, im->block_count, 24u, size) ||
        !add_table(&spans, im->dep_table, im->dep_count, 16u, size) ||
        !add_table(&spans, im->export_table, im->export_count, 24u, size) ||
        !add_table(&spans, im->import_table, im->import_count, 24u, size) ||
        !add_table(&spans, im->strings, 1u, im->string_size, size) ||
        !add_span(&spans, debug_offset, debug_size, size))
        return WAX_E_FORMAT;

    /* Strings: leading and trailing NUL, ASCII only. */
    const uint8_t *table = d + im->strings;
    if (table[0] || table[im->string_size - 1u])
        return WAX_E_FORMAT;
    for (uint32_t i = 0; i < im->string_size; ++i) {
        if (table[i] & 0x80u)
            return WAX_E_FORMAT;
    }
    const char *own_id = str(im, im->image_id);
    if (!is_id(own_id) || !is_id(str(im, word(im, 108))))
        return WAX_E_FORMAT;

    /* Blocks (§3). */
    uint32_t end = 0;
    im->align = 4;
    for (uint32_t i = 0; i < im->block_count; ++i) {
        struct block b;
        get_block(im, i, &b);
        if (b.perm != PERM_R && b.perm != PERM_RW && b.perm != PERM_RX)
            return WAX_E_FORMAT;
        if (b.align < 4u || (b.align & (b.align - 1u)) || (b.address & (b.align - 1u)))
            return WAX_E_FORMAT;
        if (!b.memory_size || b.file_size > b.memory_size || !b.file_size != !b.file_offset ||
            b.memory_size > 0xFFFFFFFFu - b.address)
            return WAX_E_FORMAT;
        if (i ? b.address < end : b.address != im->base)
            return WAX_E_FORMAT; /* sorted, no overlap; first at base_addr / 0 */
        if ((b.file_offset & 3u) || !add_span(&spans, b.file_offset, b.file_size, size))
            return WAX_E_FORMAT;
        if (b.align > im->align)
            im->align = b.align;
        end = b.address + b.memory_size;
    }
    im->span = end - im->base;
    if (!spans_ok(&spans, d, size, 1))
        return WAX_E_FORMAT;

    /* Entry and stack (§3, WAX only). */
    if (type == TYPE_WAX) {
        uint32_t top = im->stack_top, stack = im->stack_size;
        if ((im->entry_offset & 3u) || !in_block(im, im->entry_offset, 4u, NEED_EXEC))
            return WAX_E_FORMAT;
        if (!top || (top & 7u) || (stack & 7u) || stack > top)
            return WAX_E_FORMAT;
        if (top - stack < end && im->base < top)
            return WAX_E_FORMAT; /* stack overlaps the image reservation */
    }

    /* Dependencies (§4.1). */
    for (uint32_t i = 0; i < im->dep_count; ++i) {
        uint32_t e = im->dep_table + 16u * i;
        const char *id = str(im, word(im, e));
        if (!is_id(id) || !word(im, e + 4) || word(im, e + 12) || str_eq(id, own_id))
            return WAX_E_FORMAT;
        for (uint32_t j = 0; j < i; ++j) {
            if (str_eq(id, str(im, word(im, im->dep_table + 16u * j))))
                return WAX_E_FORMAT;
        }
    }

    /* Exports (§4.2): sorted by name, no duplicates even across kinds. */
    const char *previous = 0;
    for (uint32_t i = 0; i < im->export_count; ++i) {
        uint32_t e = im->export_table + 24u * i;
        const char *name = str(im, word(im, e));
        uint32_t kind = word(im, e + 4), offset = word(im, e + 8), len = word(im, e + 12);
        if (!is_symbol(name) || word(im, e + 20) || (previous && str_cmp(previous, name) >= 0))
            return WAX_E_FORMAT;
        previous = name;
        if (kind == KIND_FUNCTION) {
            if (!is_signature(str(im, word(im, e + 16))) || !len || (len & 3u) || (offset & 3u) ||
                !in_block(im, offset, len, NEED_EXEC))
                return WAX_E_FORMAT;
        } else if (kind == KIND_OBJECT) {
            if (word(im, e + 16) || !len || !in_block(im, offset, len, NEED_DATA))
                return WAX_E_FORMAT;
        } else {
            return WAX_E_FORMAT;
        }
    }

    /* Imports (§4.3): sorted, disjoint 8-byte RW slots that start zeroed;
     * every dependency is used. */
    uint32_t used = 0, slot_end = 0;
    for (uint32_t i = 0; i < im->import_count; ++i) {
        uint32_t e = im->import_table + 24u * i;
        uint32_t dep = word(im, e), kind = word(im, e + 8), slot = word(im, e + 12);
        uint32_t len = word(im, e + 16), signature = word(im, e + 20);
        if (dep >= im->dep_count || !is_symbol(str(im, word(im, e + 4))))
            return WAX_E_FORMAT;
        if (kind == KIND_FUNCTION ? len || !is_signature(str(im, signature))
                                  : kind != KIND_OBJECT || signature || !len)
            return WAX_E_FORMAT;
        if ((slot & 3u) || (i && slot < slot_end) || !in_block(im, slot, 8u, NEED_RW))
            return WAX_E_FORMAT;
        for (uint32_t k = 0; k < 8u; ++k) {
            if (initial_byte(im, slot + k))
                return WAX_E_FORMAT;
        }
        slot_end = slot + 8u;
        used |= 1u << dep;
    }
    if (used != (1u << im->dep_count) - 1u)
        return WAX_E_FORMAT;
    return WAX_OK;
}

static enum wax_error parse(struct wax_image *im, const void *data, uint32_t size) {
    const uint8_t *d = data;
    for (uint32_t i = 0; i < sizeof *im; ++i)
        ((uint8_t *)im)[i] = 0;
    im->data = d;
    im->size = size;
    if (!d || size < 8u || d[0] != 'W' || d[1] != 'A' || d[2] != 'X' || d[3] != 0)
        return WAX_E_FORMAT;
    im->version = rd16(d + 4);
    if (im->version == 2u)
        return parse_v2(im);
    if (im->version == 3u)
        return parse_v3(im);
    return WAX_E_UNSUPPORTED;
}

enum wax_error wax_base_addr(const void *data, uint32_t size, uint32_t *base) {
    const uint8_t *d = data;
    if (!d || size < HEADER_V2 || d[0] != 'W' || d[1] != 'A' || d[2] != 'X' || d[3] != 0)
        return WAX_E_FORMAT;
    uint32_t version = rd16(d + 4);
    if (version == 2u) {
        *base = rd32(d + 16);
        return WAX_OK;
    }
    if (version == 3u) {
        if (size < HEADER_V3)
            return WAX_E_FORMAT;
        *base = rd32(d + 24);
        return WAX_OK;
    }
    return WAX_E_UNSUPPORTED;
}

/* --- launch planning (§4.1, §5, §7) --- */

/* Every v3 image names the same OS ABI, at a minor the OS supports. The
 * call ABI is already 1 (parse_v3). */
static int abi_ok(const struct wax_image *im, const struct wax_target *target) {
    return str_eq(str(im, word(im, 108)), target->os_id) && word(im, 112) == target->os_major &&
           word(im, 116) <= target->os_minor;
}

static enum wax_error fail(struct wax_launch *launch, enum wax_error error, uint32_t image,
                           uint32_t index, const char *name) {
    launch->diag_image = image;
    launch->diag_index = index;
    launch->diag_name = name;
    return error;
}

struct resolver {
    struct wax_launch *launch;
    const struct wax_library *libs;
    uint32_t lib_count;
    const struct wax_target *target;
    uint8_t color[WAX_MAX_IMAGES]; /* 1 while its dependencies resolve, then 2 */
    uint8_t order[WAX_MAX_IMAGES]; /* post-order: dependencies first */
    uint32_t ordered;
};

/* Cheap filter before a full parse: a v3 WAL whose header names image_id
 * id at this major. Reads only bytes inside the file. */
static int may_provide(const struct wax_library *lib, const char *id, uint32_t major) {
    const uint8_t *d = lib->data;
    if (!d || lib->size < HEADER_V3 || rd16(d + 4) != 3u || rd16(d + 6) != TYPE_WAL ||
        rd32(d + 100) != major)
        return 0;
    uint32_t strings = rd32(d + 88), size = rd32(d + 92), offset = rd32(d + 96);
    if (strings > lib->size || size > lib->size - strings || offset >= size)
        return 0;
    for (uint32_t i = offset; i < size; ++i) {
        if (d[strings + i] != (uint8_t)*id)
            return 0;
        if (!*id++)
            return 1;
    }
    return 0;
}

/* Provider selection: among the libraries that validate as a WAL with this
 * image_id and abi_major, the highest abi_minor; the first one listed wins a
 * tie. The choice does not depend on min_minor, so combining minor
 * requirements by maximum is a check against the chosen instance. */
static enum wax_error resolve(struct resolver *r, uint32_t index) {
    struct wax_launch *launch = r->launch;
    struct wax_image *im = &launch->image[index];
    r->color[index] = 1;
    for (uint32_t d = 0; d < im->dep_count; ++d) {
        uint32_t e = im->dep_table + 16u * d;
        const char *id = str(im, word(im, e));
        uint32_t major = word(im, e + 4), minor = word(im, e + 8), j;
        for (j = 1; j < launch->count; ++j) {
            if (str_eq(str(&launch->image[j], launch->image[j].image_id), id))
                break;
        }
        if (j < launch->count) {
            if (r->color[j] == 1u)
                return fail(launch, WAX_E_DEPENDENCY_CYCLE, index, d, id);
            if (launch->image[j].abi_major != major)
                return fail(launch, WAX_E_DEPENDENCY_CONFLICT, index, d, id);
            if (launch->image[j].abi_minor < minor)
                return fail(launch, WAX_E_DEPENDENCY_MISSING, index, d, id);
        } else {
            if (launch->count == WAX_MAX_IMAGES)
                return fail(launch, WAX_E_RESOURCE_LIMIT, index, d, id);
            struct wax_image *chosen = &launch->image[j];
            int found = 0;
            for (uint32_t c = 0; c < r->lib_count; ++c) {
                struct wax_image candidate;
                if (!may_provide(&r->libs[c], id, major) ||
                    parse(&candidate, r->libs[c].data, r->libs[c].size) != WAX_OK ||
                    candidate.version != 3u || candidate.type != TYPE_WAL ||
                    candidate.abi_major != major || !str_eq(str(&candidate, candidate.image_id), id))
                    continue;
                if (!found || candidate.abi_minor > chosen->abi_minor)
                    *chosen = candidate;
                found = 1;
            }
            if (!found || chosen->abi_minor < minor)
                return fail(launch, WAX_E_DEPENDENCY_MISSING, index, d, id);
            if (!abi_ok(chosen, r->target))
                return fail(launch, WAX_E_ABI_MISMATCH, index, d, id);
            launch->count = j + 1u;
            r->color[j] = 0;
            enum wax_error error = resolve(r, j);
            if (error != WAX_OK)
                return error;
        }
        im->dep_image[d] = (uint8_t)j;
    }
    r->color[index] = 2;
    r->order[r->ordered++] = (uint8_t)index;
    return WAX_OK;
}

/* Export entry offset of name in im, or 0 (binary search, §4.2 order). */
static uint32_t find_export(const struct wax_image *im, const char *name) {
    uint32_t low = 0, high = im->export_count;
    while (low < high) {
        uint32_t mid = low + (high - low) / 2u, e = im->export_table + 24u * mid;
        int order = str_cmp(name, str(im, word(im, e)));
        if (!order)
            return e;
        if (order < 0)
            high = mid;
        else
            low = mid + 1u;
    }
    return 0;
}

static enum wax_error check_imports(struct wax_launch *launch) {
    for (uint32_t i = 0; i < launch->count; ++i) {
        const struct wax_image *im = &launch->image[i];
        for (uint32_t k = 0; k < im->import_count; ++k) {
            uint32_t e = im->import_table + 24u * k;
            const struct wax_image *provider = &launch->image[im->dep_image[word(im, e)]];
            const char *name = str(im, word(im, e + 4));
            uint32_t x = find_export(provider, name), kind = word(im, e + 8);
            if (!x)
                return fail(launch, WAX_E_SYMBOL_MISSING, i, k, name);
            if (word(provider, x + 4) != kind ||
                (kind == KIND_FUNCTION ? !str_eq(str(im, word(im, e + 20)), str(provider, word(provider, x + 16)))
                                       : word(im, e + 16) != word(provider, x + 12)))
                return fail(launch, WAX_E_SYMBOL_MISMATCH, i, k, name);
        }
    }
    return WAX_OK;
}

static int overlaps(uint32_t a, uint32_t a_end, uint32_t b, uint32_t b_end) {
    return a < b_end && b < a_end;
}

/* First fit from the window start, aligned to the library's largest block
 * alignment, clear of the executable, its stack and the libraries placed
 * before it. Placement follows the dependency post-order. */
static enum wax_error place(struct wax_launch *launch, const struct resolver *r) {
    const struct wax_target *t = r->target;
    struct span taken[WAX_MAX_IMAGES + 1u];
    uint32_t n = 0;
    const struct wax_image *exe = &launch->image[0];
    taken[n].start = exe->base;
    taken[n++].end = exe->base + exe->span;
    taken[n].start = exe->stack_top - exe->stack_size;
    taken[n++].end = exe->stack_top;
    for (uint32_t i = 0; i < n; ++i) {
        if (taken[i].start < t->window_start || taken[i].end > t->window_end)
            return fail(launch, WAX_E_FORMAT, 0, 0, 0);
    }
    for (uint32_t o = 0; o < r->ordered; ++o) {
        uint32_t index = r->order[o];
        struct wax_image *im = &launch->image[index];
        if (!index)
            continue;
        uint32_t mask = im->align - 1u, at = t->window_start, moved = 1;
        while (moved) {
            moved = 0;
            if (at > 0xFFFFFFFFu - mask)
                return fail(launch, WAX_E_FORMAT, index, 0, 0);
            at = (at + mask) & ~mask;
            if (im->span > t->window_end || at > t->window_end - im->span)
                return fail(launch, WAX_E_FORMAT, index, 0, 0);
            for (uint32_t i = 0; i < n; ++i) {
                if (overlaps(at, at + im->span, taken[i].start, taken[i].end)) {
                    at = taken[i].end;
                    moved = 1;
                    break;
                }
            }
        }
        im->origin = at;
        taken[n].start = at;
        taken[n++].end = at + im->span;
    }
    return WAX_OK;
}

/* §5: region 0 is the stack; the blocks of all images follow in address
 * order, merging only exactly adjacent blocks with equal permissions. */
static enum wax_error plan_regions(struct wax_launch *launch) {
    const struct wax_image *exe = &launch->image[0];
    uint32_t next[WAX_MAX_IMAGES] = {0}, n = 1;
    launch->region[0].start = exe->stack_top - exe->stack_size;
    launch->region[0].end = exe->stack_top;
    launch->region[0].flags = PERM_RW;
    launch->top = 0;
    for (;;) {
        uint32_t best = WAX_MAX_IMAGES, best_start = 0;
        struct block b, chosen;
        for (uint32_t i = 0; i < launch->count; ++i) {
            const struct wax_image *im = &launch->image[i];
            if (next[i] == im->block_count)
                continue;
            get_block(im, next[i], &b);
            uint32_t start = im->origin + (b.address - im->base);
            if (best == WAX_MAX_IMAGES || start < best_start) {
                best = i;
                best_start = start;
                chosen = b;
            }
        }
        if (best == WAX_MAX_IMAGES)
            break;
        ++next[best];
        uint32_t end = best_start + chosen.memory_size;
        struct wax_region *last = &launch->region[n - 1u];
        if (n > 1u && last->end == best_start && last->flags == chosen.perm) {
            last->end = end;
        } else {
            if (n == WAX_REGIONS)
                return fail(launch, WAX_E_REGION_LIMIT, best, next[best] - 1u, 0);
            launch->region[n].start = best_start;
            launch->region[n].end = end;
            launch->region[n].flags = chosen.perm;
            ++n;
        }
        if (end > launch->top)
            launch->top = end;
    }
    launch->region_count = n;
    return WAX_OK;
}

enum wax_error wax_plan(struct wax_launch *launch, const void *data, uint32_t size,
                        const struct wax_library *libs, uint32_t lib_count,
                        const struct wax_target *target) {
    struct resolver r;
    launch->count = 0;
    launch->region_count = 0;
    fail(launch, WAX_OK, 0, 0, 0);
    struct wax_image *exe = &launch->image[0];
    enum wax_error error = parse(exe, data, size);
    if (error != WAX_OK)
        return error;
    if (exe->type != TYPE_WAX)
        return WAX_E_FORMAT;
    if (exe->version == 3u && !abi_ok(exe, target))
        return WAX_E_ABI_MISMATCH;
    exe->origin = exe->base;
    launch->count = 1;
    r.launch = launch;
    r.libs = libs;
    r.lib_count = lib_count;
    r.target = target;
    r.ordered = 0;
    if ((error = resolve(&r, 0)) != WAX_OK || (error = check_imports(launch)) != WAX_OK ||
        (error = place(launch, &r)) != WAX_OK || (error = plan_regions(launch)) != WAX_OK)
        return error;
    launch->entry = exe->base + exe->entry_offset;
    launch->stack_top = exe->stack_top;
    launch->stack_size = exe->stack_size;
    return WAX_OK;
}

static void put32(uintptr_t bias, uint32_t address, uint32_t value) {
    volatile uint8_t *p = (volatile uint8_t *)(bias + address);
    p[0] = (uint8_t)(value >> 24);
    p[1] = (uint8_t)(value >> 16);
    p[2] = (uint8_t)(value >> 8);
    p[3] = (uint8_t)value;
}

void wax_load(const struct wax_launch *launch, uintptr_t bias) {
    for (uint32_t i = 0; i < launch->count; ++i) {
        const struct wax_image *im = &launch->image[i];
        for (uint32_t k = 0; k < im->block_count; ++k) {
            struct block b;
            get_block(im, k, &b);
            volatile uint8_t *dst = (volatile uint8_t *)(bias + im->origin + (b.address - im->base));
            const uint8_t *src = im->data + b.file_offset;
            uint32_t at = 0;
            for (; at < b.file_size; ++at)
                dst[at] = src[at];
            for (; at < b.memory_size; ++at)
                dst[at] = 0;
        }
    }
    volatile uint8_t *stack = (volatile uint8_t *)(bias + launch->stack_top - launch->stack_size);
    for (uint32_t at = 0; at < launch->stack_size; ++at)
        stack[at] = 0;
    /* Import descriptors (§4.3): resolved address, provider origin. */
    for (uint32_t i = 0; i < launch->count; ++i) {
        const struct wax_image *im = &launch->image[i];
        for (uint32_t k = 0; k < im->import_count; ++k) {
            uint32_t e = im->import_table + 24u * k;
            const struct wax_image *provider = &launch->image[im->dep_image[word(im, e)]];
            uint32_t x = find_export(provider, str(im, word(im, e + 4)));
            uint32_t slot = im->origin + word(im, e + 12);
            put32(bias, slot, provider->origin + word(provider, x + 8));
            put32(bias, slot + 4u, provider->origin);
        }
    }
}
