/* ファイル系の残り・ユーザー DB・パターン照合（doc/spec_pkcl.md §6）。 */
#define _GNU_SOURCE 1
#include <errno.h>
#include <fnmatch.h>
#include <glob.h>
#include <grp.h>
#include <pwd.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

#include <foundation/fnd.h>
#include <pkcl/pkcl.h>

/* ---- リンク類: FS2 にはない ---- */
int link(const char *a, const char *b) { (void)a, (void)b; errno = EPERM; return -1; }
int symlink(const char *a, const char *b) { (void)a, (void)b; errno = EPERM; return -1; }
ssize_t readlink(const char *p, char *b, size_t n) { (void)p, (void)b, (void)n; errno = EINVAL; return -1; }
int mknod(const char *p, mode_t m, dev_t d) { (void)p, (void)m, (void)d; errno = EPERM; return -1; }
int chroot(const char *p) { (void)p; errno = EPERM; return -1; }
int fchdir(int fd) { (void)fd; errno = ENOSYS; return -1; }
/* 所有者は 1 人だけ（root）なので、存在するパスなら成功にする。 */
int chown(const char *p, uid_t u, gid_t g) { (void)u, (void)g; struct stat st; return stat(p, &st); }
int lchown(const char *p, uid_t u, gid_t g) { return chown(p, u, g); }
/* §7 G5: fs に時刻の更新がないので、存在確認だけして成功にする（時刻は変わらない）。 */
int utimes(const char *p, const struct timeval tv[2]) { (void)tv; struct stat st; return stat(p, &st); }

void *mmap(void *a, size_t n, int p, int f, int fd, off_t o) {
    (void)a, (void)n, (void)p, (void)f, (void)fd, (void)o;
    errno = ENOSYS;
    return (void *)-1;
}
int munmap(void *a, size_t n) { (void)a, (void)n; errno = EINVAL; return -1; }
int mallopt(int p, int v) { (void)p, (void)v; return 1; }
int clearenv(void) { extern char **environ; if (environ) environ[0] = 0; return 0; }

/* ---- パス ---- */
char *dirname(char *path) {
    if (!path || !*path)
        return (char *)".";
    size_t n = strlen(path);
    while (n > 1 && path[n - 1] == '/')
        path[--n] = 0;
    char *slash = strrchr(path, '/');
    if (!slash)
        return (char *)".";
    while (slash > path && slash[-1] == '/')
        --slash;
    if (slash == path)
        path[1] = 0;
    else
        *slash = 0;
    return path;
}

/* symlink がないので、"." と ".." と重複した "/" を畳んで存在を確かめるだけでよい。 */
char *realpath(const char *path, char *out) {
    char full[PKCL_PATH_MAX + 2];
    size_t n = 0;
    if (path[0] != '/') {
        if (!getcwd(full, sizeof full))
            return 0;
        n = strlen(full);
    }
    if (n + 1 + strlen(path) >= sizeof full) {
        errno = ENAMETOOLONG;
        return 0;
    }
    full[n++] = '/';
    strcpy(full + n, path);
    char *result = out ? out : malloc(PKCL_PATH_MAX + 1);
    if (!result) {
        errno = ENOMEM;
        return 0;
    }
    size_t len = 0;
    for (char *p = full; *p;) {
        char *end = strchr(p, '/');
        size_t part = end ? (size_t)(end - p) : strlen(p);
        if (part == 0 || (part == 1 && p[0] == '.')) {
        } else if (part == 2 && p[0] == '.' && p[1] == '.') {
            while (len > 0 && result[--len] != '/') {}
        } else {
            result[len++] = '/';
            memcpy(result + len, p, part);
            len += part;
        }
        p += part + (end != 0);
    }
    if (len == 0)
        result[len++] = '/';
    result[len] = 0;
    struct stat st;
    if (stat(result, &st) < 0) {
        if (!out)
            free(result);
        return 0;
    }
    return result;
}

/* ---- パターン照合 ---- */
static int match_here(const char *pat, const char *s, int flags, const char *start) {
    for (; *pat; ++pat) {
        if (*pat == '*') {
            while (pat[1] == '*')
                ++pat;
            if ((flags & FNM_PERIOD) && *s == '.' && s == start)
                return FNM_NOMATCH;
            for (;; ++s) {
                if (match_here(pat + 1, s, flags, start) == 0)
                    return 0;
                if (!*s || ((flags & FNM_PATHNAME) && *s == '/'))
                    return FNM_NOMATCH;
            }
        }
        if (!*s)
            return FNM_NOMATCH;
        if ((flags & FNM_PATHNAME) && *s == '/' && *pat != '/')
            return FNM_NOMATCH;
        if ((flags & FNM_PERIOD) && *s == '.' && s == start && *pat != '.')
            return FNM_NOMATCH;
        if (*pat == '?') {
            ++s;
        } else if (*pat == '[') {
            const char *p = pat + 1;
            int negate = *p == '!' || *p == '^', hit = 0;
            p += negate;
            for (int first = 1; *p && (*p != ']' || first); first = 0) {
                char lo = *p++, hi = lo;
                if (lo == '\\' && !(flags & FNM_NOESCAPE) && *p)
                    lo = hi = *p++;
                if (*p == '-' && p[1] && p[1] != ']') {
                    hi = p[1];
                    p += 2;
                }
                hit |= (unsigned char)*s >= (unsigned char)lo && (unsigned char)*s <= (unsigned char)hi;
            }
            if (*p != ']' || hit == negate)
                return FNM_NOMATCH;
            pat = p;
            ++s;
        } else {
            if (*pat == '\\' && !(flags & FNM_NOESCAPE) && pat[1])
                ++pat;
            if (*pat != *s)
                return FNM_NOMATCH;
            ++s;
        }
        if (pat[0] == 0)
            break;
    }
    return *s ? FNM_NOMATCH : 0;
}

int fnmatch(const char *pattern, const char *string, int flags) {
    return match_here(pattern, string, flags, string);
}

/* §6 の残り: ディレクトリを辿る glob はまだない。パターンに特殊文字がなければその名前だけ返す。 */
int glob(const char *pattern, int flags, int (*errfunc)(const char *, int), glob_t *g) {
    (void)errfunc;
    struct stat st;
    if (strpbrk(pattern, "*?[") || stat(pattern, &st) < 0)
        return GLOB_NOMATCH;
    size_t offs = (flags & GLOB_DOOFFS) ? g->gl_offs : 0;
    g->gl_pathv = calloc(offs + 2, sizeof(char *));
    if (!g->gl_pathv)
        return GLOB_NOSPACE;
    g->gl_pathv[offs] = strdup(pattern);
    g->gl_pathc = 1;
    return 0;
}
void globfree(glob_t *g) {
    for (size_t i = 0; g->gl_pathv && i < g->gl_pathc; ++i)
        free(g->gl_pathv[i]);
    free(g->gl_pathv);
    g->gl_pathv = 0;
}

/* ---- ユーザー DB: root だけ ---- */
static char root_name[] = "root", root_dir[] = "/", root_shell[] = "/bin/sh", empty[] = "";
static struct passwd root_pw = {root_name, empty, 0, 0, empty, root_name, root_dir, root_shell};
static char *no_members[1];
static struct group root_gr = {root_name, empty, 0, no_members};

struct passwd *getpwuid(uid_t uid) { return uid == 0 ? &root_pw : 0; }
struct passwd *getpwnam(const char *name) { return strcmp(name, "root") == 0 ? &root_pw : 0; }
struct group *getgrgid(gid_t gid) { return gid == 0 ? &root_gr : 0; }
struct group *getgrnam(const char *name) { return strcmp(name, "root") == 0 ? &root_gr : 0; }
void endgrent(void) {}

/* Newlib はこの 2 つを __ 付きの名前で持つ（ヘッダが POSIX の名前に写す）。 */
#include <stdio.h>
extern ssize_t __getline(char **, size_t *, FILE *);
extern ssize_t __getdelim(char **, size_t *, int, FILE *);
ssize_t getline(char **line, size_t *size, FILE *stream) { return __getline(line, size, stream); }
ssize_t getdelim(char **line, size_t *size, int delim, FILE *stream) { return __getdelim(line, size, delim, stream); }
