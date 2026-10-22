#ifndef FOUNDATION_SYS_H
#define FOUNDATION_SYS_H
#include <cpt32/cpu.h>
#include <foundation/abi.h>

/* User-mode system call wrappers (doc/spec_Foundation.md §7, §12). They are
 * macros over the cpt32_syscall intrinsic, so they expand inline and also
 * work in code that may not call outside its granted range. */
#define fnd_sys_yield() ((int32_t)CPT32_SYSCALL0(FND_SYS_YIELD))
#define fnd_sys_sleep(ticks) ((int32_t)CPT32_SYSCALL1(FND_SYS_SLEEP, (ticks)))
#define fnd_sys_thread_exit(code) ((void)CPT32_SYSCALL1(FND_SYS_THREAD_EXIT, (uint32_t)(code)))
#define fnd_sys_proc_exit(code) ((void)CPT32_SYSCALL1(FND_SYS_PROC_EXIT, (uint32_t)(code)))
#define fnd_sys_time_get(selector) ((int32_t)CPT32_SYSCALL1(FND_SYS_TIME_GET, (selector)))
#define fnd_sys_debug_write(buffer, len)                                              \
    ((int32_t)CPT32_SYSCALL2(FND_SYS_DEBUG_WRITE, (uint32_t)(uintptr_t)(buffer), (len)))

#endif
