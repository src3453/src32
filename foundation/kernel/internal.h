#ifndef FOUNDATION_KERNEL_INTERNAL_H
#define FOUNDATION_KERNEL_INTERNAL_H
#include <foundation/kernel.h>

/* Thread states and BLOCKED reasons (doc/spec_Foundation.md §2). */
enum { T_FREE, T_READY, T_RUNNING, T_BLOCKED, T_DEAD };
enum { B_NONE, B_SEND, B_REPLY, B_RECV, B_SLEEP, B_WAIT, B_BLK_IO };

struct fnd_thread {
    cpt32_ctx_frame frame; /* saved user context (continuation style) */
    uint32_t state;
    uint32_t reason;
    uint32_t timed;     /* BLOCKED with a wake-up tick */
    uint32_t wake_tick;
    uint32_t priority;
    uint32_t slice;     /* ticks left in the current time slice */
    struct fnd_proc *proc;
    struct fnd_thread *next; /* ready queue link */
};

/* sched.c */
extern struct fnd_thread *fnd_current;
struct fnd_thread *sched_alloc_thread(void);
void sched_make_ready(struct fnd_thread *thread);
void sched_timer_start(void);
void sched_timer_stop(void);
void sched_tick(void);
void sched_yield(void);
void sched_sleep(uint32_t ticks);
void sched_kill_proc(struct fnd_proc *proc);
int sched_proc_runnable(const struct fnd_proc *proc);
cpt32_ctx_frame *sched_next(void);
void sched_cpu_enter(void);
void sched_cpu_leave(void);
void sched_cpu_snapshot(uint32_t info[4]);

/* proc.c */
void proc_activate(struct fnd_proc *proc);
int proc_user_range_ok(const struct fnd_proc *proc, uint32_t addr, uint32_t len, uint32_t perm);
void proc_exit(struct fnd_proc *proc, int32_t code, uint32_t fault_epc);
int32_t proc_sys_info(const struct fnd_proc *caller, uint32_t sel, uint32_t buf, uint32_t cap);

/* MMIO (doc/spec_PeC.md, doc/spec_IRQC.md). */
#define FND_REG32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define FND_REG16(address) (*(volatile uint16_t *)(uintptr_t)(address))
#define FND_IRQC_PENDING 0xFFFF0040u
#define FND_IRQC_ENABLE 0xFFFF0044u
#define FND_IRQ_TIMER 4u

#endif
