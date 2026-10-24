/* Scheduler (doc/spec_Foundation.md §2, §10.1): four priorities with FIFO
 * round robin inside each, 100 Hz tick on PeC timer ch0 (IRQ4) and a
 * 1-tick (10 ms) time slice. The kernel itself is never preempted. */
#include "internal.h"

#define TIMER_ENABLE 0x80040184u
#define TIMER_ACK 0x8004018Cu
#define TIMER0_CTRL 0x80040400u
#define TIMER0_COMPARE_LO 0x80040410u
#define TIMER0_COMPARE_HI 0x80040414u
#define TIMER0_RELOAD_LO 0x80040418u
#define TIMER0_RELOAD_HI 0x8004041Cu
#define TIMER0_PRESCALER 0x80040420u
#define TIMER_PERIODIC_IRQ 0x0Du /* periodic | ENABLE | IRQ_ENABLE */
#define TIMER_RESET 0x10u
#define PRIORITIES 3u            /* queued priorities; IDLE is not queued */

struct fnd_thread *fnd_current;
struct fnd_thread fnd_threads[FND_MAX_THREADS];
#define threads fnd_threads
static struct fnd_thread idle;
static uint32_t idle_stack[64] __attribute__((aligned(8)));
static struct fnd_thread *queue_head[PRIORITIES], *queue_tail[PRIORITIES];
static uint32_t ticks, switches, need_resched;

/* ch1 is a free-running 48 MHz accounting clock. C trap boundaries leave
 * the assembly save/restore cost charged to the resumed context. */
#define CPU_CLOCK_CTRL 0x80040440u
#define CPU_CLOCK_LO 0x80040448u
#define CPU_CLOCK_PRESCALER 0x80040460u
static uint32_t cpu_last, cpu_origin, cpu_kernel, cpu_idle;

void sched_cpu_enter(void) {
    uint32_t now = FND_REG32(CPU_CLOCK_LO);
    uint32_t elapsed = now - cpu_last;
    if (fnd_current == &idle)
        cpu_idle += elapsed;
    else
        fnd_current->proc->cpu_cycles += elapsed;
    cpu_last = now;
}

void sched_cpu_leave(void) {
    uint32_t now = FND_REG32(CPU_CLOCK_LO);
    cpu_kernel += now - cpu_last;
    cpu_last = now;
}

void sched_cpu_snapshot(uint32_t info[4]) {
    sched_cpu_leave(); /* include this syscall up to the snapshot */
    info[0] = cpu_last - cpu_origin;
    info[1] = cpu_kernel;
    info[2] = cpu_idle;
    info[3] = 48000000u;
}

uint32_t fnd_ticks(void) { return ticks; }
uint32_t fnd_context_switches(void) { return switches; }

/* The idle thread: S mode, IE = 1, HALT until the next IRQ traps out. It must
 * not touch R30 (the trap stub address). */
static void idle_loop(void) {
    for (;;)
        cpt32_halt();
}

static void init_idle(void) {
    for (unsigned i = 0; i < 32u; ++i)
        idle.frame.r[i] = 0;
    idle.frame.r[28] = (uint32_t)(uintptr_t)&idle_stack[64];
    idle.frame.epc = (uint32_t)(uintptr_t)idle_loop;
    idle.frame.status = CPT32_STATUS_PIE;
    idle.priority = FND_PRIO_IDLE;
    idle.state = T_RUNNING;
    fnd_current = &idle;
}

struct fnd_thread *sched_alloc_thread(void) {
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        struct fnd_thread *thread = &threads[i];
        if ((thread->state == T_FREE || thread->state == T_DEAD) && thread != fnd_current &&
            thread->refs == 0) {
            thread->state = T_FREE;
            thread->wait_proc = 0;
            thread->wait_thread = 0;
            thread->reason = B_NONE;
            thread->timed = 0;
            thread->next = 0;
            thread->slice = FND_SLICE_TICKS;
            return thread;
        }
    }
    return 0;
}

static void enqueue(struct fnd_thread *thread, int at_head) {
    uint32_t p = thread->priority;
    thread->state = T_READY;
    if (at_head) {
        thread->next = queue_head[p];
        queue_head[p] = thread;
        if (!queue_tail[p])
            queue_tail[p] = thread;
    } else {
        thread->next = 0;
        if (queue_tail[p])
            queue_tail[p]->next = thread;
        else
            queue_head[p] = thread;
        queue_tail[p] = thread;
    }
}

static struct fnd_thread *dequeue(void) {
    for (uint32_t p = 0; p < PRIORITIES; ++p) {
        while (queue_head[p]) {
            struct fnd_thread *thread = queue_head[p];
            queue_head[p] = thread->next;
            if (!queue_head[p])
                queue_tail[p] = 0;
            thread->next = 0;
            if (thread->state == T_READY)
                return thread;
        }
    }
    return 0;
}

/* The single place that unblocks a thread: result is its syscall's R1. */
void sched_wake(struct fnd_thread *thread, int32_t result) {
    thread->frame.r[1] = (uint32_t)result;
    thread->reason = B_NONE;
    thread->timed = 0;
    enqueue(thread, 0);
    if (thread->priority < fnd_current->priority)
        need_resched = 1;
}

void sched_make_ready(struct fnd_thread *thread) {
    if (thread->priority >= PRIORITIES)
        thread->priority = FND_PRIO_APP;
    enqueue(thread, 0);
    if (thread->priority < fnd_current->priority)
        need_resched = 1;
}

void sched_timer_start(void) {
    if (!fnd_current)
        init_idle();
    FND_REG32(TIMER0_CTRL) = TIMER_RESET;
    FND_REG32(TIMER0_PRESCALER) = 0;
    FND_REG32(TIMER0_COMPARE_HI) = 0;
    FND_REG32(TIMER0_COMPARE_LO) = FND_TICK_CYCLES;
    FND_REG32(TIMER0_RELOAD_HI) = 0;
    FND_REG32(TIMER0_RELOAD_LO) = FND_TICK_CYCLES;
    FND_REG32(TIMER_ACK) = 1u;
    FND_REG32(TIMER0_CTRL) = TIMER_PERIODIC_IRQ;
    FND_REG32(CPU_CLOCK_CTRL) = TIMER_RESET;
    FND_REG32(CPU_CLOCK_PRESCALER) = 0;
    FND_REG32(CPU_CLOCK_CTRL) = 6u; /* free-running, enabled, no IRQ */
    FND_REG32(TIMER_ENABLE) = 3u;
    cpu_last = cpu_origin = FND_REG32(CPU_CLOCK_LO);
    FND_REG16(FND_IRQC_PENDING) = 1u << FND_IRQ_TIMER;
    FND_REG16(FND_IRQC_ENABLE) = 1u << FND_IRQ_TIMER;
}

void sched_timer_stop(void) {
    FND_REG16(FND_IRQC_ENABLE) = 0;
    FND_REG32(TIMER0_CTRL) = 0;
    FND_REG32(TIMER_ENABLE) = 0;
    FND_REG32(TIMER_ACK) = 1u;
    FND_REG16(FND_IRQC_PENDING) = 1u << FND_IRQ_TIMER;
}

void sched_tick(void) {
    FND_REG32(TIMER_ACK) = 1u;
    ++ticks;
    ipc_pec_poll();
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        struct fnd_thread *thread = &threads[i];
        if (thread->state == T_BLOCKED && thread->timed && (int32_t)(ticks - thread->wake_tick) >= 0)
            ipc_timed_out(thread); /* OK for sleep, TIMEOUT otherwise (§7.1) */
    }
    if (fnd_current != &idle && fnd_current->state == T_RUNNING) {
        if (fnd_current->slice)
            --fnd_current->slice;
        if (!fnd_current->slice)
            need_resched = 1;
    }
}

void sched_yield(void) {
    fnd_current->slice = 0; /* to the tail of its queue */
    need_resched = 1;
}

/* Blocks thread (the current one) for reason; timeout in ticks (§7.1). */
void sched_block(struct fnd_thread *thread, uint32_t reason, uint32_t timeout) {
    thread->state = T_BLOCKED;
    thread->reason = reason;
    thread->timed = timeout != FND_FOREVER;
    thread->wake_tick = ticks + timeout;
    thread->frame.r[1] = FND_OK;
}

void sched_sleep(uint32_t count) {
    if (count == 0) {
        sched_yield();
        return;
    }
    sched_block(fnd_current, B_SLEEP, count);
}

void sched_kill_proc(struct fnd_proc *proc) {
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        if (threads[i].proc == proc && threads[i].state != T_FREE)
            threads[i].state = T_DEAD; /* dequeue skips it */
    }
}

/* Some thread of proc is ready or running (sys_info state 0). */
int sched_proc_runnable(const struct fnd_proc *proc) {
    for (unsigned i = 0; i < FND_MAX_THREADS; ++i) {
        if (threads[i].proc == proc && (threads[i].state == T_READY || threads[i].state == T_RUNNING))
            return 1;
    }
    return 0;
}

/* Chooses the thread to resume after a trap and returns its frame. */
cpt32_ctx_frame *sched_next(void) {
    struct fnd_thread *current = fnd_current;
    if (current->state == T_RUNNING) {
        if (current == &idle) {
            need_resched = 1; /* leave idle as soon as anything is ready */
        } else if (!need_resched) {
            return &current->frame;
        } else if (current->slice == 0) {
            current->slice = FND_SLICE_TICKS;
            enqueue(current, 0);
        } else {
            enqueue(current, 1); /* preempted by a more urgent thread */
        }
    }
    need_resched = 0;
    struct fnd_thread *next = dequeue();
    if (!next)
        next = &idle;
    next->state = T_RUNNING;
    if (next != current)
        ++switches;
    if (next != &idle)
        proc_activate(next->proc);
    fnd_current = next;
    return &next->frame;
}
