#ifndef PKCL_TIME_H
#define PKCL_TIME_H
#include_next <time.h>
int nanosleep(const struct timespec *req, struct timespec *rem);
#endif
