/* ls [-l] [path...] (doc/spec_Foundation_shell.md §10.3, §10.4). The fs
 * returns a listing in no particular order (API §7), so the entries are
 * collected and sorted here: ASCII letters fold, other bytes compare as
 * UTF-8 (libfnd's fnd_name_fold is not in the WAL "fnd" 1.0 exports). */
#include "cmd.h"

struct item {
    uint32_t type, size, mtime;
    char *name; /* the stored form */
    char *key; /* the sort key */
};

static struct item **items;
static uint32_t item_count, item_cap;
static uint32_t file_total, dir_total;

static int32_t key_cmp(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return (int32_t)(uint8_t)*a - (int32_t)(uint8_t)*b;
}

static int32_t add_item(const fnd_stat_t *st, const char *name, uint32_t name_bytes) {
    if (item_count == item_cap) {
        uint32_t cap = item_cap ? item_cap * 2u : 32u;
        struct item **grown = fnd_mem_alloc(cap * sizeof *grown);
        if (!grown)
            return FND_E_NO_MEMORY;
        for (uint32_t i = 0; i < item_count; ++i)
            grown[i] = items[i];
        if (items)
            fnd_mem_free(items);
        items = grown;
        item_cap = cap;
    }
    struct item *it = fnd_mem_alloc(sizeof *it + 2u * (name_bytes + 1u));
    if (!it)
        return FND_E_NO_MEMORY;
    it->type = st->type, it->size = st->size_lo, it->mtime = st->modified_sec_lo;
    it->name = (char *)(it + 1);
    it->key = it->name + name_bytes + 1u;
    for (uint32_t i = 0; i < name_bytes; ++i) {
        char c = name[i];
        it->name[i] = c;
        it->key[i] = c >= 'A' && c <= 'Z' ? (char)(c + 32) : c;
    }
    it->name[name_bytes] = it->key[name_bytes] = 0;
    items[item_count++] = it;
    return FND_OK;
}

static void sort_items(void) {
    for (uint32_t i = 1; i < item_count; ++i) {
        struct item *it = items[i];
        uint32_t j = i;
        while (j > 0 && key_cmp(items[j - 1]->key, it->key) > 0) {
            items[j] = items[j - 1];
            --j;
        }
        items[j] = it;
    }
}

static void free_items(void) {
    for (uint32_t i = 0; i < item_count; ++i)
        fnd_mem_free(items[i]);
    item_count = 0;
}

static void out_pad(uint32_t value, uint32_t width, char fill) {
    char text[3];
    text[0] = (char)('0' + value / 10u % 10u);
    text[1] = (char)('0' + value % 10u);
    text[2] = 0;
    (void)width;
    (void)fill;
    cmd_out(text);
}

/* UNIX seconds as UTC "YYYY-MM-DD HH:MM" (days from civil, Hinnant). */
static void out_time(uint32_t secs) {
    uint32_t days = secs / 86400u, rem = secs % 86400u;
    uint32_t z = days + 719468u;
    uint32_t era = z / 146097u, doe = z - era * 146097u;
    uint32_t yoe = (doe - doe / 1460u + doe / 36524u - doe / 146096u) / 365u;
    uint32_t doy = doe - (365u * yoe + yoe / 4u - yoe / 100u);
    uint32_t mp = (5u * doy + 2u) / 153u;
    uint32_t day = doy - (153u * mp + 2u) / 5u + 1u;
    uint32_t month = mp < 10u ? mp + 3u : mp - 9u;
    uint32_t year = yoe + era * 400u + (month <= 2u ? 1u : 0u);
    cmd_out_dec((int32_t)year, 4);
    cmd_out("-");
    out_pad(month, 2, '0');
    cmd_out("-");
    out_pad(day, 2, '0');
    cmd_out(" ");
    out_pad(rem / 3600u, 2, '0');
    cmd_out(":");
    out_pad(rem / 60u % 60u, 2, '0');
}

static void out_entry(int long_format, uint32_t type, uint32_t size, uint32_t mtime, const char *name) {
    if (long_format) {
        out_time(mtime);
        if (type == FND_TYPE_DIR) {
            cmd_out("  <DIR>     ");
            ++dir_total;
        } else {
            cmd_out("  ");
            cmd_out_dec((int32_t)size, 10);
            ++file_total;
        }
        cmd_out("  ");
    }
    cmd_out(name);
    cmd_out("\n");
}

static int32_t list_dir(const char *path, int long_format) {
    fnd_msg rep;
    fnd_status status = fnd_call(fnd_fs(), FND_FS_DIR_OPEN, 0, path, cmd_len(path), 0, 0, &rep);
    if (status < 0)
        return status;
    fnd_handle dir = rep.a3;
    uint8_t *buf = fnd_mem_alloc(FND_IPC_MAX_DATA);
    if (!buf) {
        fnd_close(dir);
        return FND_E_NO_MEMORY;
    }
    for (;;) {
        status = fnd_call(dir, FND_DIR_NEXT, 0, 0, 0, buf, FND_IPC_MAX_DATA, &rep);
        if (status < 0 || rep.a0 == 0)
            break;
        uint32_t off = 0;
        for (uint32_t n = 0; n < rep.a0 && off + FND_DIRENT_HEAD <= rep.len; ++n) {
            const uint32_t *e = (const uint32_t *)(buf + off);
            uint32_t rec_len = e[0];
            uint32_t name_bytes = (uint32_t)buf[off + 68] | (uint32_t)buf[off + 69] << 8;
            status = add_item((const fnd_stat_t *)(buf + off + 4), (const char *)buf + off + 72, name_bytes);
            if (status < 0 || rec_len < FND_DIRENT_HEAD)
                break;
            off += rec_len;
        }
        if (status < 0)
            break;
    }
    fnd_mem_free(buf);
    fnd_close(dir);
    if (status < 0)
        return status;
    sort_items();
    for (uint32_t i = 0; i < item_count; ++i)
        out_entry(long_format, items[i]->type, items[i]->size, items[i]->mtime, items[i]->name);
    free_items();
    return FND_OK;
}

static int32_t list_path(const char *path, int long_format) {
    fnd_stat_t st;
    fnd_status status = fnd_path_stat(path, &st);
    if (status != FND_OK)
        return status;
    if (st.type == FND_TYPE_DIR)
        return list_dir(path, long_format);
    out_entry(long_format, st.type, st.size_lo, st.modified_sec_lo, path);
    return FND_OK;
}

static int32_t ls(uint32_t argc) {
    int long_format = 0;
    uint32_t first = 1;
    for (; first < argc && fnd_argv(first)[0] == '-' && fnd_argv(first)[1]; ++first) {
        const char *opt = fnd_argv(first);
        if (opt[1] != 'l' || opt[2])
            return cmd_usage("ls [-l] [path...]");
        long_format = 1;
    }
    int32_t result = FND_OK;
    uint32_t paths = argc - first;
    for (uint32_t i = first; i < argc || (paths == 0 && i == first); ++i) {
        const char *path = paths ? fnd_argv(i) : ".";
        if (paths > 1u) {
            if (i > first)
                cmd_out("\n");
            cmd_out(path);
            cmd_out(":\n");
        }
        fnd_status status = list_path(path, long_format);
        if (status != FND_OK)
            result = cmd_report("ls", path, status);
        if (paths == 0)
            break;
    }
    if (long_format) {
        cmd_out_dec((int32_t)file_total, 0);
        cmd_out(" file(s), ");
        cmd_out_dec((int32_t)dir_total, 0);
        cmd_out(" dir(s)\n");
    }
    return result;
}

CMD_MAIN(ls)
