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

/* Drain PENDING & ENABLE (IRQC §5). Bound lines stay pending until irq_ack;
 * any unbound line is masked and acknowledged. A tick that fires between the
 * timer ACK and the IRQC W1C keeps the timer source high without a new edge,
 * so the sticky timer pending bit is checked again before returning. */
static void handle_irqs(void) {
    for (;;) {
        uint32_t active = FND_REG16(FND_IRQC_PENDING) & FND_REG16(FND_IRQC_ENABLE);
        if (!active && (FND_REG32(TIMER_PENDING) & 1u))
            active = 1u << FND_IRQ_TIMER;
        if (!active && blk_pending())
            active = 1u << FND_IRQ_IDC; /* the same race for the IDC level */
        if (!active)
            return;
        uint32_t line = 0;
        while (!(active & (1u << line)))
            ++line;
        if (line == FND_IRQ_TIMER) {
            sched_tick();
            if (tick_hook && !stop_code)
                stop_code = tick_hook(fnd_ticks());
        } else if (line == FND_IRQ_IDC) {
            blk_irq();
        } else if (ipc_irq_notify(line)) {
            continue; /* the service owns the W1C */
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
    uint8_t bytes[FND_DEBUG_WRITE_MAX];
    mm_copy_from_user(bytes, proc, buffer, len);
    for (uint32_t i = 0; i < len; ++i)
        fnd_debug_putc(bytes[i]);
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
    case FND_SYS_THREAD_EXIT:
        sys_thread_exit(thread, (int32_t)a1);
        return;
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
    case FND_SYS_IPC_CALL:
        result = sys_ipc_call(thread, a1, a2, a3);
        break;
    case FND_SYS_IPC_RECV:
        result = sys_ipc_recv(thread, a1, a2);
        break;
    case FND_SYS_IPC_REPLY:
        result = sys_ipc_reply(thread, a1, a2);
        break;
    case FND_SYS_IPC_REPLY_RECV:
        result = sys_ipc_reply_recv(thread, a1, a2, a3, frame->r[4]);
        break;
    case FND_SYS_PORT_CREATE:
        result = sys_port_create(proc);
        break;
    case FND_SYS_CHAN_CREATE:
        result = sys_chan_create(proc, a1, a2);
        break;
    case FND_SYS_HANDLE_CLOSE:
        result = sys_handle_close(proc, a1);
        break;
    case FND_SYS_HANDLE_DUP:
        result = sys_handle_dup(proc, a1);
        break;
    case FND_SYS_WAIT:
        result = sys_wait(thread, a1, a2, a3);
        break;
    case FND_SYS_PROC_FORK:
        result = proc_fork(thread);
        break;
    case FND_SYS_PROC_SPAWN:
        result = proc_spawn(proc, a1);
        break;
    case FND_SYS_PROC_KILL:
        result = sys_proc_kill(proc, a1);
        break;
    case FND_SYS_PROC_STARTUP:
        result = proc_startup(proc, a1, a2);
        break;
    case FND_SYS_MEM_GROW:
        result = proc_mem_grow(proc, a1);
        break;
    case FND_SYS_SVC_REGISTER:
        result = sys_svc_register(proc, a1, a2);
        break;
    case FND_SYS_SVC_LOOKUP:
        result = sys_svc_lookup(proc, a1);
        break;
    case FND_SYS_BLK_IO:
        result = sys_blk_io(thread, a1);
        break;
    case FND_SYS_THREAD_CREATE:
        result = sys_thread_create(thread, a1, a2, a3);
        break;
    case FND_SYS_IRQ_BIND:
        result = sys_irq_bind(proc, a1, a2);
        break;
    case FND_SYS_IRQ_ACK:
        result = sys_irq_ack(proc, a1);
        break;
    default:
        result = FND_E_BAD_CALL;
        break;
    }
    if (thread->state == T_RUNNING)
        frame->r[1] = (uint32_t)result;
}

cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *frame) {
    sched_cpu_enter();
    uint32_t cause = frame->r[0];
    if (!CPT32_CAUSE_IS_EXCEPTION(cause)) {
        handle_irqs();
    } else if ((frame->status & CPT32_STATUS_PU) == 0u) {
        kernel_panic(cause, frame->epc); /* exception in the idle thread */
    } else if ((cause & 0xFFFF0000u) == CAUSE_SYSCALL) {
        handle_syscall(frame, cause);
    } else if (mm_vm && cause == 0x8006000Au &&
               mm_vm_cow(fnd_current->proc, cpt32_mfsr(CPT32_SR_VFAULT)) == FND_OK) {
        /* Retry the precise store at the unchanged EPC with a private page. */
    } else {
        /* A user fault ends the process; the exit code is CAUSE as i32. */
        proc_exit(fnd_current->proc, (int32_t)cause, frame->epc);
    }
    if (stop_code)
        return halt_machine(stop_code);
    cpt32_ctx_frame *next = sched_next();
    sched_cpu_leave();
    return next;
}

/* Panic screen: the kernel writes the PCG text planes itself, because the
 * console service and IPC cannot be trusted any more. The layout is the one
 * the console service sets up (SCREEN_MODE 1, 80x30 cells, scroll 0). */
#define PANIC_COLS 80u
#define PANIC_ROWS 30u
#define PANIC_VRAM 0x10000000u
#define PANIC_FG 0x1000u
#define PANIC_BG 0x2000u
#define PANIC_REG8(a) (*(volatile uint8_t *)(uintptr_t)(a))

static uint32_t panic_col, panic_row, panic_entered;

static void panic_putc(uint32_t c) {
    if (c == '\n') {
        panic_col = 0;
        ++panic_row;
        return;
    }
    if (panic_col >= PANIC_COLS || panic_row >= PANIC_ROWS)
        return; /* no scrolling: the message is short and the first lines matter */
    uint32_t cell = PANIC_VRAM + panic_row * PANIC_COLS + panic_col++;
    PANIC_REG8(cell) = (uint8_t)c;
}

static void panic_puts(const char *s) {
    while (*s)
        panic_putc((uint8_t)*s++);
}

static void panic_hex(uint32_t value) {
    for (int digit = 0; digit < 8; ++digit) {
        uint32_t nibble = (value >> (28 - 4 * digit)) & 0xFu;
        panic_putc(nibble < 10u ? '0' + nibble : 'A' + nibble - 10u);
    }
}

static const char *panic_cause_name(uint32_t cause) {
    switch (cause >> 16) {
    case 0x8001u: return "illegal instruction";
    case 0x8002u: return "bus error";
    case 0x8003u: return "SYSCALL in kernel";
    case 0x8004u: return "privileged instruction in U";
    case 0x8005u: return "protection fault";
    default: return cause & 0x80000000u ? "unknown exception" : "IRQ in kernel";
    }
}

static void panic_screen(uint32_t cause, uint32_t epc) {
    PANIC_REG8(0x80000000u) = 1; /* common DISPLAY_ENABLE */
    PANIC_REG8(0x80000001u) = 1; /* DISPLAY_MODE: PCG text */
    PANIC_REG8(0x80000004u) = 0; /* FORMAT: back from a bitmap lease */
    PANIC_REG8(0x8000F00Bu) = 0; /* OVERLAY */
    PANIC_REG8(0x8000F000u) = 0; /* PCG enabled */
    PANIC_REG8(0x8000F003u) = 1; /* SCREEN_MODE 1 */
    PANIC_REG8(0x8000F007u) = 0; /* no cursor */
    PANIC_REG8(0x8000F00Cu) = 0; /* scroll 0, low byte first */
    PANIC_REG8(0x8000F00Du) = 0;
    PANIC_REG8(0x8000F00Eu) = 0;
    PANIC_REG8(0x8000F00Fu) = 0;
    for (uint32_t i = 0; i < PANIC_COLS * PANIC_ROWS; ++i) {
        PANIC_REG8(PANIC_VRAM + i) = ' ';
        PANIC_REG8(PANIC_VRAM + PANIC_FG + i) = 0x3Fu;
        PANIC_REG8(PANIC_VRAM + PANIC_BG + i) = 0x00u;
    }
    panic_col = panic_row = 0;
    panic_puts("Foundation: kernel panic\n");
    panic_puts("cause=");
    panic_hex(cause);
    panic_puts(" (");
    panic_puts(panic_cause_name(cause));
    panic_puts(")\nepc=");
    panic_hex(epc);
    panic_puts(" ticks=");
    panic_hex(fnd_ticks());
    panic_puts("\nthread: ");
    struct fnd_thread *thread = fnd_current;
    if (thread && thread->proc) {
        panic_puts("pid ");
        panic_hex(thread->proc->pid);
        panic_putc(' ');
        for (uint32_t i = 0; i < 15u && thread->proc->name[i]; ++i)
            panic_putc((uint8_t)thread->proc->name[i]);
    } else {
        panic_puts("idle or none");
    }
    /* The stub keeps the context saved at the last trap; for an exception in
     * the kernel itself that is the interrupted thread's, not the kernel's. */
    const cpt32_ctx_frame *frame = cpt32_ctx_current;
    if (frame) {
        panic_puts("\nlast saved context: epc=");
        panic_hex(frame->epc);
        panic_puts(" status=");
        panic_hex(frame->status);
        for (uint32_t n = 1; n < 32u; ++n) {
            if ((n - 1u) % 4u == 0u)
                panic_putc('\n');
            else
                panic_putc(' ');
            panic_putc('R');
            panic_putc('0' + n / 10u);
            panic_putc('0' + n % 10u);
            panic_putc('=');
            panic_hex(frame->r[n]);
        }
    }
}

void kernel_panic(uint32_t cause, uint32_t epc) {
    sched_timer_stop();
    /* A fault while reporting (an unmapped UART, for example) re-enters
     * here from the stub; stop without touching the devices again. */
    if (!panic_entered) {
        panic_entered = 1;
        panic_screen(cause, epc);
        fnd_debug_puts("\r\nFoundation: kernel panic cause=");
        fnd_debug_hex(cause);
        fnd_debug_puts(" epc=");
        fnd_debug_hex(epc);
        fnd_debug_puts("\r\n");
    }
    cpt32_jump((uint32_t)(uintptr_t)halt_code, 0x200u | (cause >> 16 & 0xFFu),
               (uint32_t)(uintptr_t)&halt_stack[16]);
}

void fnd_kernel_run(void) {
    mm_init();
    sched_timer_start();
    cpt32_ctx_frame *next = sched_next();
    sched_cpu_leave();
    cpt32_ctx_resume(next);
}
