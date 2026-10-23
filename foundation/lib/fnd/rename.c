/* fnd_file_rename (doc/spec_Foundation_API.md §11.4). A file of its own,
 * not part of start.c: programs that link libfnd statically (fs, whose slot
 * is nearly full) leave it out, and the WAL "fnd" exports it. */
#include <foundation/fnd.h>

fnd_status fnd_file_rename(const char *old_path, const char *new_path) {
    uint32_t old_len = 0, new_len = 0;
    while (old_path[old_len])
        ++old_len;
    while (new_path[new_len])
        ++new_len;
    if (old_len + new_len > FND_IPC_MAX_DATA)
        return FND_E_NAME_TOO_LONG;
    uint8_t *both = fnd_mem_alloc(old_len + new_len + 1u);
    if (!both)
        return FND_E_NO_MEMORY;
    for (uint32_t i = 0; i < old_len; ++i)
        both[i] = (uint8_t)old_path[i];
    for (uint32_t i = 0; i < new_len; ++i)
        both[old_len + i] = (uint8_t)new_path[i];
    /* The data is the two paths back to back, a0 the length of the first. */
    uint32_t args[4] = {old_len, 0, 0, 0};
    fnd_status status = fnd_call(fnd_fs(), FND_FS_RENAME, args, both, old_len + new_len, 0, 0, 0);
    fnd_mem_free(both);
    return status;
}
