#ifndef PKCL_SYS_RESOURCE_H
#define PKCL_SYS_RESOURCE_H
#include_next <sys/resource.h>
#ifndef RLIMIT_CORE
typedef unsigned long rlim_t;
struct rlimit { rlim_t rlim_cur; rlim_t rlim_max; };
#define RLIM_INFINITY ((rlim_t)-1)
#define RLIMIT_CPU 0
#define RLIMIT_FSIZE 1
#define RLIMIT_DATA 2
#define RLIMIT_STACK 3
#define RLIMIT_CORE 4
#define RLIMIT_NOFILE 7
#define RLIMIT_AS 9
int getrlimit(int resource, struct rlimit *rlim);
int setrlimit(int resource, const struct rlimit *rlim);
#endif
#endif
