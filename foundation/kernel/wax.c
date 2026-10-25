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
#define FLAG_RELOCATABLE 4u
#define RELOC_PAIR 1u
#define OP_LDIH 0x3Du
#define OP_LDIL 0x3Cu
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
    if (rd16(d + 8) != 1u || type < TYPE_WAX || type > TYPE_WAL || (flags & ~7u) || word(im, 20) != 1u)
        return WAX_E_UNSUPPORTED; /* arch, type, flags, call_abi */
    if (!(flags & FLAG_PROTECTED) || word(im, 12) != HEADER_V3 || word(im, 16) != size)
        return WAX_E_FORMAT;
    for (uint32_t offset = 140; offset < HEADER_V3; offset += 4u) {
        if (word(im, offset))
            return WAX_E_FORMAT;
    }
    /* §3.1: the relocation table and heap hint belong to relocatable executables. */
    im->relocatable = (flags & FLAG_RELOCATABLE) != 0;
    im->reloc_table = word(im, 128);
    im->reloc_count = word(im, 132);
    im->heap_hint = word(im, 136);
    if (im->relocatable ? type != TYPE_WAX || !im->reloc_count || (im->heap_hint & 7u)
                        : im->reloc_table || im->reloc_count || im->heap_hint)
        return WAX_E_FORMAT;
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
        !add_table(&spans, im->reloc_table, im->reloc_count, 4u, size) ||
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
        if (im->relocatable && top != im->base)
            return WAX_E_FORMAT; /* §3.1: the stack lies just below the image */
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

    /* Relocations (§3.1): ascending, each a whole data word or an LDIH/LDIL
     * pair in initialized bytes of a block that may hold it. */
    uint32_t previous_at = 0, k = 0, loaded = ~0u;
    struct block b = {0};
    for (uint32_t i = 0; i < im->reloc_count; ++i) {
        uint32_t entry = word(im, im->reloc_table + 4u * i), at = entry & ~3u, kind = entry & 3u;
        if (kind > RELOC_PAIR || (i && at <= previous_at) || at < im->base || at - im->base > 0xFFFFFFF0u)
            return WAX_E_FORMAT;
        previous_at = at;
        uint32_t len = kind == RELOC_PAIR ? 8u : 4u;
        /* Entries and blocks both ascend, so one cursor walks the blocks (a
         * search per entry costs thousands of entries a scan each). */
        while (k < im->block_count) {
            if (loaded != k) {
                get_block(im, k, &b);
                loaded = k;
            }
            if (at >= b.address && at - b.address >= b.file_size)
                ++k;
            else
                break;
        }
        if (k >= im->block_count || at < b.address || len > b.file_size - (at - b.address) ||
            (kind == RELOC_PAIR ? b.perm != PERM_RX : b.perm == PERM_RX))
            return WAX_E_FORMAT;
        if (kind == RELOC_PAIR) {
            const uint8_t *code = d + b.file_offset + (at - b.address);
            if (rd32(code) >> 26 != OP_LDIH || rd32(code + 4) >> 26 != OP_LDIL ||
                ((rd32(code) ^ rd32(code + 4)) & 0x03E00000u))
                return WAX_E_FORMAT;
        }
    }
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

/* A light read of the header and block table: the plan validates the image
 * properly, and this runs first on every spawn, so it only guards its own
 * reads. */
enum wax_error wax_inspect(const void *data, uint32_t size, struct wax_info *info) {
    const uint8_t *d = data;
    if (!d || size < HEADER_V2 || d[0] != 'W' || d[1] != 'A' || d[2] != 'X' || d[3] != 0)
        return WAX_E_FORMAT;
    uint32_t version = rd16(d + 4), base, stack_top, stack_size, blocks, count, entry_size, flags = rd16(d + 10);
    if (version == 2u) {
        base = rd32(d + 16);
        blocks = rd32(d + 24), count = rd32(d + 28), entry_size = rd32(d + 32);
        stack_top = rd32(d + 48), stack_size = rd32(d + 52);
        flags = 0;
    } else if (version == 3u && size >= HEADER_V3) {
        base = rd32(d + 24);
        stack_top = rd32(d + 32), stack_size = rd32(d + 36);
        blocks = rd32(d + 40), count = rd32(d + 44), entry_size = 24u;
        info->heap_hint = rd32(d + 136);
    } else {
        return version == 2u || version == 3u ? WAX_E_FORMAT : WAX_E_UNSUPPORTED;
    }
    uint32_t block_size = version == 2u ? 16u : 24u;
    if (entry_size != block_size || !count || count > WAX_MAX_BLOCKS || blocks > size ||
        count * block_size > size - blocks || stack_size > stack_top)
        return WAX_E_FORMAT;
    uint32_t end = base;
    for (uint32_t i = 0; i < count; ++i) {
        const uint8_t *e = d + blocks + i * block_size;
        uint32_t address = rd32(e), len = version == 2u ? rd32(e + 8) : rd32(e + 12);
        if (address < base || len > CPU_RESERVED - address)
            return WAX_E_FORMAT;
        if (address + len > end)
            end = address + len;
    }
    uint32_t stack_low = stack_top - stack_size;
    info->base = base;
    info->low = stack_low < base ? stack_low : base;
    info->high = stack_top > end ? stack_top : end;
    info->relocatable = (flags & FLAG_RELOCATABLE) != 0;
    info->stack_size = stack_size;
    if (version != 3u)
        info->heap_hint = 0;
    return WAX_OK;
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
    taken[n].start = exe->origin;
    taken[n++].end = exe->origin + exe->span;
    taken[n].start = exe->stack_top - exe->stack_size + r->launch->delta;
    taken[n++].end = exe->stack_top + r->launch->delta;
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
    launch->region[0].start = exe->stack_top - exe->stack_size + launch->delta;
    launch->region[0].end = exe->stack_top + launch->delta;
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
    launch->delta = 0;
    launch->heap_hint = exe->heap_hint;
    if (exe->relocatable && target->load_base) {
        /* §3.1: the stack moves with the image. */
        if (target->load_base < exe->stack_size || (target->load_base & 3u) ||
            exe->span > CPU_RESERVED - target->load_base)
            return WAX_E_FORMAT;
        exe->origin = target->load_base;
        launch->delta = target->load_base - exe->base;
    }
    launch->count = 1;
    r.launch = launch;
    r.libs = libs;
    r.lib_count = lib_count;
    r.target = target;
    r.ordered = 0;
    if ((error = resolve(&r, 0)) != WAX_OK || (error = check_imports(launch)) != WAX_OK ||
        (error = place(launch, &r)) != WAX_OK || (error = plan_regions(launch)) != WAX_OK)
        return error;
    launch->entry = exe->origin + exe->entry_offset;
    launch->stack_top = exe->stack_top + launch->delta;
    launch->stack_size = exe->stack_size;
    return WAX_OK;
}

enum wax_error wax_rebase(struct wax_launch *launch, uint32_t shift) {
    for (uint32_t i = 0; i < launch->count; ++i) {
        if (launch->image[i].align > 1u && (shift & (launch->image[i].align - 1u)))
            return WAX_E_FORMAT;
    }
    for (uint32_t i = 0; i < launch->count; ++i)
        launch->image[i].origin += shift;
    for (uint32_t i = 0; i < launch->region_count; ++i) {
        launch->region[i].start += shift;
        launch->region[i].end += shift;
    }
    launch->entry += shift;
    launch->stack_top += shift;
    launch->top += shift;
    launch->delta += shift;
    return WAX_OK;
}

/* Byte copies and fills, by words (eight per step) where the addresses are
 * 4-byte aligned: byte by byte, loading the boot modules took most of a
 * second. */
static void copy_bytes(volatile uint8_t *dst, const uint8_t *src, uint32_t n) {
    uint32_t at = 0;
    if ((((uintptr_t)dst | (uintptr_t)src) & 3u) == 0) {
        volatile uint32_t *d = (volatile uint32_t *)dst;
        const uint32_t *s = (const uint32_t *)src;
        for (; at + 32u <= n; at += 32u, d += 8, s += 8) {
            d[0] = s[0], d[1] = s[1], d[2] = s[2], d[3] = s[3];
            d[4] = s[4], d[5] = s[5], d[6] = s[6], d[7] = s[7];
        }
        for (; at + 4u <= n; at += 4u)
            *d++ = *s++;
    }
    for (; at < n; ++at)
        dst[at] = src[at];
}

static void zero_bytes(volatile uint8_t *dst, uint32_t n) {
    uint32_t at = 0;
    for (; at < n && ((uintptr_t)(dst + at) & 3u); ++at)
        dst[at] = 0;
    volatile uint32_t *d = (volatile uint32_t *)(dst + at);
    for (; at + 32u <= n; at += 32u, d += 8)
        d[0] = 0, d[1] = 0, d[2] = 0, d[3] = 0, d[4] = 0, d[5] = 0, d[6] = 0, d[7] = 0;
    for (; at + 4u <= n; at += 4u)
        *d++ = 0;
    for (; at < n; ++at)
        dst[at] = 0;
}

/* The kernel's own view: address a is at bias + a. */
static uint8_t *bias_map(void *context, uint32_t address, uint32_t *run) {
    *run = 0xFFFFFFFFu;
    return (uint8_t *)((uintptr_t)context + address);
}

/* Writes n bytes at the process address a through map, one contiguous run
 * at a time. */
static void put_bytes(wax_map_fn map, void *context, uint32_t address, const uint8_t *src, uint32_t n) {
    while (n) {
        uint32_t run;
        uint8_t *dst = map(context, address, &run);
        if (run > n)
            run = n;
        copy_bytes(dst, src, run);
        address += run;
        src += run;
        n -= run;
    }
}

static void put_zeros(wax_map_fn map, void *context, uint32_t address, uint32_t n) {
    while (n) {
        uint32_t run;
        uint8_t *dst = map(context, address, &run);
        if (run > n)
            run = n;
        zero_bytes(dst, run);
        address += run;
        n -= run;
    }
}

static void put32(wax_map_fn map, void *context, uint32_t address, uint32_t value) {
    uint8_t bytes[4] = {(uint8_t)(value >> 24), (uint8_t)(value >> 16), (uint8_t)(value >> 8), (uint8_t)value};
    put_bytes(map, context, address, bytes, 4u);
}

static uint32_t get32(wax_map_fn map, void *context, uint32_t address) {
    uint32_t run;
    return rd32(map(context, address, &run)); /* aligned: a run is at least 4 bytes */
}

/* The instruction word insn with its 16-bit immediate replaced. */
static __attribute__((noinline)) uint32_t imm_of(uint32_t insn, uint32_t imm) {
    return insn - (insn & 0xFFFFu) + (imm & 0xFFFFu);
}

/* §3.1: adds the load offset to every address the executable holds. */
static void relocate(const struct wax_launch *launch, wax_map_fn map, void *context) {
    const struct wax_image *exe = &launch->image[0];
    if (!exe->relocatable || launch->delta == 0)
        return;
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
    /* The kernel's own memory, and words in the byte order of the image: a
     * relocation is a few operations (this runs for thousands of them per
     * spawn, and the generated code does not keep values in registers). */
    if (map == bias_map && !((uintptr_t)exe->data & 3u)) {
        const uint32_t *table = (const uint32_t *)(exe->data + exe->reloc_table);
        uint32_t delta = launch->delta;
        for (uint32_t i = 0; i < exe->reloc_count; ++i) {
            uint32_t entry = table[i];
            volatile uint32_t *at = (volatile uint32_t *)((uintptr_t)context + (entry & ~3u) + delta);
            if (entry & RELOC_PAIR) {
                uint32_t value = (at[0] << 16) + (at[1] & 0xFFFFu) + delta;
                at[0] = (at[0] & 0xFFFF0000u) | value >> 16;
                at[1] = (at[1] & 0xFFFF0000u) | (value & 0xFFFFu);
            } else {
                *at += delta;
            }
        }
        return;
    }
#endif
    for (uint32_t i = 0; i < exe->reloc_count; ++i) {
        uint32_t entry = word(exe, exe->reloc_table + 4u * i), at = (entry & ~3u) + launch->delta;
        if (entry & RELOC_PAIR) {
            uint32_t high = get32(map, context, at), low = get32(map, context, at + 4u);
            uint32_t value = (high & 0xFFFFu) * 0x10000u + (low & 0xFFFFu) + launch->delta;
            put32(map, context, at, imm_of(high, value / 0x10000u));
            put32(map, context, at + 4u, imm_of(low, value));
        } else {
            put32(map, context, at, get32(map, context, at) + launch->delta);
        }
    }
}

void wax_load_mapped(const struct wax_launch *launch, wax_map_fn map, void *context) {
    for (uint32_t i = 0; i < launch->count; ++i) {
        const struct wax_image *im = &launch->image[i];
        for (uint32_t k = 0; k < im->block_count; ++k) {
            struct block b;
            get_block(im, k, &b);
            uint32_t address = im->origin + (b.address - im->base);
            put_bytes(map, context, address, im->data + b.file_offset, b.file_size);
            put_zeros(map, context, address + b.file_size, b.memory_size - b.file_size);
        }
    }
    put_zeros(map, context, launch->stack_top - launch->stack_size, launch->stack_size);
    relocate(launch, map, context);
    /* Import descriptors (§4.3): resolved address, provider origin. */
    for (uint32_t i = 0; i < launch->count; ++i) {
        const struct wax_image *im = &launch->image[i];
        for (uint32_t k = 0; k < im->import_count; ++k) {
            uint32_t e = im->import_table + 24u * k;
            const struct wax_image *provider = &launch->image[im->dep_image[word(im, e)]];
            uint32_t x = find_export(provider, str(im, word(im, e + 4)));
            uint32_t slot = im->origin + word(im, e + 12);
            put32(map, context, slot, provider->origin + word(provider, x + 8));
            put32(map, context, slot + 4u, provider->origin);
        }
    }
}

void wax_load(const struct wax_launch *launch, uintptr_t bias) {
    wax_load_mapped(launch, bias_map, (void *)bias);
}
