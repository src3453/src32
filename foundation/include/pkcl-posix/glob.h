#ifndef PKCL_GLOB_H
#define PKCL_GLOB_H
#include_next <glob.h>
#ifndef GLOB_NOMATCH
#define GLOB_NOMATCH (-3)
#endif
#endif
