#ifndef PKCL_DIRENT_H
#define PKCL_DIRENT_H
#include <sys/types.h>

/* Directory streams over fnd_dir_open / fnd_dir_next (doc/spec_pkcl.md §6.2).
 * Newlib's own <dirent.h> is an #error on a target without one. */
#define DT_UNKNOWN 0
#define DT_CHR 2
#define DT_DIR 4
#define DT_REG 8

#define NAME_MAX 1024
#ifndef MAXNAMLEN
#define MAXNAMLEN NAME_MAX
#endif

struct dirent {
    ino_t d_ino;
    unsigned char d_type;
    char d_name[NAME_MAX + 1];
};

typedef struct pkcl_dir DIR;

DIR *opendir(const char *path);
struct dirent *readdir(DIR *dir);
int readdir_r(DIR *dir, struct dirent *entry, struct dirent **result);
void rewinddir(DIR *dir);
int closedir(DIR *dir);
int dirfd(DIR *dir);

#endif
