/* パスに対する POSIX 呼び出し（doc/spec_pkcl.md §6.2〜§6.3）。パスは書き換えず fs へ送る。 */
#define _GNU_SOURCE 1 /* libc のヘッダに O_CLOEXEC などを出させる */
#include <errno.h>
#include <fcntl.h>
#include <foundation/api.h>
#include <pkcl/pkcl.h>
#include <sys/stat.h>
#include <unistd.h>

int pkcl_stat_convert(const fnd_stat_t *fs, struct stat *st) {
    if (fs->size_hi != 0 || fs->size_lo > 0x7FFFFFFFu) {
        errno = EOVERFLOW;
        return -1;
    }
    st->st_dev = 1;
    st->st_ino = 0;
    st->st_mode = pkcl_mode_from_fnd(fs->type, fs->mode);
    if (fs->flags & FND_STAT_READ_ONLY)
        st->st_mode &= ~(mode_t)(S_IWUSR | S_IWGRP | S_IWOTH);
    st->st_nlink = 1;
    st->st_uid = fs->uid;
    st->st_gid = 0;
    st->st_rdev = 0;
    st->st_size = (off_t)fs->size_lo;
    st->st_blksize = 512;
    st->st_blocks = (fs->size_lo + 511u) / 512u;
    st->st_mtime = (time_t)(((uint64_t)fs->modified_sec_hi << 32) | fs->modified_sec_lo);
    st->st_atime = st->st_mtime;
    st->st_ctime = (time_t)(((uint64_t)fs->changed_sec_hi << 32) | fs->changed_sec_lo);
    return 0;
}

int stat(const char *path, struct stat *st) {
    pkcl_init();
    fnd_stat_t fs;
    if (pkcl_fail(fnd_path_stat(path, &fs)))
        return -1;
    return pkcl_stat_convert(&fs, st);
}

/* FS2 にシンボリックリンクはないので lstat は stat と同じ。 */
int lstat(const char *path, struct stat *st) { return stat(path, st); }

int access(const char *path, int mode) {
    pkcl_init();
    fnd_stat_t fs;
    if (pkcl_fail(fnd_path_stat(path, &fs)))
        return -1;
    if ((mode & W_OK) && (fs.flags & FND_STAT_READ_ONLY)) {
        errno = EROFS;
        return -1;
    }
    /* v1 のセッションは UID 0 で、R/W の検査を省略できる（FS2 §8）。実行は X bit が 1 つもなければ拒否する。 */
    if ((mode & X_OK) && fs.type == FND_TYPE_FILE && !(fs.mode & 011u)) {
        errno = EACCES;
        return -1;
    }
    return 0;
}

int mkdir(const char *path, mode_t mode) {
    pkcl_init();
    return pkcl_fail(fnd_dir_make(path, pkcl_mode_to_fnd(mode & ~pkcl_current_umask())));
}

int rmdir(const char *path) {
    pkcl_init();
    return pkcl_fail(fnd_dir_remove(path));
}

int unlink(const char *path) {
    pkcl_init();
    fnd_stat_t fs;
    /* POSIX は unlink(ディレクトリ) を EISDIR（Linux）または EPERM で拒否する。 */
    if (fnd_path_stat(path, &fs) == FND_OK && fs.type == FND_TYPE_DIR) {
        errno = EISDIR;
        return -1;
    }
    return pkcl_fail(fnd_file_delete(path));
}

int rename(const char *old_path, const char *new_path) {
    pkcl_init();
    /* POSIX は既存の new_path を置き換えるが、fnd_file_rename は EXISTS を返す。
     * 置換の模倣（削除してから rename）は途中で失敗すると両方を失うので、
     * 置換先が通常ファイルのときだけ行う（doc/spec_pkcl.md §6.2）。 */
    fnd_status status = fnd_file_rename(old_path, new_path);
    if (status == FND_E_EXISTS) {
        fnd_stat_t from, to;
        if (fnd_path_stat(old_path, &from) == FND_OK && fnd_path_stat(new_path, &to) == FND_OK &&
            from.type == FND_TYPE_FILE && to.type == FND_TYPE_FILE) {
            status = fnd_file_delete(new_path);
            if (status == FND_OK)
                status = fnd_file_rename(old_path, new_path);
        }
    }
    return pkcl_fail(status);
}

int chdir(const char *path) {
    pkcl_init();
    return pkcl_fail(fnd_cwd_set(path));
}

char *getcwd(char *buf, size_t size) {
    pkcl_init();
    uint32_t len = 0;
    if (!buf || size == 0) {
        errno = EINVAL;
        return 0;
    }
    fnd_status status = fnd_cwd_get(buf, (uint32_t)size, &len);
    if (status == FND_E_LIMIT) {
        errno = ERANGE;
        return 0;
    }
    return pkcl_fail(status) ? 0 : buf;
}

int chmod(const char *path, mode_t mode) {
    pkcl_init();
    uint32_t args[4] = {pkcl_mode_to_fnd(mode), 0, 0, 0};
    uint32_t n = 0;
    while (path[n])
        ++n;
    if (n == 0 || n > PKCL_PATH_MAX) {
        errno = n ? ENAMETOOLONG : ENOENT;
        return -1;
    }
    return pkcl_fail(fnd_call(fnd_fs(), FND_FS_CHMOD, args, path, n, 0, 0, 0));
}

int truncate(const char *path, off_t length) {
    int fd = open(path, O_WRONLY);
    if (fd < 0)
        return -1;
    int result = ftruncate(fd, length);
    int saved = errno;
    close(fd);
    errno = saved;
    return result;
}
