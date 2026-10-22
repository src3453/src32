/* FS2 volume driver (doc/spec_FS2.md). See fs2.h.
 *
 * Sectors go through a small write-through cache, so every write reaches
 * the device in the order of the calls below; that order is the one §4 and
 * §9 require (allocate and initialise, then publish; unpublish, then free).
 * There is no journal (§9.1): a crash between two writes can leave the
 * volume inconsistent, and DIRTY makes the next mount refuse it. */
#include "fs2.h"

#include <string.h>

#include "name.h"

/* KAN-38: boot/build.py temporarily disables the expensive full scan.
 * Keep standalone/host builds strict unless explicitly overridden. */
#ifndef FS2_FULL_MOUNT_CHECK
#define FS2_FULL_MOUNT_CHECK 1
#endif

/* ---- sectors ---------------------------------------------------------- */

/* Direct-mapped, so a lookup is one compare and a read-ahead run never
 * evicts itself. Metadata (below data_start) and data blocks map to
 * separate lines, so reading a file does not keep evicting its inode. */
static fs2_cache_line *cache_line(fs2_vol *v, uint32_t lba) {
    if (lba < v->data_start)
        return &v->cache[lba % FS2_CACHE_META];
    return &v->cache[FS2_CACHE_META + lba % (FS2_CACHE_SECTORS - FS2_CACHE_META)];
}

static fs2_cache_line *cache_find(fs2_vol *v, uint32_t lba) {
    fs2_cache_line *c = cache_line(v, lba);
    return c->valid && c->lba == lba ? c : 0;
}

/* Copies with words when both ends share their alignment. The toolchain's
 * memcpy and memset go byte by byte, and sector and payload copies are most
 * of the CPU time of fs. no_builtin keeps clang from turning the loops back
 * into library calls; the loops are unrolled because the backend spends
 * more on the loop than on a word. */
#ifdef __clang__
__attribute__((no_builtin("memcpy")))
#endif
void fs_copy(void *dst, const void *src, uint32_t n) {
    uint8_t *d = (uint8_t *)dst;
    const uint8_t *s = (const uint8_t *)src;
    if (!(((uintptr_t)d ^ (uintptr_t)s) & 3u)) {
        for (; n && ((uintptr_t)d & 3u); --n)
            *d++ = *s++;
        uint32_t *dw = (uint32_t *)(void *)d;
        const uint32_t *sw = (const uint32_t *)(const void *)s;
        for (; n >= 32; n -= 32, dw += 8, sw += 8) {
            dw[0] = sw[0], dw[1] = sw[1], dw[2] = sw[2], dw[3] = sw[3];
            dw[4] = sw[4], dw[5] = sw[5], dw[6] = sw[6], dw[7] = sw[7];
        }
        for (; n >= 4; n -= 4)
            *dw++ = *sw++;
        d = (uint8_t *)dw;
        s = (const uint8_t *)sw;
    }
    while (n--)
        *d++ = *s++;
}

#ifdef __clang__
__attribute__((no_builtin("memset")))
#endif
void fs_zero(void *dst, uint32_t n) {
    uint8_t *d = (uint8_t *)dst;
    for (; n && ((uintptr_t)d & 3u); --n)
        *d++ = 0;
    uint32_t *w = (uint32_t *)(void *)d;
    for (; n >= 32; n -= 32, w += 8)
        w[0] = w[1] = w[2] = w[3] = w[4] = w[5] = w[6] = w[7] = 0;
    for (; n >= 4; n -= 4)
        *w++ = 0;
    d = (uint8_t *)w;
    while (n--)
        *d++ = 0;
}

/* The cached sector lba, valid until the next sec_* call. A miss reads up
 * to FS2_READ_AHEAD sectors from lba, stopping at the end of the device or
 * at a sector already cached (the cache is write-through, so a cached
 * sector equals the media). */
static fnd_status sec_get(fs2_vol *v, uint32_t lba, const uint8_t **data) {
    fs2_cache_line *c = cache_find(v, lba);
    if (!c && v->scan) {
        /* A long scan reads the media through the read-ahead buffer only, so
         * it does not push the lines other requests rely on out of the
         * 24-line data cache. */
        if (lba - v->scan_lba >= v->scan_n) {
            if (lba >= v->dev->sectors)
                return FND_E_IO;
            uint32_t n = v->dev->sectors - lba < FS2_READ_AHEAD ? v->dev->sectors - lba : FS2_READ_AHEAD;
            v->scan_n = 0;
            fnd_status st = v->dev->read(v->dev->ctx, lba, n, v->ahead);
            if (st != FND_OK)
                return st;
            v->scan_lba = lba, v->scan_n = n;
        }
        *data = v->ahead + (lba - v->scan_lba) * FS2_BLOCK;
        return FND_OK;
    }
    if (!c) {
        if (lba >= v->dev->sectors)
            return FND_E_IO;
        v->scan_n = 0; /* the miss below reuses v->ahead */
        uint32_t n = 1;
        while (n < FS2_READ_AHEAD && lba + n < v->dev->sectors && !cache_find(v, lba + n))
            ++n;
        fnd_status st = v->dev->read(v->dev->ctx, lba, n, v->ahead);
        if (st != FND_OK)
            return st;
        for (uint32_t i = 0; i < n; ++i) {
            fs2_cache_line *line = cache_line(v, lba + i);
            fs_copy(line->data, v->ahead + i * FS2_BLOCK, FS2_BLOCK);
            line->lba = lba + i;
            line->valid = 1;
        }
        c = cache_line(v, lba);
    }
    *data = c->data;
    return FND_OK;
}

static fnd_status sec_read(fs2_vol *v, uint32_t lba, uint8_t *buf) {
    const uint8_t *data;
    fnd_status st = sec_get(v, lba, &data);
    if (st == FND_OK)
        fs_copy(buf, data, FS2_BLOCK);
    return st;
}

static fnd_status sec_write(fs2_vol *v, uint32_t lba, const uint8_t *buf) {
    if (v->failed)
        return FND_E_IO;
    fs2_cache_line *c = cache_find(v, lba);
    v->scan_n = 0;
    fnd_status st = v->dev->write(v->dev->ctx, lba, 1, buf);
    if (st != FND_OK) {
        /* §9.1: keep DIRTY and stop new updates after an I/O failure. */
        v->failed = 1;
        if (c)
            c->valid = 0;
        return st;
    }
    if (!c)
        c = cache_line(v, lba);
    fs_copy(c->data, buf, FS2_BLOCK);
    c->lba = lba;
    c->valid = 1;
    return FND_OK;
}

/* Sets DIRTY on the media before the first change of a read-write mount
 * (§9.1). Done lazily so that a volume only read since boot stays clean. */
static void dir_index_drop(fs2_vol *v) {
    for (uint32_t i = 0; i < FS2_DIRIDX; ++i) {
        fs_free(v->dir_index[i].key);
        v->dir_index[i].key = 0;
    }
}

static fnd_status begin_change(fs2_vol *v) {
    dir_index_drop(v);
    uint8_t sb[FS2_BLOCK] FS_ALIGNED;
    if (!v->writable)
        return FND_E_READ_ONLY;
    if (v->failed)
        return FND_E_IO;
    if (v->dirty)
        return FND_OK;
    fnd_status st = sec_read(v, 1, sb);
    if (st != FND_OK)
        return st;
    fs_put16(sb + 6, fs_be16(sb + 6) | FS2_SB_DIRTY);
    st = sec_write(v, 1, sb);
    if (st == FND_OK)
        v->dirty = 1;
    return st;
}

/* ---- bitmap ----------------------------------------------------------- */

static int bit_get(const uint8_t *map, uint32_t i) { return (map[i >> 3] >> (7 - (i & 7))) & 1; }
static void bit_set(uint8_t *map, uint32_t i, int on) {
    uint8_t m = (uint8_t)(0x80u >> (i & 7));
    if (on)
        map[i >> 3] |= m;
    else
        map[i >> 3] &= (uint8_t)~m;
}

static fnd_status bitmap_flush(fs2_vol *v, uint32_t block) {
    uint32_t sector = (block >> 3) / FS2_BLOCK;
    return sec_write(v, v->bitmap_start + sector, v->bitmap + sector * FS2_BLOCK);
}

/* First fit (§4). The bit is persisted before the caller links the block. */
static fnd_status block_alloc(fs2_vol *v, uint32_t *out) {
    if (v->free_blocks == 0)
        return FND_E_NO_SPACE;
    for (uint32_t b = v->data_start; b < v->total_blocks; ++b) {
        if (!bit_get(v->bitmap, b)) {
            bit_set(v->bitmap, b, 1);
            fnd_status st = bitmap_flush(v, b);
            if (st != FND_OK) {
                bit_set(v->bitmap, b, 0);
                return st;
            }
            --v->free_blocks;
            *out = b;
            return FND_OK;
        }
    }
    return FND_E_NO_SPACE;
}

static fnd_status block_free(fs2_vol *v, uint32_t b) {
    bit_set(v->bitmap, b, 0);
    ++v->free_blocks;
    return bitmap_flush(v, b);
}

/* ---- inodes ----------------------------------------------------------- */

static void time_get(const uint8_t *p, fs_time *t) {
    t->sec = (int64_t)fs_be64(p);
    t->ms = fs_be16(p + 8);
}
static void time_put(uint8_t *p, fs_time t) {
    fs_put64(p, (uint64_t)t.sec);
    fs_put16(p + 8, t.ms);
}

static void inode_decode(const uint8_t *p, fs2_inode *n) {
    n->type = p[0];
    n->mode = fs_be16(p + 2);
    n->uid = fs_be32(p + 4);
    n->parent = fs_be32(p + 8);
    n->first = fs_be32(p + 12);
    n->size = fs_be64(p + 16);
    n->count = fs_be32(p + 24);
    time_get(p + 0x20, &n->created);
    time_get(p + 0x2C, &n->modified);
    time_get(p + 0x38, &n->changed);
}

static void inode_encode(uint8_t *p, const fs2_inode *n) {
    memset(p, 0, FS2_INODE_BYTES);
    if (n->type == FS2_TYPE_FREE)
        return;
    p[0] = (uint8_t)n->type;
    fs_put16(p + 2, n->mode);
    fs_put32(p + 4, n->uid);
    fs_put32(p + 8, n->parent);
    fs_put32(p + 12, n->first);
    fs_put64(p + 16, n->size);
    fs_put32(p + 24, n->count);
    time_put(p + 0x20, n->created);
    time_put(p + 0x2C, n->modified);
    time_put(p + 0x38, n->changed);
}

static uint32_t inode_lba(fs2_vol *v, uint32_t ino) { return v->inode_start + (ino - 1) / 4; }
static uint32_t inode_off(uint32_t ino) { return ((ino - 1) % 4) * FS2_INODE_BYTES; }

fnd_status fs2_inode_read(fs2_vol *v, uint32_t ino, fs2_inode *out) {
    const uint8_t *buf;
    if (ino == 0 || ino > v->inode_count)
        return FND_E_NOT_FOUND;
    fnd_status st = sec_get(v, inode_lba(v, ino), &buf);
    if (st != FND_OK)
        return st;
    inode_decode(buf + inode_off(ino), out);
    return FND_OK;
}

static fnd_status inode_write(fs2_vol *v, uint32_t ino, const fs2_inode *n) {
    uint8_t buf[FS2_BLOCK] FS_ALIGNED;
    fnd_status st = sec_read(v, inode_lba(v, ino), buf);
    if (st != FND_OK)
        return st;
    inode_encode(buf + inode_off(ino), n);
    return sec_write(v, inode_lba(v, ino), buf);
}

/* ---- block chains ----------------------------------------------------- */

static void hint_drop(fs2_vol *v, uint32_t ino) {
    for (uint32_t i = 0; i < FS2_HINTS; ++i)
        if (v->hints[i].ino == ino)
            v->hints[i].ino = 0;
}

static void hint_note(fs2_vol *v, uint32_t ino, uint32_t index, uint32_t lba) {
    fs2_hint *slot = &v->hints[0];
    for (uint32_t i = 0; i < FS2_HINTS; ++i) {
        if (v->hints[i].ino == ino) {
            slot = &v->hints[i];
            break;
        }
        if (v->hints[i].age < slot->age)
            slot = &v->hints[i];
    }
    slot->ino = ino, slot->index = index, slot->lba = lba, slot->age = ++v->clock;
}

static uint32_t next_of(const uint8_t *block) { return fs_be32(block); }

/* Finds block `index` (< count) of the inode's chain. */
static fnd_status chain_seek(fs2_vol *v, uint32_t ino, const fs2_inode *n, uint32_t index, uint32_t *lba) {
    uint32_t at = 0, b = n->first;
    const uint8_t *buf;
    for (uint32_t i = 0; i < FS2_HINTS; ++i) {
        fs2_hint *h = &v->hints[i];
        if (h->ino == ino && h->index <= index && h->index >= at) {
            at = h->index;
            b = h->lba;
        }
    }
    while (at < index) {
        fnd_status st = sec_get(v, b, &buf);
        if (st != FND_OK)
            return st;
        b = next_of(buf);
        if (b < v->data_start || b >= v->total_blocks)
            return FND_E_IO; /* also needed when the full mount scan is off */
        ++at;
    }
    hint_note(v, ino, index, b);
    *lba = b;
    return FND_OK;
}

static uint32_t blocks_for(uint64_t size) { return (uint32_t)((size + FS2_PAYLOAD - 1) / FS2_PAYLOAD); }

/* fs2_read with the inode already decoded (the caller read it for ino). */
static fnd_status read_with(fs2_vol *v, uint32_t ino, const fs2_inode *n, uint64_t off, void *out, uint32_t len,
                            uint32_t *got) {
    const uint8_t *buf;
    fnd_status st;
    *got = 0;
    if (off >= n->size || len == 0)
        return FND_OK;
    if ((uint64_t)len > n->size - off)
        len = (uint32_t)(n->size - off);
    /* Files stay far below 4 GiB, so the 32-bit division is the usual case;
     * the 64-bit one is a software loop on this CPU. */
    uint32_t index, in, lba, done = 0;
    if ((off >> 32) == 0) {
        uint32_t o = (uint32_t)off;
        index = o / FS2_PAYLOAD, in = o % FS2_PAYLOAD;
    } else {
        index = (uint32_t)(off / FS2_PAYLOAD), in = (uint32_t)(off % FS2_PAYLOAD);
    }
    st = chain_seek(v, ino, n, index, &lba);
    while (st == FND_OK) {
        st = sec_get(v, lba, &buf);
        if (st != FND_OK)
            break;
        uint32_t take = FS2_PAYLOAD - in;
        if (take > len - done)
            take = len - done;
        fs_copy((uint8_t *)out + done, buf + 4 + in, take);
        done += take;
        in = 0;
        if (done == len)
            break;
        lba = next_of(buf);
        if (lba < v->data_start || lba >= v->total_blocks)
            return FND_E_IO;
        hint_note(v, ino, ++index, lba);
    }
    if (st == FND_OK)
        *got = done;
    return st;
}

fnd_status fs2_read(fs2_vol *v, uint32_t ino, uint64_t off, void *out, uint32_t len, uint32_t *got) {
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, ino, &n);
    if (st != FND_OK)
        return st;
    return read_with(v, ino, &n, off, out, len, got);
}

/* Appends `extra` new blocks to the chain. Their payload is zero except for
 * the part of [src_off, src_off + src_len) of the file that falls in them,
 * taken from `src`. The new blocks are fully written before the old tail
 * (or first_block) is linked to them; the caller then writes the inode. */
static fnd_status chain_extend(fs2_vol *v, uint32_t ino, fs2_inode *n, uint32_t extra, uint64_t src_off,
                               const uint8_t *src, uint32_t src_len) {
    uint8_t buf[FS2_BLOCK] FS_ALIGNED;
    uint32_t head = 0, prev = 0, cur, index = n->count;
    fnd_status st;
    if (extra > v->free_blocks)
        return FND_E_NO_SPACE;
    st = block_alloc(v, &cur);
    if (st != FND_OK)
        return st;
    head = cur;
    for (uint32_t k = 0; k < extra; ++k, ++index) {
        uint32_t next = 0;
        if (k + 1 < extra) {
            st = block_alloc(v, &next);
            if (st != FND_OK)
                return st; /* allocated bits leak until fsck (§9.1) */
        }
        fs_zero(buf, sizeof buf);
        fs_put32(buf, next);
        uint64_t start = (uint64_t)index * FS2_PAYLOAD, end = start + FS2_PAYLOAD;
        uint64_t s0 = src_off, s1 = src_off + src_len;
        if (src && s0 < end && s1 > start) {
            uint64_t a = s0 > start ? s0 : start, b = s1 < end ? s1 : end;
            fs_copy(buf + 4 + (a - start), src + (a - s0), (uint32_t)(b - a));
        }
        st = sec_write(v, cur, buf);
        if (st != FND_OK)
            return st;
        prev = cur;
        cur = next;
    }
    (void)prev;
    if (n->count == 0) {
        n->first = head;
    } else {
        uint32_t tail;
        st = chain_seek(v, ino, n, n->count - 1, &tail);
        if (st == FND_OK)
            st = sec_read(v, tail, buf);
        if (st != FND_OK)
            return st;
        fs_put32(buf, head);
        st = sec_write(v, tail, buf);
        if (st != FND_OK)
            return st;
    }
    n->count += extra;
    return FND_OK;
}

static fnd_status chain_free_from(fs2_vol *v, uint32_t b, uint32_t count) {
    uint8_t buf[FS2_BLOCK] FS_ALIGNED;
    while (count--) {
        fnd_status st = sec_read(v, b, buf);
        if (st == FND_OK)
            st = block_free(v, b);
        if (st != FND_OK)
            return st;
        b = next_of(buf);
    }
    return FND_OK;
}

/* Writes into blocks that already exist. */
static fnd_status write_existing(fs2_vol *v, uint32_t ino, const fs2_inode *n, uint64_t off,
                                 const uint8_t *src, uint32_t len) {
    uint8_t buf[FS2_BLOCK] FS_ALIGNED;
    uint64_t limit = (uint64_t)n->count * FS2_PAYLOAD;
    if (off >= limit || len == 0)
        return FND_OK;
    if ((uint64_t)len > limit - off)
        len = (uint32_t)(limit - off);
    uint32_t index = (uint32_t)(off / FS2_PAYLOAD), in = (uint32_t)(off % FS2_PAYLOAD), lba, done = 0;
    fnd_status st = chain_seek(v, ino, n, index, &lba);
    while (st == FND_OK && done < len) {
        st = sec_read(v, lba, buf);
        if (st != FND_OK)
            break;
        uint32_t take = FS2_PAYLOAD - in;
        if (take > len - done)
            take = len - done;
        fs_copy(buf + 4 + in, src + done, take);
        st = sec_write(v, lba, buf);
        done += take;
        in = 0;
        if (st == FND_OK && done < len) {
            lba = next_of(buf);
            hint_note(v, ino, ++index, lba);
        }
    }
    return st;
}

static void touch(fs2_inode *n, fs_time now, int modified) {
    if (modified)
        n->modified = now;
    n->changed = now;
}

static fnd_status file_write(fs2_vol *v, uint32_t ino, fs2_inode *n, uint64_t off, const uint8_t *src,
                             uint32_t len, fs_time now) {
    uint64_t end = off + len;
    if (end < off)
        return FND_E_BAD_ARG;
    uint64_t new_size = end > n->size ? end : n->size;
    uint64_t need = (new_size + FS2_PAYLOAD - 1) / FS2_PAYLOAD;
    if (need > 0xFFFFFFFFu)
        return FND_E_NO_SPACE;
    fnd_status st = begin_change(v);
    if (st != FND_OK)
        return st;
    uint32_t old_count = n->count;
    if ((uint32_t)need > n->count) {
        st = chain_extend(v, ino, n, (uint32_t)need - n->count, off, src, len);
        if (st != FND_OK)
            return st;
    }
    /* The part of the write that falls in the blocks that already existed. */
    fs2_inode old = *n;
    old.count = old_count;
    st = write_existing(v, ino, &old, off, src, len);
    if (st != FND_OK)
        return st;
    n->size = new_size;
    touch(n, now, 1);
    return inode_write(v, ino, n);
}

fnd_status fs2_write(fs2_vol *v, uint32_t ino, uint64_t off, const void *buf, uint32_t len, fs_time now) {
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, ino, &n);
    if (st != FND_OK)
        return st;
    if (n.type != FS2_TYPE_FILE)
        return FND_E_IS_DIR;
    if (len == 0)
        return FND_OK;
    return file_write(v, ino, &n, off, (const uint8_t *)buf, len, now);
}

static fnd_status file_truncate(fs2_vol *v, uint32_t ino, fs2_inode *n, uint64_t size, fs_time now) {
    uint8_t buf[FS2_BLOCK] FS_ALIGNED;
    fnd_status st = begin_change(v);
    if (st != FND_OK)
        return st;
    if (size > n->size) {
        uint64_t need = (size + FS2_PAYLOAD - 1) / FS2_PAYLOAD;
        if (need > 0xFFFFFFFFu)
            return FND_E_NO_SPACE;
        /* The old tail's unused payload is already 0 (§4). */
        if ((uint32_t)need > n->count) {
            st = chain_extend(v, ino, n, (uint32_t)need - n->count, 0, 0, 0);
            if (st != FND_OK)
                return st;
        }
        n->size = size;
        touch(n, now, 1);
        return inode_write(v, ino, n);
    }
    uint32_t count = blocks_for(size), freed_head = 0, freed = n->count - count;
    if (count == 0) {
        freed_head = n->first;
        n->first = 0;
    } else {
        uint32_t tail;
        st = chain_seek(v, ino, n, count - 1, &tail);
        if (st == FND_OK)
            st = sec_read(v, tail, buf);
        if (st != FND_OK)
            return st;
        freed_head = next_of(buf);
        uint32_t used = (uint32_t)(size - (uint64_t)(count - 1) * FS2_PAYLOAD);
        memset(buf + 4 + used, 0, FS2_PAYLOAD - used);
        fs_put32(buf, 0);
        st = sec_write(v, tail, buf);
        if (st != FND_OK)
            return st;
    }
    n->count = count;
    n->size = size;
    touch(n, now, 1);
    hint_drop(v, ino);
    st = inode_write(v, ino, n);
    if (st == FND_OK && freed)
        st = chain_free_from(v, freed_head, freed);
    return st;
}

fnd_status fs2_truncate(fs2_vol *v, uint32_t ino, uint64_t size, fs_time now) {
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, ino, &n);
    if (st != FND_OK)
        return st;
    if (n.type != FS2_TYPE_FILE)
        return FND_E_IS_DIR;
    if (size == n.size)
        return FND_OK;
    return file_truncate(v, ino, &n, size, now);
}

/* ---- directories ------------------------------------------------------ */

static fnd_status raw_read(fs2_vol *v, uint32_t ino, uint64_t off, uint8_t *buf, uint32_t len) {
    uint32_t got;
    fnd_status st = fs2_read(v, ino, off, buf, len, &got);
    if (st == FND_OK && got != len)
        st = FND_E_IO;
    return st;
}

static fnd_status raw_read_with(fs2_vol *v, uint32_t ino, const fs2_inode *n, uint64_t off, uint8_t *buf,
                                uint32_t len) {
    uint32_t got;
    fnd_status st = read_with(v, ino, n, off, buf, len, &got);
    if (st == FND_OK && got != len)
        st = FND_E_IO;
    return st;
}

/* The directory inode n is already decoded. Entries are read as the 8-byte
 * head plus the first 32 name bytes (nearly every name), then the rest of a
 * longer name. */
#define DIRENT_FIRST 32u
static fnd_status dirent_read(fs2_vol *v, uint32_t dir, const fs2_inode *n, uint32_t slot, fs2_dirent *d) {
    uint8_t head[8 + DIRENT_FIRST];
    uint64_t off = (uint64_t)slot * FS2_DIRENT;
    fnd_status st = raw_read_with(v, dir, n, off, head, sizeof head);
    if (st != FND_OK)
        return st;
    d->ino = fs_be32(head);
    d->name_bytes = fs_be16(head + 4);
    d->name_chars = fs_be16(head + 6);
    if (d->ino == 0)
        return FND_OK;
    if (d->name_bytes == 0 || d->name_bytes > FS2_NAME_MAX_BYTES)
        return FND_E_IO;
    uint32_t first = d->name_bytes < DIRENT_FIRST ? d->name_bytes : DIRENT_FIRST;
    fs_copy(d->name, head + 8, first);
    if (d->name_bytes == first)
        return FND_OK;
    return raw_read_with(v, dir, n, off + 8 + first, d->name + first, d->name_bytes - first);
}

static uint32_t dir_slots(const fs2_inode *n) { return (uint32_t)(n->size / FS2_DIRENT); }

fnd_status fs2_dir_next(fs2_vol *v, uint32_t dir, uint32_t *slot, fs2_dirent *out) {
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, dir, &n);
    if (st != FND_OK)
        return st;
    if (n.type != FS2_TYPE_DIR)
        return FND_E_NOT_DIR;
    for (uint32_t s = *slot; s < dir_slots(&n); ++s) {
        st = dirent_read(v, dir, &n, s, out);
        if (st != FND_OK)
            return st;
        if (out->ino) {
            *slot = s + 1;
            return FND_OK;
        }
    }
    *slot = dir_slots(&n);
    return FND_END;
}

/* Builds (or returns) the index of `dir`; 0 with *st set on an error. */
static fs2_dir_index *dir_index_get(fs2_vol *v, uint32_t dir, const fs2_inode *n, fnd_status *st) {
    fs2_dir_index *x = 0, *victim = &v->dir_index[0];
    *st = FND_OK;
    for (uint32_t i = 0; i < FS2_DIRIDX && !x; ++i) {
        fs2_dir_index *c = &v->dir_index[i];
        if (c->key && c->dir == dir)
            x = c;
        else if (!c->key || (victim->key && c->age < victim->age))
            victim = c;
    }
    if (!x) {
        fs2_dirent d;
        uint32_t slots = dir_slots(n);
        fs_free(victim->key);
        victim->key = (uint32_t *)fs_alloc((2 * slots + 2) * 4u);
        if (!victim->key) {
            *st = FND_E_NO_MEMORY;
            return 0;
        }
        x = victim;
        x->dir = dir, x->count = 0, x->free_slot = slots;
        v->scan = 1, v->scan_n = 0;
        for (uint32_t s = 0; s < slots; ++s) {
            *st = dirent_read(v, dir, n, s, &d);
            if (*st != FND_OK) {
                v->scan = 0;
                fs_free(x->key);
                x->key = 0;
                return 0;
            }
            if (!d.ino) {
                if (x->free_slot == slots)
                    x->free_slot = s;
            } else {
                x->key[2 * x->count] = fnd_name_key_hash(d.name, d.name_bytes);
                x->key[2 * x->count + 1] = s;
                ++x->count;
            }
        }
        v->scan = 0;
    }
    x->age = ++v->clock;
    return x;
}

/* Finds `name` (by folded key) in `dir`. FND_E_NOT_FOUND if absent; *slot
 * is then the first free slot, or the slot count if there is none. */
static fnd_status dir_find(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t *slot,
                           fs2_dirent *d) {
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, dir, &n);
    if (st != FND_OK)
        return st;
    if (n.type != FS2_TYPE_DIR)
        return FND_E_NOT_DIR;
    fs2_dir_index *x = dir_index_get(v, dir, &n, &st);
    if (!x)
        return st;
    uint32_t h = fnd_name_key_hash(name, len);
    for (uint32_t k = 0; k < x->count; ++k) {
        if (x->key[2 * k] != h)
            continue;
        st = dirent_read(v, dir, &n, x->key[2 * k + 1], d);
        if (st != FND_OK)
            return st;
        if (d->ino && fnd_name_key_equal(d->name, d->name_bytes, name, len)) {
            *slot = x->key[2 * k + 1];
            return FND_OK;
        }
    }
    *slot = x->free_slot;
    return FND_E_NOT_FOUND;
}

fnd_status fs2_lookup(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t *ino,
                      fs2_dirent *stored) {
    fs2_dirent d;
    uint32_t slot;
    fnd_status st = dir_find(v, dir, name, len, &slot, stored ? stored : &d);
    if (st == FND_OK)
        *ino = (stored ? stored : &d)->ino;
    return st;
}

fnd_status fs2_dir_empty(fs2_vol *v, uint32_t dir) {
    fs2_dirent d;
    uint32_t slot = 0;
    fnd_status st = fs2_dir_next(v, dir, &slot, &d);
    if (st == FND_END)
        return FND_OK;
    return st == FND_OK ? FND_E_NOT_EMPTY : st;
}

/* Writes one entry. A new slot at the end grows the directory by 1040
 * bytes; the caller has already called begin_change. */
static fnd_status dirent_write(fs2_vol *v, uint32_t dir, uint32_t slot, uint32_t ino, const uint8_t *name,
                               uint32_t len, uint32_t chars, fs_time now) {
    static uint8_t e[FS2_DIRENT] FS_ALIGNED;
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, dir, &n);
    if (st != FND_OK)
        return st;
    memset(e, 0, sizeof e);
    if (ino) {
        fs_put32(e, ino);
        fs_put16(e + 4, len);
        fs_put16(e + 6, chars);
        memcpy(e + 8, name, len);
    }
    return file_write(v, dir, &n, (uint64_t)slot * FS2_DIRENT, e, FS2_DIRENT, now);
}

fnd_status fs2_create(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t type,
                      uint32_t mode, uint32_t uid, fs_time now, uint32_t *ino) {
    fs2_dirent d;
    uint32_t slot, chars, num = 0;
    fnd_status st = fnd_name_check(name, len, &chars);
    if (st != FND_OK)
        return st;
    st = dir_find(v, dir, name, len, &slot, &d);
    if (st == FND_OK)
        return FND_E_EXISTS;
    if (st != FND_E_NOT_FOUND)
        return st;
    for (uint32_t i = 1; i <= v->inode_count; ++i) {
        if (!bit_get(v->inode_used, i)) {
            num = i;
            break;
        }
    }
    if (!num)
        return FND_E_LIMIT;
    /* §12 contract example: a new entry may need blocks for the directory. */
    fs2_inode pn;
    st = fs2_inode_read(v, dir, &pn);
    if (st != FND_OK)
        return st;
    if ((uint64_t)slot * FS2_DIRENT + FS2_DIRENT > (uint64_t)pn.count * FS2_PAYLOAD) {
        uint32_t need = blocks_for((uint64_t)slot * FS2_DIRENT + FS2_DIRENT) - pn.count;
        if (need > v->free_blocks)
            return FND_E_NO_SPACE;
    }
    st = begin_change(v);
    if (st != FND_OK)
        return st;
    fs2_inode n;
    memset(&n, 0, sizeof n);
    n.type = type;
    n.mode = mode;
    n.uid = uid;
    n.parent = dir;
    n.created = n.modified = n.changed = now;
    st = inode_write(v, num, &n);
    if (st != FND_OK)
        return st;
    bit_set(v->inode_used, num, 1);
    st = dirent_write(v, dir, slot, num, name, len, chars, now);
    if (st != FND_OK) {
        /* Unpublished: free the inode again. */
        memset(&n, 0, sizeof n);
        if (inode_write(v, num, &n) == FND_OK)
            bit_set(v->inode_used, num, 0);
        return st;
    }
    *ino = num;
    return FND_OK;
}

fnd_status fs2_remove(fs2_vol *v, uint32_t dir, const uint8_t *name, uint32_t len, uint32_t want_dir,
                      fs_time now) {
    fs2_dirent d;
    fs2_inode n;
    uint32_t slot;
    fnd_status st = dir_find(v, dir, name, len, &slot, &d);
    if (st != FND_OK)
        return st;
    st = fs2_inode_read(v, d.ino, &n);
    if (st != FND_OK)
        return st;
    if (want_dir && n.type != FS2_TYPE_DIR)
        return FND_E_NOT_DIR;
    if (!want_dir && n.type == FS2_TYPE_DIR)
        return FND_E_IS_DIR;
    if (want_dir) {
        st = fs2_dir_empty(v, d.ino);
        if (st != FND_OK)
            return st;
    }
    st = begin_change(v);
    if (st != FND_OK)
        return st;
    /* §9 unlink/rmdir: entry, then inode, then the data blocks. */
    st = dirent_write(v, dir, slot, 0, 0, 0, 0, now);
    if (st != FND_OK)
        return st;
    fs2_inode empty;
    memset(&empty, 0, sizeof empty);
    st = inode_write(v, d.ino, &empty);
    if (st != FND_OK)
        return st;
    bit_set(v->inode_used, d.ino, 0);
    hint_drop(v, d.ino);
    return chain_free_from(v, n.first, n.count);
}

fnd_status fs2_is_under(fs2_vol *v, uint32_t ino, uint32_t anc, uint32_t *result) {
    for (uint32_t steps = 0; steps <= v->inode_count; ++steps) {
        if (ino == anc) {
            *result = 1;
            return FND_OK;
        }
        if (ino == FS2_ROOT)
            break;
        fs2_inode n;
        fnd_status st = fs2_inode_read(v, ino, &n);
        if (st != FND_OK)
            return st;
        ino = n.parent;
    }
    *result = 0;
    return FND_OK;
}

fnd_status fs2_rename(fs2_vol *v, uint32_t src_dir, const uint8_t *src, uint32_t src_len, uint32_t dst_dir,
                      const uint8_t *dst, uint32_t dst_len, fs_time now) {
    fs2_dirent from, to;
    fs2_inode n;
    uint32_t src_slot, dst_slot, chars, under;
    fnd_status st = fnd_name_check(dst, dst_len, &chars);
    if (st != FND_OK)
        return st;
    st = dir_find(v, src_dir, src, src_len, &src_slot, &from);
    if (st != FND_OK)
        return st;
    st = dir_find(v, dst_dir, dst, dst_len, &dst_slot, &to);
    if (st == FND_OK) {
        if (src_dir != dst_dir || dst_slot != src_slot)
            return FND_E_EXISTS;
        /* The same entry: same stored name is a no-op, a case change is a
         * rename onto itself (§6, §9). */
        if (to.name_bytes == dst_len && memcmp(to.name, dst, dst_len) == 0)
            return FND_OK;
    } else if (st != FND_E_NOT_FOUND) {
        return st;
    }
    st = fs2_inode_read(v, from.ino, &n);
    if (st != FND_OK)
        return st;
    if (n.type == FS2_TYPE_DIR) {
        st = fs2_is_under(v, dst_dir, from.ino, &under);
        if (st != FND_OK)
            return st;
        if (under)
            return FND_E_BAD_ARG;
    }
    st = begin_change(v);
    if (st != FND_OK)
        return st;
    if (src_dir == dst_dir) {
        st = dirent_write(v, src_dir, src_slot, from.ino, dst, dst_len, chars, now);
    } else {
        st = dirent_write(v, dst_dir, dst_slot, from.ino, dst, dst_len, chars, now);
        if (st == FND_OK) {
            n.parent = dst_dir;
            touch(&n, now, 0);
            st = inode_write(v, from.ino, &n);
        }
        if (st == FND_OK)
            st = dirent_write(v, src_dir, src_slot, 0, 0, 0, 0, now);
        return st;
    }
    if (st == FND_OK) {
        touch(&n, now, 0);
        st = inode_write(v, from.ino, &n);
    }
    return st;
}

fnd_status fs2_set_owner(fs2_vol *v, uint32_t ino, uint32_t set_mode, uint32_t mode, uint32_t set_uid,
                         uint32_t uid, fs_time now) {
    fs2_inode n;
    fnd_status st = fs2_inode_read(v, ino, &n);
    if (st != FND_OK)
        return st;
    if (set_mode && (mode & ~FS2_MODE_MASK))
        return FND_E_BAD_ARG;
    if (set_uid && uid == 0xFFFFFFFFu)
        return FND_E_BAD_ARG;
    st = begin_change(v);
    if (st != FND_OK)
        return st;
    if (set_mode)
        n.mode = mode;
    if (set_uid)
        n.uid = uid;
    touch(&n, now, 0);
    return inode_write(v, ino, &n);
}

fnd_status fs2_sync(fs2_vol *v) {
    /* Writes are synchronous; nothing is held back. */
    return v->failed ? FND_E_IO : FND_OK;
}

/* ---- format ----------------------------------------------------------- */

static int layout(uint32_t total, uint32_t boot_blocks, uint32_t inodes, uint32_t *bitmap_start,
                  uint32_t *bitmap_blocks, uint32_t *inode_start, uint32_t *inode_blocks, uint32_t *data_start) {
    uint64_t bs = 2ull + boot_blocks;
    uint64_t bb = ((uint64_t)total + 4095) / 4096;
    uint64_t is = bs + bb;
    uint64_t ib = ((uint64_t)inodes + 3) / 4;
    uint64_t ds = is + ib;
    if (ds > total)
        return 0;
    *bitmap_start = (uint32_t)bs, *bitmap_blocks = (uint32_t)bb, *inode_start = (uint32_t)is;
    *inode_blocks = (uint32_t)ib, *data_start = (uint32_t)ds;
    return 1;
}

fnd_status fs2_format(fs_dev *dev, uint32_t total, uint32_t boot_blocks, uint32_t inodes, uint32_t root_mode,
                      fs_time now) {
    uint8_t buf[FS2_BLOCK] FS_ALIGNED;
    uint32_t bms, bmb, ins, inb, ds;
    if (inodes < 1 || inodes > 0xFFFFFFFEu || total > dev->sectors ||
        !layout(total, boot_blocks, inodes, &bms, &bmb, &ins, &inb, &ds))
        return FND_E_BAD_ARG;
    if (dev->read_only)
        return FND_E_READ_ONLY;
    fnd_status st;
    fs_zero(buf, sizeof buf);
    buf[0x1FE] = 0x55, buf[0x1FF] = 0xAA;
    if ((st = dev->write(dev->ctx, 0, 1, buf)) != FND_OK)
        return st;
    fs_zero(buf, sizeof buf);
    for (uint32_t b = 2; b < ds; ++b) {
        if (b >= bms && b < ins) {
            fs_zero(buf, sizeof buf);
            uint32_t first = (b - bms) * FS2_BLOCK * 8;
            for (uint32_t i = 0; i < FS2_BLOCK * 8; ++i)
                if (first + i < ds)
                    bit_set(buf, i, 1);
        } else if (b == ins) {
            fs_zero(buf, sizeof buf);
            fs2_inode root;
            memset(&root, 0, sizeof root);
            root.type = FS2_TYPE_DIR, root.mode = root_mode, root.parent = FS2_ROOT;
            root.created = root.modified = root.changed = now;
            inode_encode(buf, &root);
        } else {
            fs_zero(buf, sizeof buf);
        }
        if ((st = dev->write(dev->ctx, b, 1, buf)) != FND_OK)
            return st;
    }
    fs_zero(buf, sizeof buf);
    buf[0] = 'F', buf[1] = 'S', buf[2] = '2', buf[3] = 0;
    fs_put16(buf + 4, 1);
    fs_put32(buf + 0x08, total);
    fs_put32(buf + 0x0C, 2);
    fs_put32(buf + 0x10, boot_blocks);
    fs_put32(buf + 0x18, bms);
    fs_put32(buf + 0x1C, bmb);
    fs_put32(buf + 0x20, ins);
    fs_put32(buf + 0x24, inb);
    fs_put32(buf + 0x28, inodes);
    fs_put32(buf + 0x2C, ds);
    fs_put32(buf + 0x30, FS2_ROOT);
    fs_put32(buf + 0x34, FS2_CASEFOLD_VERSION);
    time_put(buf + 0x38, now);
    return dev->write(dev->ctx, 1, 1, buf);
}

/* ---- mount (§10) -------------------------------------------------------- */

static int all_zero(const uint8_t *p, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i)
        if (p[i])
            return 0;
    return 1;
}

/* Scratch state of the mount check. */
typedef struct {
    uint8_t *seen;    /* data blocks owned by a chain */
    uint8_t *refd;    /* inodes referenced by an entry */
    uint32_t *hashes; /* folded key hashes of one directory */
    uint32_t hash_cap;
} check_state;

static fnd_status check_inode_fields(fs2_vol *v, const uint8_t *p, uint32_t ino) {
    uint32_t type = p[0];
    if (type == FS2_TYPE_FREE)
        return all_zero(p, FS2_INODE_BYTES) ? FND_OK : FND_E_FORMAT;
    if (type != FS2_TYPE_FILE && type != FS2_TYPE_DIR)
        return FND_E_FORMAT;
    if (p[1] || (fs_be16(p + 2) & ~FS2_MODE_MASK) || fs_be32(p + 4) == 0xFFFFFFFFu)
        return FND_E_FORMAT;
    if (!all_zero(p + 0x1C, 4) || !all_zero(p + 0x2A, 2) || !all_zero(p + 0x36, 2) || !all_zero(p + 0x42, 62))
        return FND_E_FORMAT;
    if (fs_be16(p + 0x28) > 999 || fs_be16(p + 0x34) > 999 || fs_be16(p + 0x40) > 999)
        return FND_E_FORMAT;
    fs2_inode n;
    inode_decode(p, &n);
    uint64_t need = n.size / FS2_PAYLOAD + (n.size % FS2_PAYLOAD != 0);
    if (need != n.count || (n.count == 0) != (n.first == 0))
        return FND_E_FORMAT;
    if (n.type == FS2_TYPE_DIR && n.size % FS2_DIRENT)
        return FND_E_FORMAT;
    if (n.parent == 0 || n.parent > v->inode_count)
        return FND_E_FORMAT;
    if (ino == FS2_ROOT && (n.type != FS2_TYPE_DIR || n.parent != FS2_ROOT))
        return FND_E_FORMAT;
    return FND_OK;
}

/* §10 step 7: the chain ends in range after exactly `count` blocks, never
 * reuses a block, only uses blocks marked used, and the last payload is
 * zero past the end of the file. */
static fnd_status check_chain(fs2_vol *v, check_state *cs, const fs2_inode *n) {
    const uint8_t *buf;
    uint32_t b = n->first;
    for (uint32_t i = 0; i < n->count; ++i) {
        if (b < v->data_start || b >= v->total_blocks || bit_get(cs->seen, b) || !bit_get(v->bitmap, b))
            return FND_E_FORMAT;
        bit_set(cs->seen, b, 1);
        fnd_status st = sec_get(v, b, &buf);
        if (st != FND_OK)
            return st;
        uint32_t next = next_of(buf);
        if (i + 1 == n->count) {
            uint32_t used = (uint32_t)(n->size - (uint64_t)i * FS2_PAYLOAD);
            if (next != 0 || !all_zero(buf + 4 + used, FS2_PAYLOAD - used))
                return FND_E_FORMAT;
        }
        b = next;
    }
    return FND_OK;
}

/* §10 step 6, and the reference half of step 5. */
static fnd_status check_dir(fs2_vol *v, check_state *cs, uint32_t dir, const fs2_inode *n) {
    static uint8_t e[FS2_DIRENT] FS_ALIGNED;
    uint32_t slots = dir_slots(n), used = 0;
    /* Two words per used entry: the hash and the slot. */
    if (2 * slots > cs->hash_cap) {
        fs_free(cs->hashes);
        cs->hashes = (uint32_t *)fs_alloc(2 * slots * 4u);
        cs->hash_cap = cs->hashes ? 2 * slots : 0;
        if (!cs->hashes)
            return FND_E_NO_MEMORY;
    }
    for (uint32_t s = 0; s < slots; ++s) {
        fnd_status st = raw_read(v, dir, (uint64_t)s * FS2_DIRENT, e, FS2_DIRENT);
        if (st != FND_OK)
            return st;
        uint32_t ino = fs_be32(e);
        if (ino == 0) {
            if (!all_zero(e, FS2_DIRENT))
                return FND_E_FORMAT;
            continue;
        }
        uint32_t nb = fs_be16(e + 4), nc = fs_be16(e + 6), chars;
        if (ino == FS2_ROOT || ino > v->inode_count || !bit_get(v->inode_used, ino) || bit_get(cs->refd, ino))
            return FND_E_FORMAT;
        if (nb < 1 || nb > FS2_NAME_MAX_BYTES || nc < 1 || nc > FS2_NAME_MAX_CHARS)
            return FND_E_FORMAT;
        if (fnd_name_check(e + 8, nb, &chars) != FND_OK || chars != nc)
            return FND_E_FORMAT;
        if (!all_zero(e + 8 + nb, FS2_NAME_MAX_BYTES - nb) || !all_zero(e + 0x408, 8))
            return FND_E_FORMAT;
        fs2_inode child;
        st = fs2_inode_read(v, ino, &child);
        if (st != FND_OK)
            return st;
        if (child.parent != dir)
            return FND_E_FORMAT;
        bit_set(cs->refd, ino, 1);
        /* Unique folded keys: compare names only when the hashes agree. */
        uint32_t h = fnd_name_key_hash(e + 8, nb);
        for (uint32_t k = 0; k < used; ++k) {
            if (cs->hashes[2 * k] != h)
                continue;
            fs2_dirent other;
            st = dirent_read(v, dir, n, cs->hashes[2 * k + 1], &other);
            if (st != FND_OK)
                return st;
            if (fnd_name_key_equal(other.name, other.name_bytes, e + 8, nb))
                return FND_E_FORMAT;
        }
        cs->hashes[2 * used] = h;
        cs->hashes[2 * used + 1] = s;
        ++used;
    }
    return FND_OK;
}

static fnd_status check_volume(fs2_vol *v, check_state *cs) {
    const uint8_t *buf;
    fnd_status st;
    /* Step 3: metadata bits set, padding bits past total_blocks clear. */
    for (uint32_t b = 0; b < v->data_start; ++b)
        if (!bit_get(v->bitmap, b))
            return FND_E_FORMAT;
    for (uint32_t b = v->total_blocks; b < v->bitmap_blocks * FS2_BLOCK * 8; ++b)
        if (bit_get(v->bitmap, b))
            return FND_E_FORMAT;
    /* Step 4: every inode slot, and the padding after the last one. */
    for (uint32_t blk = 0; blk < v->inode_blocks; ++blk) {
        st = sec_get(v, v->inode_start + blk, &buf);
        if (st != FND_OK)
            return st;
        for (uint32_t k = 0; k < 4; ++k) {
            uint32_t ino = blk * 4 + k + 1;
            const uint8_t *p = buf + k * FS2_INODE_BYTES;
            if (ino > v->inode_count) {
                if (!all_zero(p, FS2_INODE_BYTES))
                    return FND_E_FORMAT;
                continue;
            }
            st = check_inode_fields(v, p, ino);
            if (st != FND_OK)
                return st;
            if (p[0] != FS2_TYPE_FREE)
                bit_set(v->inode_used, ino, 1);
            else if (ino == FS2_ROOT)
                return FND_E_FORMAT;
        }
    }
    if (!FS2_FULL_MOUNT_CHECK) {
        /* Mount bookkeeping is still required when the full scan is off.
         * Do not read file/directory chains just to count free blocks. */
        v->free_blocks = 0;
        for (uint32_t b = v->data_start; b < v->total_blocks; ++b)
            if (!bit_get(v->bitmap, b))
                ++v->free_blocks;
        return FND_OK;
    }
    /* Step 7 for every inode, before any directory is read through its
     * chain. */
    for (uint32_t ino = 1; ino <= v->inode_count; ++ino) {
        if (!bit_get(v->inode_used, ino))
            continue;
        fs2_inode n;
        if ((st = fs2_inode_read(v, ino, &n)) != FND_OK || (st = check_chain(v, cs, &n)) != FND_OK)
            return st;
    }
    /* Step 8: no used data block outside every chain. */
    v->free_blocks = 0;
    for (uint32_t b = v->data_start; b < v->total_blocks; ++b) {
        if (bit_get(v->bitmap, b) != bit_get(cs->seen, b))
            return FND_E_FORMAT;
        if (!bit_get(v->bitmap, b))
            ++v->free_blocks;
    }
    /* Step 6 and the references of step 5. */
    for (uint32_t ino = 1; ino <= v->inode_count; ++ino) {
        if (!bit_get(v->inode_used, ino))
            continue;
        fs2_inode n;
        if ((st = fs2_inode_read(v, ino, &n)) != FND_OK)
            return st;
        if (n.type == FS2_TYPE_DIR && (st = check_dir(v, cs, ino, &n)) != FND_OK)
            return st;
    }
    /* Step 5: one entry per inode, and every parent chain reaches the root
     * (no cycle detached from the tree). */
    for (uint32_t ino = 2; ino <= v->inode_count; ++ino) {
        if (!bit_get(v->inode_used, ino))
            continue;
        if (!bit_get(cs->refd, ino))
            return FND_E_FORMAT;
        uint32_t under;
        if ((st = fs2_is_under(v, ino, FS2_ROOT, &under)) != FND_OK)
            return st;
        if (!under)
            return FND_E_FORMAT;
    }
    return FND_OK;
}

static fnd_status read_superblock(fs2_vol *v) {
    uint8_t sb[FS2_BLOCK] FS_ALIGNED;
    fnd_status st = sec_read(v, 0, sb);
    if (st != FND_OK)
        return st;
    if (sb[0x1FE] != 0x55 || sb[0x1FF] != 0xAA)
        return FND_E_FORMAT;
    if ((st = sec_read(v, 1, sb)) != FND_OK)
        return st;
    if (memcmp(sb, "FS2\0", 4) != 0)
        return FND_E_FORMAT;
    if (fs_be16(sb + 4) != 1 || (fs_be16(sb + 6) & ~FS2_SB_DIRTY))
        return FND_E_UNSUPPORTED;
    if (fs_be32(sb + 0x34) != FS2_CASEFOLD_VERSION)
        return FND_E_UNSUPPORTED;
    v->flags = fs_be16(sb + 6);
    v->total_blocks = fs_be32(sb + 0x08);
    v->boot_blocks = fs_be32(sb + 0x10);
    v->boot_size = fs_be32(sb + 0x14);
    v->bitmap_start = fs_be32(sb + 0x18);
    v->bitmap_blocks = fs_be32(sb + 0x1C);
    v->inode_start = fs_be32(sb + 0x20);
    v->inode_blocks = fs_be32(sb + 0x24);
    v->inode_count = fs_be32(sb + 0x28);
    v->data_start = fs_be32(sb + 0x2C);
    for (uint32_t i = 0; i < FS2_CACHE_SECTORS; ++i)
        v->cache[i].valid = 0; /* cache_line() depends on data_start */
    dir_index_drop(v);
    time_get(sb + 0x38, &v->created);
    uint32_t bms, bmb, ins, inb, ds;
    if (fs_be32(sb + 0x0C) != 2 || fs_be32(sb + 0x30) != FS2_ROOT || v->inode_count < 1 ||
        v->inode_count > 0xFFFFFFFEu ||
        !layout(v->total_blocks, v->boot_blocks, v->inode_count, &bms, &bmb, &ins, &inb, &ds) ||
        bms != v->bitmap_start || bmb != v->bitmap_blocks || ins != v->inode_start || inb != v->inode_blocks ||
        ds != v->data_start || v->total_blocks > v->dev->sectors ||
        (uint64_t)v->boot_size > (uint64_t)v->boot_blocks * FS2_BLOCK || v->created.ms > 999 ||
        !all_zero(sb + 0x42, 446))
        return FND_E_FORMAT;
    if (v->flags & FS2_SB_DIRTY)
        return FND_E_NEEDS_FSCK;
    return FND_OK;
}

static void vol_release(fs2_vol *v) {
    fs_free(v->bitmap);
    fs_free(v->inode_used);
    v->bitmap = v->inode_used = 0;
}

fnd_status fs2_mount(fs2_vol *v, fs_dev *dev, uint32_t writable) {
    fs_zero(v, sizeof *v);
    v->dev = dev;
    fnd_status st = read_superblock(v);
    if (st != FND_OK)
        return st;
    v->bitmap = (uint8_t *)fs_alloc(v->bitmap_blocks * FS2_BLOCK);
    v->inode_used = (uint8_t *)fs_alloc(v->inode_count / 8 + 1);
    check_state cs = {0, 0, 0, 0};
    if (FS2_FULL_MOUNT_CHECK) {
        cs.seen = (uint8_t *)fs_alloc(v->bitmap_blocks * FS2_BLOCK);
        cs.refd = (uint8_t *)fs_alloc(v->inode_count / 8 + 1);
    }
    if (!v->bitmap || !v->inode_used || (FS2_FULL_MOUNT_CHECK && (!cs.seen || !cs.refd))) {
        st = FND_E_NO_MEMORY;
    } else {
        for (uint32_t i = 0; i < v->bitmap_blocks && st == FND_OK; ++i)
            st = sec_read(v, v->bitmap_start + i, v->bitmap + i * FS2_BLOCK);
        if (st == FND_OK)
            st = check_volume(v, &cs);
    }
    fs_free(cs.seen);
    fs_free(cs.refd);
    fs_free(cs.hashes);
    if (st != FND_OK) {
        vol_release(v);
        return st;
    }
    v->writable = writable && !dev->read_only;
    return FND_OK;
}

fnd_status fs2_unmount(fs2_vol *v) {
    fnd_status st = FND_OK;
    if (v->dirty && !v->failed) {
        uint8_t sb[FS2_BLOCK] FS_ALIGNED;
        st = sec_read(v, 1, sb);
        if (st == FND_OK) {
            fs_put16(sb + 6, fs_be16(sb + 6) & ~FS2_SB_DIRTY);
            st = sec_write(v, 1, sb);
        }
        if (st == FND_OK)
            v->dirty = 0;
    }
    if (st == FND_OK)
        vol_release(v);
    return st;
}
