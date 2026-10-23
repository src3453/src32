/* cp [-f] src... dst (doc/spec_Foundation_shell.md §10.5). The new file gets
 * the fs's creation defaults. Same-file detection compares the path text only
 * (FS2 has no hard links and fnd_stat_t carries no identity). */
#include "cmd.h"

static int path_equal(const char *a, const char *b) {
    while (*a && *a == *b)
        ++a, ++b;
    return *a == *b;
}

/* The last component of path, ignoring trailing slashes. */
static const char *base_name(const char *path, uint32_t *len) {
    uint32_t end = cmd_len(path);
    while (end > 1u && path[end - 1u] == '/')
        --end;
    uint32_t start = end;
    while (start > 0 && path[start - 1u] != '/')
        --start;
    *len = end - start;
    return path + start;
}

static int32_t copy_file(const char *src, const char *dst, int force, uint8_t *buf) {
    fnd_stat_t st;
    fnd_status status = fnd_path_stat(src, &st);
    if (status != FND_OK)
        return status;
    if (st.type == FND_TYPE_DIR)
        return FND_E_IS_DIR;
    if (path_equal(src, dst))
        return FND_E_BAD_ARG;
    fnd_handle in, out;
    status = fnd_file_open(src, FND_OPEN_READ, FND_MODE_DEFAULT, &in);
    if (status != FND_OK)
        return status;
    uint32_t flags = FND_OPEN_WRITE | FND_OPEN_CREATE | (force ? FND_OPEN_TRUNC : FND_OPEN_EXCL);
    status = fnd_file_open(dst, flags, FND_MODE_DEFAULT, &out);
    if (status != FND_OK) {
        fnd_close(in);
        return status;
    }
    for (;;) {
        uint32_t got = 0;
        status = fnd_read(in, buf, FND_IPC_MAX_DATA, &got);
        if (status != FND_OK || got == 0)
            break;
        status = fnd_write(out, buf, got, 0);
        if (status != FND_OK)
            break;
    }
    fnd_close(in);
    fnd_status closed = fnd_close(out);
    return status != FND_OK ? status : closed;
}

static int32_t cp(uint32_t argc) {
    int force = 0;
    uint32_t first = 1;
    for (; first < argc && fnd_argv(first)[0] == '-' && fnd_argv(first)[1]; ++first) {
        const char *opt = fnd_argv(first);
        if (opt[1] != 'f' || opt[2])
            return cmd_usage("cp [-f] src... dst");
        force = 1;
    }
    if (argc - first < 2u)
        return cmd_usage("cp [-f] src... dst");
    uint32_t last = argc - 1u;
    const char *dst = fnd_argv(last);
    fnd_stat_t st;
    int dst_is_dir = fnd_path_stat(dst, &st) == FND_OK && st.type == FND_TYPE_DIR;
    if (last - first > 1u && !dst_is_dir)
        return cmd_report("cp", dst, FND_E_NOT_DIR);
    uint8_t *buf = fnd_mem_alloc(FND_IPC_MAX_DATA);
    uint32_t dst_len = cmd_len(dst);
    char *target = fnd_mem_alloc(dst_len + 2u + 1024u * 4u);
    if (!buf || !target) {
        fnd_mem_free(buf);
        fnd_mem_free(target);
        return cmd_report("cp", 0, FND_E_NO_MEMORY);
    }
    int32_t result = FND_OK;
    for (uint32_t i = first; i < last; ++i) {
        const char *src = fnd_argv(i);
        const char *to = dst;
        if (dst_is_dir) {
            uint32_t name_len;
            const char *name = base_name(src, &name_len);
            uint32_t n = 0;
            for (; n < dst_len; ++n)
                target[n] = dst[n];
            if (n && target[n - 1u] != '/')
                target[n++] = '/';
            for (uint32_t k = 0; k < name_len && n < dst_len + 1024u * 4u; ++k)
                target[n++] = name[k];
            target[n] = 0;
            to = target;
        }
        fnd_status status = copy_file(src, to, force, buf);
        if (status != FND_OK) {
            int32_t code = cmd_report("cp", status == FND_E_EXISTS || status == FND_E_BAD_ARG ? to : src, status);
            if (result == FND_OK)
                result = code;
        }
    }
    fnd_mem_free(buf);
    fnd_mem_free(target);
    return result;
}

CMD_MAIN(cp)
