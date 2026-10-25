#ifndef PKCL_SYS_UTSNAME_H
#define PKCL_SYS_UTSNAME_H

/* uname(2) (doc/spec_pkcl.md §6.8): the fields are fixed strings of the running Foundation. */
struct utsname {
    char sysname[65];
    char nodename[65];
    char release[65];
    char version[65];
    char machine[65];
};

int uname(struct utsname *name);

#endif
