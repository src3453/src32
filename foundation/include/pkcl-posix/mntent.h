#ifndef PKCL_MNTENT_H
#define PKCL_MNTENT_H
#include <stdio.h>

/* Foundation has no mount table to read (doc/spec_pkcl.md §6.8). */
struct mntent {
    char *mnt_fsname;
    char *mnt_dir;
    char *mnt_type;
    char *mnt_opts;
    int mnt_freq;
    int mnt_passno;
};

FILE *setmntent(const char *file, const char *mode);
struct mntent *getmntent(FILE *stream);
int endmntent(FILE *stream);

#endif
