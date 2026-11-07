/* tree [-d] [path...] (doc/spec_Foundation_shell.md §10.3). Prints each
 * directory as an ASCII tree, entries sorted like ls (ASCII letters fold,
 * other bytes compare as UTF-8), then "N directories, M files" for all the
 * paths together. The fs returns a listing in no particular order (API §7),
 * so every level is collected and sorted before it is printed. Recursion is
 * limited to TREE_MAX_DEPTH levels; deeper directories are shown but not
 * entered. */
#include "cmd.h"

#define TREE_MAX_DEPTH 32u
#define BRANCH_BYTES 4u

struct node {
    uint32_t type;
    char *name; /* the stored form */
    char *key;  /* the sort key */
};

struct level {
    struct node **nodes;
    uint32_t count, cap;
};

static uint32_t dir_total, file_total;
static int dirs_only;
static int32_t result = FND_OK;
/* One BRANCH_BYTES piece per level above the entry being printed. */
static char prefix[TREE_MAX_DEPTH * BRANCH_BYTES + 1u];

static int32_t key_cmp(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return (int32_t)(uint8_t)*a - (int32_t)(uint8_t)*b;
}

static int32_t add_node(struct level *lv, uint32_t type, const char *name, uint32_t name_bytes) {
    if (lv->count == lv->cap) {
        uint32_t cap = lv->cap ? lv->cap * 2u : 16u;
        struct node **grown = fnd_mem_alloc(cap * sizeof *grown);
        if (!grown)
            return FND_E_NO_MEMORY;
        for (uint32_t i = 0; i < lv->count; ++i)
            grown[i] = lv->nodes[i];
        if (lv->nodes)
            fnd_mem_free(lv->nodes);
        lv->nodes = grown;
        lv->cap = cap;
    }
    struct node *n = fnd_mem_alloc(sizeof *n + 2u * (name_bytes + 1u));
    if (!n)
        return FND_E_NO_MEMORY;
    n->type = type;
    n->name = (char *)(n + 1);
    n->key = n->name + name_bytes + 1u;
    for (uint32_t i = 0; i < name_bytes; ++i) {
        char c = name[i];
        n->name[i] = c;
        n->key[i] = c >= 'A' && c <= 'Z' ? (char)(c + 32) : c;
    }
    n->name[name_bytes] = n->key[name_bytes] = 0;
    lv->nodes[lv->count++] = n;
    return FND_OK;
}

static void sort_level(struct level *lv) {
    for (uint32_t i = 1; i < lv->count; ++i) {
        struct node *n = lv->nodes[i];
        uint32_t j = i;
        while (j > 0 && key_cmp(lv->nodes[j - 1]->key, n->key) > 0) {
            lv->nodes[j] = lv->nodes[j - 1];
            --j;
        }
        lv->nodes[j] = n;
    }
}

static void free_level(struct level *lv) {
    for (uint32_t i = 0; i < lv->count; ++i)
        fnd_mem_free(lv->nodes[i]);
    if (lv->nodes)
        fnd_mem_free(lv->nodes);
    lv->nodes = 0;
    lv->count = lv->cap = 0;
}

static int32_t read_level(const char *path, struct level *lv) {
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
            const fnd_stat_t *st = (const fnd_stat_t *)(buf + off + 4);
            status = add_node(lv, st->type, (const char *)buf + off + 72, name_bytes);
            if (status < 0 || rec_len < FND_DIRENT_HEAD)
                break;
            off += rec_len;
        }
        if (status < 0)
            break;
    }
    fnd_mem_free(buf);
    fnd_close(dir);
    return status < 0 ? status : FND_OK;
}

static char *join_path(const char *dir, const char *name) {
    uint32_t dl = cmd_len(dir), nl = cmd_len(name);
    int slash = dl > 0 && dir[dl - 1] != '/';
    char *out = fnd_mem_alloc(dl + (uint32_t)slash + nl + 1u);
    if (!out)
        return 0;
    for (uint32_t i = 0; i < dl; ++i)
        out[i] = dir[i];
    if (slash)
        out[dl] = '/';
    for (uint32_t i = 0; i < nl; ++i)
        out[dl + (uint32_t)slash + i] = name[i];
    out[dl + (uint32_t)slash + nl] = 0;
    return out;
}

static void fail(const char *path, int32_t status) {
    int32_t code = cmd_report("tree", path, status);
    if (result == FND_OK)
        result = code;
}

static void walk(const char *path, uint32_t depth) {
    struct level lv = {0, 0, 0};
    int32_t status = read_level(path, &lv);
    if (status < 0) {
        free_level(&lv);
        fail(path, status);
        return;
    }
    sort_level(&lv);
    /* With -d the files are neither printed nor counted, so the last
     * directory is the last branch. */
    uint32_t last = lv.count;
    for (uint32_t i = 0; i < lv.count; ++i)
        if (!dirs_only || lv.nodes[i]->type == FND_TYPE_DIR)
            last = i;
    for (uint32_t i = 0; i < lv.count; ++i) {
        struct node *n = lv.nodes[i];
        int is_dir = n->type == FND_TYPE_DIR;
        if (dirs_only && !is_dir)
            continue;
        cmd_out(prefix);
        cmd_out(i == last ? "`-- " : "|-- ");
        cmd_out(n->name);
        cmd_out("\n");
        if (!is_dir) {
            ++file_total;
            continue;
        }
        ++dir_total;
        if (depth + 1u >= TREE_MAX_DEPTH)
            continue;
        char *child = join_path(path, n->name);
        if (!child) {
            fail(path, FND_E_NO_MEMORY);
            continue;
        }
        char *tail = prefix + depth * BRANCH_BYTES;
        const char *piece = i == last ? "    " : "|   ";
        for (uint32_t k = 0; k < BRANCH_BYTES; ++k)
            tail[k] = piece[k];
        tail[BRANCH_BYTES] = 0;
        walk(child, depth + 1u);
        tail[0] = 0;
        fnd_mem_free(child);
    }
    free_level(&lv);
}

static void tree_path(const char *path) {
    fnd_stat_t st;
    fnd_status status = fnd_path_stat(path, &st);
    if (status != FND_OK) {
        fail(path, status);
        return;
    }
    cmd_out(path);
    cmd_out("\n");
    if (st.type != FND_TYPE_DIR) {
        if (!dirs_only)
            ++file_total;
        return;
    }
    prefix[0] = 0;
    walk(path, 0);
}

static int32_t tree(uint32_t argc) {
    uint32_t first = 1;
    for (; first < argc && fnd_argv(first)[0] == '-' && fnd_argv(first)[1]; ++first) {
        const char *opt = fnd_argv(first);
        if (opt[1] != 'd' || opt[2])
            return cmd_usage("tree [-d] [path...]");
        dirs_only = 1;
    }
    if (first == argc)
        tree_path(".");
    for (uint32_t i = first; i < argc; ++i)
        tree_path(fnd_argv(i));
    cmd_out("\n");
    cmd_out_dec((int32_t)dir_total, 0);
    cmd_out(dir_total == 1u ? " directory" : " directories");
    if (!dirs_only) {
        cmd_out(", ");
        cmd_out_dec((int32_t)file_total, 0);
        cmd_out(file_total == 1u ? " file" : " files");
    }
    cmd_out("\n");
    return result;
}

CMD_MAIN(tree)
