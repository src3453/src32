#ifndef PKCL_SYS_STAT_H
#define PKCL_SYS_STAT_H
#include_next <sys/stat.h>
/* pkcl は symlink を持たないので lstat は stat と同じ (doc/spec_pkcl.md §6) */
int mknod(const char *path, mode_t mode, dev_t dev);
int lstat(const char *path, struct stat *st);
#endif
