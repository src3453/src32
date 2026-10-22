/* Trap handling (doc/spec_Foundation.md §3.4, §7, §9, §10.2). kernel_trap runs
 * on the single kernel stack with interrupts off; blocking system calls
 * store their result in the thread's frame and switch away. */
#include "internal.h"

#define UART_DATA 0x80040000u
#define TIMER_PENDING 0x80040188u
#define RTC_SECONDS_LO 0x80040208u
#define RTC_SECONDS_HI 0x8004020Cu
#define CAUSE_SYSCALL 0x80030000u
#define CAUSE_SYSCALL_CONTEXT(cause) (((cause) >> 8) & 0xFFu)

/* HALT, HALT: an S frame resumed here with IE = 0 stops the CPU with R1 kept
 * (a C function would clobber R1 in its prologue). */
static const uint32_t halt_code[2] = {0xFC000000u, 0xFC000000u};
static uint32_t halt_stack[16] __attribute__((aligned(8)));
static cpt32_ctx_frame halt_frame;
static fnd_tick_hook tick_hook;
static uint32_t stop_code;

void fnd_set_tick_hook(fnd_tick_hook hook) { tick_hook = hook; }

void fnd_debug_putc(uint32_t c) { *(volatile uint8_t *)(uintptr_t)UART_DATA = (uint8_t)c; }

void fnd_debug_puts(const char *s) {
    while (*s)
        fnd_debug_putc((uint8_t)*s++);
}

void fnd_debug_hex(uint32_t value) {
    for (int digit = 0; digit < 8; ++digit) {
        uint32_t nibble = (value >> (28 - 4 * digit)) & 0xFu;
        fnd_debug_putc(nibble < 10u ? '0' + nibble : 'A' + nibble - 10u);
    }
}

static cpt32_ctx_frame *halt_machine(uint32_t code) {
    sched_timer_stop();
    for (unsigned i = 0; i < 32u; ++i)
        halt_frame.r[i] = 0;
    halt_frame.r[1] = code;
    halt_frame.r[28] = (uint32_t)(uintptr_t)&halt_stack[16];
    halt_frame.epc = (uint32_t)(uintptr_t)halt_code;
    halt_frame.status = 0; /* S mode, IE = 0 after IRET */
    return &halt_frame;
}

/* Drain PENDING & ENABLE (IRQC §5). Only the timer is enabled in phase 1;
 * any other line is masked and acknowledged. A tick that fires between the
 * timer ACK and the IRQC W1C keeps the timer source high without a new edge,
 * so the sticky timer pending bit is checked again before returning. */
static void handle_irqs(void) {
    for (;;) {
        uint32_t active = FND_REG16(FND_IRQC_PENDING) & FND_REG16(FND_IRQC_ENABLE);
        if (!active && (FND_REG32(TIMER_PENDING) & 1u))
            active = 1u << FND_IRQ_TIMER;
        if (!active)
            return;
        uint32_t line = 0;
        while (!(active & (1u << line)))
            ++line;
        if (line == FND_IRQ_TIMER) {
            sched_tick();
            if (tick_hook && !stop_code)
                stop_code = tick_hook(fnd_ticks());
        } else {
            FND_REG16(FND_IRQC_ENABLE) = FND_REG16(FND_IRQC_ENABLE) & ~(1u << line);
        }
        FND_REG16(FND_IRQC_PENDING) = (uint16_t)(1u << line);
    }
}

static int32_t debug_write(struct fnd_proc *proc, uint32_t buffer, uint32_t len) {
    if (len > FND_DEBUG_WRITE_MAX)
        len = FND_DEBUG_WRITE_MAX;
    if (!proc_user_range_ok(proc, buffer, len, CPT32_PERM_R))
        return FND_E_ACCESS;
    for (uint32_t i = 0; i < len; ++i)
        fnd_debug_putc(*(volatile uint8_t *)(uintptr_t)(buffer + i));
    return (int32_t)len;
}

static int32_t time_get(uint32_t selector) {
    switch (selector) {
    case FND_TIME_TICK_LO:
        return (int32_t)fnd_ticks();
    case FND_TIME_TICK_HI:
        return 0; /* the phase 1 tick counter is 32-bit */
    case FND_TIME_TICK_HZ:
        return (int32_t)FND_TICK_HZ;
    case FND_TIME_RTC_LO:
        return (int32_t)FND_REG32(RTC_SECONDS_LO); /* latches HI */
    case FND_TIME_RTC_HI:
        return (int32_t)FND_REG32(RTC_SECONDS_HI);
    default:
        return FND_E_BAD_ARG;
    }
}

static void handle_syscall(cpt32_ctx_frame *frame, uint32_t cause) {
    struct fnd_thread *thread = fnd_current;
    struct fnd_proc *proc = thread->proc;
    uint32_t a1 = frame->r[1], a2 = frame->r[2], a3 = frame->r[3];
    int32_t result;
    if (CAUSE_SYSCALL_CONTEXT(cause) != 0) {
        frame->r[1] = (uint32_t)FND_E_BAD_CALL;
        return;
    }
    switch (CPT32_CAUSE_SYSCALL_NUMBER(cause)) {
    case FND_SYS_YIELD:
        sched_yield();
        result = FND_OK;
        break;
    case FND_SYS_SLEEP:
        sched_sleep(a1); /* stores the result itself when it blocks */
        result = FND_OK;
        break;
    case FND_SYS_THREAD_EXIT: /* one thread per process in phase 1 */
    case FND_SYS_PROC_EXIT:
        proc_exit(proc, (int32_t)a1, 0);
        return;
    case FND_SYS_TIME_GET:
        result = time_get(a1);
        break;
    case FND_SYS_DEBUG_WRITE:
        result = debug_write(proc, a1, a2);
        break;
    case FND_SYS_INFO:
        result = proc_sys_info(proc, a1, a2, a3);
        break;
    case FND_SYS_IPC_CALL: case FND_SYS_IPC_RECV:
    case FND_SYS_IPC_REPLY: case FND_SYS_IPC_REPLY_RECV: case FND_SYS_PORT_CREATE:
    case FND_SYS_CHAN_CREATE: case FND_SYS_HANDLE_CLOSE: case FND_SYS_HANDLE_DUP:
    case FND_SYS_THREAD_CREATE: case FND_SYS_WAIT: case FND_SYS_PROC_SPAWN:
    case FND_SYS_PROC_KILL: case FND_SYS_PROC_STARTUP: case FND_SYS_MEM_GROW:
    case FND_SYS_IRQ_BIND: case FND_SYS_IRQ_ACK: case FND_SYS_BLK_IO:
    case FND_SYS_SVC_REGISTER: case FND_SYS_SVC_LOOKUP:
        result = FND_E_UNSUPPORTED; /* defined by the ABI, later phases */
        break;
    default:
        result = FND_E_BAD_CALL;
        break;
    }
    if (thread->state == T_RUNNING)
        frame->r[1] = (uint32_t)result;
}

cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *frame) {
    uint32_t cause = frame->r[0];
    if (!CPT32_CAUSE_IS_EXCEPTION(cause)) {
        handle_irqs();
    } else if ((frame->status & CPT32_STATUS_PU) == 0u) {
        kernel_panic(cause, frame->epc); /* exception in the idle thread */
    } else if ((cause & 0xFFFF0000u) == CAUSE_SYSCALL) {
        handle_syscall(frame, cause);
    } else {
        /* A user fault ends the process; the exit code is CAUSE as i32. */
        proc_exit(fnd_current->proc, (int32_t)cause, frame->epc);
    }
    if (stop_code)
        return halt_machine(stop_code);
    return sched_next();
}

void kernel_panic(uint32_t cause, uint32_t epc) {
    sched_timer_stop();
    fnd_debug_puts("\r\nFoundation: kernel panic cause=");
    fnd_debug_hex(cause);
    fnd_debug_puts(" epc=");
    fnd_debug_hex(epc);
    fnd_debug_puts("\r\n");
    cpt32_jump((uint32_t)(uintptr_t)halt_code, 0x200u | (cause >> 16 & 0xFFu),
               (uint32_t)(uintptr_t)&halt_stack[16]);
}

void fnd_kernel_run(void) {
    sched_timer_start();
    cpt32_ctx_resume(sched_next());
}
