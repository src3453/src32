#ifndef PKCL_SYS_TYPES_H
#define PKCL_SYS_TYPES_H
#include <stddef.h>
#include <stdint.h>

/* off_t は 32 bit（2 GiB 未満）。BusyBox は CONFIG_LFS=n で組む。
 * 大きなオフセットは EOVERFLOW になる（doc/spec_pkcl.md §4）。 */
typedef int32_t ssize_t;
typedef int32_t off_t;
typedef uint32_t mode_t;
typedef uint32_t uid_t;
typedef uint32_t gid_t;
typedef int32_t pid_t;
typedef uint32_t dev_t;
typedef uint32_t ino_t;
typedef uint32_t nlink_t;
typedef int64_t time_t;

#endif
