#ifndef FOUNDATION_ABI_H
#define FOUNDATION_ABI_H
#include <stdint.h>

#include <foundation/errors.h>

/* Foundation kernel ABI v1 (doc/spec_Foundation.md). */
#define FND_ABI_MAJOR 1u
#define FND_ABI_MINOR 0u

/* System call numbers (§9): SYSCALL n with context 0, arguments in R1..R4,
 * i32 result in R1; every other register is preserved. */
#define FND_SYS_INFO 0x00u
#define FND_SYS_IPC_CALL 0x01u
#define FND_SYS_IPC_RECV 0x02u
#define FND_SYS_IPC_REPLY 0x03u
#define FND_SYS_IPC_REPLY_RECV 0x04u
#define FND_SYS_PORT_CREATE 0x05u
#define FND_SYS_CHAN_CREATE 0x06u
#define FND_SYS_HANDLE_CLOSE 0x07u
#define FND_SYS_HANDLE_DUP 0x08u
#define FND_SYS_THREAD_CREATE 0x09u
#define FND_SYS_THREAD_EXIT 0x0Au
#define FND_SYS_YIELD 0x0Bu
#define FND_SYS_SLEEP 0x0Cu
#define FND_SYS_WAIT 0x0Du
#define FND_SYS_PROC_SPAWN 0x10u
#define FND_SYS_PROC_EXIT 0x11u
#define FND_SYS_PROC_KILL 0x12u
#define FND_SYS_PROC_STARTUP 0x13u
#define FND_SYS_PROC_FORK 0x14u
#define FND_SYS_MEM_GROW 0x18u
#define FND_SYS_IRQ_BIND 0x20u
#define FND_SYS_IRQ_ACK 0x21u
#define FND_SYS_BLK_IO 0x28u
#define FND_SYS_TIME_GET 0x30u
#define FND_SYS_SVC_REGISTER 0x31u
#define FND_SYS_SVC_LOOKUP 0x32u
#define FND_SYS_DEBUG_WRITE 0x3Fu

/* Timeouts are in ticks (§7.1). */
#define FND_FOREVER 0xFFFFFFFFu

/* time_get selectors (§9). */
#define FND_TIME_TICK_LO 0u
#define FND_TIME_TICK_HI 1u
#define FND_TIME_TICK_HZ 2u
#define FND_TIME_RTC_LO 3u
#define FND_TIME_RTC_HI 4u

/* Priorities (§10.1): smaller is more urgent. */
#define FND_PRIO_DRIVER 0u
#define FND_PRIO_SERVICE 1u
#define FND_PRIO_APP 2u
#define FND_PRIO_IDLE 3u

/* debug_write writes at most this many bytes per call (§9). */
#define FND_DEBUG_WRITE_MAX 256u

#endif
