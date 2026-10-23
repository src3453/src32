#ifndef FOUNDATION_SYS_H
#define FOUNDATION_SYS_H
#include <cpt32/cpu.h>
#include <foundation/abi.h>
#include <foundation/ipc.h>

/* User-mode system call wrappers (doc/spec_Foundation.md §7, §12). They are
 * macros over the cpt32_syscall intrinsic, so they expand inline and also
 * work in code that may not call outside its granted range. */
#define fnd_sys_info(selector, buffer, cap)                                           \
    ((int32_t)CPT32_SYSCALL3(FND_SYS_INFO, (selector), (uint32_t)(uintptr_t)(buffer), (cap)))
#define fnd_sys_yield() ((int32_t)CPT32_SYSCALL0(FND_SYS_YIELD))
#define fnd_sys_sleep(ticks) ((int32_t)CPT32_SYSCALL1(FND_SYS_SLEEP, (ticks)))
#define fnd_sys_thread_create(entry, sp, arg)                                         \
    ((int32_t)CPT32_SYSCALL3(FND_SYS_THREAD_CREATE, (uint32_t)(uintptr_t)(entry), (uint32_t)(uintptr_t)(sp), (arg)))
#define fnd_sys_thread_exit(code) ((void)CPT32_SYSCALL1(FND_SYS_THREAD_EXIT, (uint32_t)(code)))
#define fnd_sys_proc_exit(code) ((void)CPT32_SYSCALL1(FND_SYS_PROC_EXIT, (uint32_t)(code)))
#define fnd_sys_time_get(selector) ((int32_t)CPT32_SYSCALL1(FND_SYS_TIME_GET, (selector)))
#define fnd_sys_ipc_call(ch, req, rep)                                                \
    ((int32_t)CPT32_SYSCALL3(FND_SYS_IPC_CALL, (ch), (uint32_t)(uintptr_t)(req), (uint32_t)(uintptr_t)(rep)))
#define fnd_sys_ipc_recv(port, rcv)                                                   \
    ((int32_t)CPT32_SYSCALL2(FND_SYS_IPC_RECV, (port), (uint32_t)(uintptr_t)(rcv)))
#define fnd_sys_ipc_reply(token, msg)                                                 \
    ((int32_t)CPT32_SYSCALL2(FND_SYS_IPC_REPLY, (token), (uint32_t)(uintptr_t)(msg)))
#define fnd_sys_ipc_reply_recv(token, msg, port, rcv)                                 \
    ((int32_t)CPT32_SYSCALL4(FND_SYS_IPC_REPLY_RECV, (token), (uint32_t)(uintptr_t)(msg), (port), \
                             (uint32_t)(uintptr_t)(rcv)))
#define fnd_sys_port_create() ((int32_t)CPT32_SYSCALL0(FND_SYS_PORT_CREATE))
#define fnd_sys_chan_create(port, badge) ((int32_t)CPT32_SYSCALL2(FND_SYS_CHAN_CREATE, (port), (badge)))
#define fnd_sys_handle_close(h) ((int32_t)CPT32_SYSCALL1(FND_SYS_HANDLE_CLOSE, (h)))
#define fnd_sys_handle_dup(h) ((int32_t)CPT32_SYSCALL1(FND_SYS_HANDLE_DUP, (h)))
#define fnd_sys_wait(h, timeout, code)                                                \
    ((int32_t)CPT32_SYSCALL3(FND_SYS_WAIT, (h), (timeout), (uint32_t)(uintptr_t)(code)))
#define fnd_sys_proc_spawn(spawn) ((int32_t)CPT32_SYSCALL1(FND_SYS_PROC_SPAWN, (uint32_t)(uintptr_t)(spawn)))
#define fnd_sys_proc_kill(h) ((int32_t)CPT32_SYSCALL1(FND_SYS_PROC_KILL, (h)))
#define fnd_sys_proc_startup(buf, cap)                                                \
    ((int32_t)CPT32_SYSCALL2(FND_SYS_PROC_STARTUP, (uint32_t)(uintptr_t)(buf), (cap)))
#define fnd_sys_mem_grow(delta) ((int32_t)CPT32_SYSCALL1(FND_SYS_MEM_GROW, (delta)))
#define fnd_sys_svc_register(name, port)                                              \
    ((int32_t)CPT32_SYSCALL2(FND_SYS_SVC_REGISTER, (uint32_t)(uintptr_t)(name), (port)))
#define fnd_sys_blk_io(blk) ((int32_t)CPT32_SYSCALL1(FND_SYS_BLK_IO, (uint32_t)(uintptr_t)(blk)))
#define fnd_sys_svc_lookup(name) ((int32_t)CPT32_SYSCALL1(FND_SYS_SVC_LOOKUP, (uint32_t)(uintptr_t)(name)))
#define fnd_sys_debug_write(buffer, len)                                              \
    ((int32_t)CPT32_SYSCALL2(FND_SYS_DEBUG_WRITE, (uint32_t)(uintptr_t)(buffer), (len)))

#endif
