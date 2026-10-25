/* ディレクトリストリーム（doc/spec_pkcl.md §6.2）。fnd_dir_open / fnd_dir_next の上に作る。 */
#define _GNU_SOURCE 1
#include <dirent.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>

#include <foundation/fnd.h>
#include <pkcl/pkcl.h>

struct pkcl_dir {
    fnd_handle handle;
    struct dirent entry;
    fnd_dirent_t raw;
};

DIR *opendir(const char *path) {
    pkcl_init();
    DIR *dir = malloc(sizeof *dir);
    if (!dir) {
        errno = ENOMEM;
        return 0;
    }
    if (pkcl_fail(fnd_dir_open(path, &dir->handle))) {
        free(dir);
        return 0;
    }
    return dir;
}

int readdir_r(DIR *dir, struct dirent *entry, struct dirent **result) {
    fnd_status status = fnd_dir_next(dir->handle, &dir->raw);
    if (status == FND_END) {
        *result = 0;
        return 0;
    }
    if (status < 0) {
        *result = 0;
        return pkcl_errno(status);
    }
    entry->d_ino = 0;
    entry->d_type = dir->raw.stat.type == FND_TYPE_DIR ? DT_DIR
                    : dir->raw.stat.type == FND_TYPE_FILE ? DT_REG
                    : dir->raw.stat.type == FND_TYPE_CONSOLE ? DT_CHR : DT_UNKNOWN;
    strncpy(entry->d_name, dir->raw.name, NAME_MAX);
    entry->d_name[NAME_MAX] = 0;
    *result = entry;
    return 0;
}

struct dirent *readdir(DIR *dir) {
    struct dirent *result;
    int error = readdir_r(dir, &dir->entry, &result);
    if (error)
        errno = error;
    return result;
}

void rewinddir(DIR *dir) { fnd_dir_rewind(dir->handle); }

int closedir(DIR *dir) {
    fnd_status status = fnd_close(dir->handle);
    free(dir);
    return pkcl_fail(status);
}

int dirfd(DIR *dir) {
    (void)dir;
    errno = ENOTSUP; /* ディレクトリは fd 表に載らない */
    return -1;
}
