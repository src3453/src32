#ifndef PKCL_SYS_STATFS_H
#define PKCL_SYS_STATFS_H
#include <sys/types.h>

/* FS_VOLUME_INFO (doc/spec_pkcl.md §6.2): block counts of one volume. */
typedef struct { int __val[2]; } fsid_t;

struct statfs {
    long f_type;
    long f_bsize;
    unsigned long f_blocks;
    unsigned long f_bfree;
    unsigned long f_bavail;
    unsigned long f_files;
    unsigned long f_ffree;
    fsid_t f_fsid;
    long f_namelen;
    long f_frsize;
};

int statfs(const char *path, struct statfs *buf);
int fstatfs(int fd, struct statfs *buf);

#endif
