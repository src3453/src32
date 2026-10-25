#ifndef PKCL_SYS_WAIT_H
#define PKCL_SYS_WAIT_H
#include_next <sys/wait.h>
/* Foundation has no core dumps. */
#ifndef WCOREDUMP
#define WCOREDUMP(status) 0
#endif
#endif
