/* Read-only DSFS driver (doc/spec_DSFS.md). See dsfs.h. */
#include "dsfs.h"

#include <string.h>

static int all_zero(const uint8_t *p, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i)
        if (p[i])
            return 0;
    return 1;
}

static int bit_get(const uint8_t *map, uint32_t i) { return (map[i >> 3] >> (7 - (i & 7))) & 1; }

static int name_char(uint8_t c) { return (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-'; }

/* §6.2: non-blank characters followed by blank padding; the name field has
 * at least one character. */
static int field_ok(const uint8_t *f, uint32_t n, uint32_t min) {
    uint32_t i = 0;
    while (i < n && f[i] != ' ') {
        if (!name_char(f[i]))
            return 0;
        ++i;
    }
    if (i < min)
        return 0;
    for (; i < n; ++i)
        if (f[i] != ' ')
            return 0;
    return 1;
}

static uint32_t blocks_of(uint32_t size) { return size / 512u + (size % 512u != 0); }

fnd_status dsfs_mount(dsfs_vol *v, fs_dev *dev) {
    uint8_t buf[512];
    fnd_status st;
    memset(v, 0, sizeof *v);
    v->dev = dev;
    if ((st = dev->read(dev->ctx, 0, 1, buf)) != FND_OK)
        return st;
    if (buf[0x1FE] != 0x55 || buf[0x1FF] != 0xAA)
        return FND_E_FORMAT;
    if ((st = dev->read(dev->ctx, 1, 1, buf)) != FND_OK)
        return st;
    uint32_t total = fs_be32(buf + 8), bitmap_blocks = (uint32_t)(((uint64_t)total + 4095) / 4096);
    uint32_t dir_start = 2 + bitmap_blocks, data_start = dir_start + 16;
    if (memcmp(buf, "DSFS", 4) != 0 || fs_be16(buf + 4) != 1 || fs_be16(buf + 6) != 0 ||
        fs_be32(buf + 0x0C) != 2 || fs_be32(buf + 0x10) != bitmap_blocks || fs_be32(buf + 0x14) != dir_start ||
        fs_be32(buf + 0x18) != 16 || fs_be32(buf + 0x1C) != data_start || total < data_start ||
        total > dev->sectors || !all_zero(buf + 0x24, 512 - 0x24))
        return FND_E_FORMAT;
    v->total_blocks = total;
    v->data_start = data_start;
    uint8_t *bitmap = (uint8_t *)fs_alloc(bitmap_blocks * 512u);
    v->dir = (uint8_t *)fs_alloc(DSFS_DIR_BYTES);
    if (!bitmap || !v->dir) {
        st = FND_E_NO_MEMORY;
        goto out;
    }
    for (uint32_t i = 0; i < bitmap_blocks && st == FND_OK; ++i)
        st = dev->read(dev->ctx, 2 + i, 1, bitmap + i * 512u);
    for (uint32_t i = 0; i < 16 && st == FND_OK; ++i)
        st = dev->read(dev->ctx, dir_start + i, 1, v->dir + i * 512u);
    if (st != FND_OK)
        goto out;
    st = FND_E_FORMAT;
    for (uint32_t b = 0; b < data_start; ++b) /* §7-10 */
        if (!bit_get(bitmap, b))
            goto out;
    for (uint32_t b = total; b < bitmap_blocks * 4096u; ++b) /* §7-11 */
        if (bit_get(bitmap, b))
            goto out;
    for (uint32_t i = 0; i < DSFS_ENTRIES; ++i) {
        const uint8_t *e = v->dir + i * 32u;
        if (e[0] == 0) { /* §7-5 */
            if (!all_zero(e, 32))
                goto out;
            continue;
        }
        if (e[0] != 1 || !all_zero(e + 0x0C, 4)) /* §7-4, §7-5 */
            goto out;
        if (!field_ok(e + 1, 8, 1) || !field_ok(e + 9, 3, 0)) /* §7-6 */
            goto out;
        uint32_t start = fs_be32(e + 0x10), size = fs_be32(e + 0x14), n = blocks_of(size);
        if (size == 0 ? start != 0 : (start < data_start || (uint64_t)start + n > total)) /* §7-7 */
            goto out;
        for (uint32_t b = start; b < start + n; ++b) /* §7-9 */
            if (!bit_get(bitmap, b))
                goto out;
        for (uint32_t k = 0; k < i; ++k) { /* §7-6, §7-8 */
            const uint8_t *o = v->dir + k * 32u;
            if (o[0] != 1)
                continue;
            if (memcmp(o + 1, e + 1, 11) == 0)
                goto out;
            uint32_t os = fs_be32(o + 0x10), on = blocks_of(fs_be32(o + 0x14));
            if (n && on && start < os + on && os < start + n)
                goto out;
        }
    }
    for (uint32_t b = data_start; b < total; ++b)
        v->free_blocks += !bit_get(bitmap, b);
    st = FND_OK;
out:
    fs_free(bitmap);
    if (st != FND_OK)
        dsfs_unmount(v);
    return st;
}

void dsfs_unmount(dsfs_vol *v) {
    fs_free(v->dir);
    v->dir = 0;
}

void dsfs_entry_get(const dsfs_vol *v, uint32_t index, dsfs_entry *out) {
    const uint8_t *e = v->dir + index * 32u;
    memset(out, 0, sizeof *out);
    if (e[0] != 1)
        return;
    out->used = 1;
    out->start = fs_be32(e + 0x10);
    out->size = fs_be32(e + 0x14);
    out->created = fs_be32(e + 0x18);
    out->modified = fs_be32(e + 0x1C);
    /* §6.2: shown as NAME.EXT, or NAME without an extension. */
    for (uint32_t i = 0; i < 8 && e[1 + i] != ' '; ++i)
        out->name[out->name_len++] = e[1 + i];
    if (e[9] != ' ') {
        out->name[out->name_len++] = '.';
        for (uint32_t i = 0; i < 3 && e[9 + i] != ' '; ++i)
            out->name[out->name_len++] = e[9 + i];
    }
}

fnd_status dsfs_read(dsfs_vol *v, uint32_t index, uint64_t off, void *out, uint32_t len, uint32_t *got) {
    dsfs_entry e;
    uint8_t buf[512];
    dsfs_entry_get(v, index, &e);
    *got = 0;
    if (!e.used)
        return FND_E_NOT_FOUND;
    if (off >= e.size)
        return FND_OK;
    if ((uint64_t)len > e.size - off)
        len = (uint32_t)(e.size - off);
    uint32_t done = 0;
    while (done < len) {
        uint32_t pos = (uint32_t)off + done, in = pos % 512u, take = 512u - in;
        if (take > len - done)
            take = len - done;
        fnd_status st = v->dev->read(v->dev->ctx, e.start + pos / 512u, 1, buf);
        if (st != FND_OK)
            return st;
        memcpy((uint8_t *)out + done, buf + in, take);
        done += take;
    }
    *got = done;
    return FND_OK;
}
