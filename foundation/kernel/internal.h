#ifndef FOUNDATION_KERNEL_INTERNAL_H
#define FOUNDATION_KERNEL_INTERNAL_H
#include <foundation/kernel.h>
#include "mm.h"

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
    /* IPC (ipc.c). A caller in SEND waits in port's queue (ordered by seq)
     * and in REPLY holds token generation gen; a receiver in RECV waits on
     * port with its fnd_recv at rcv; a WAIT waits for wait_proc. */
    struct fnd_port *port;
    uint32_t seq, gen;
    uint32_t badge;
    fnd_msg req;          /* the request as sent */
    uint32_t rep;         /* caller's reply fnd_msg */
    uint32_t rep_buf, rep_cap;
    uint32_t rcv;         /* receiver's fnd_recv */
    struct fnd_proc *wait_proc;
    uint32_t code_ptr;
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
void sched_wake(struct fnd_thread *thread, int32_t result);
void sched_block(struct fnd_thread *thread, uint32_t reason, uint32_t timeout);
extern struct fnd_thread fnd_threads[FND_MAX_THREADS];
int sched_proc_runnable(const struct fnd_proc *proc);
cpt32_ctx_frame *sched_next(void);
void sched_cpu_enter(void);
void sched_cpu_leave(void);
void sched_cpu_snapshot(uint32_t info[4]);

/* proc.c */
void proc_exit(struct fnd_proc *proc, int32_t code, uint32_t fault_epc);
int32_t proc_spawn(struct fnd_proc *caller, uint32_t spawn);
int32_t proc_startup(const struct fnd_proc *proc, uint32_t buf, uint32_t cap);
int32_t proc_mem_grow(struct fnd_proc *proc, uint32_t delta);
void proc_release(struct fnd_proc *proc); /* a PROCESS handle closed */
int32_t proc_sys_info(const struct fnd_proc *caller, uint32_t sel, uint32_t buf, uint32_t cap);

/* ipc.c: handles (§6), IPC (§8) and services (§8.6). */
int32_t handle_add(struct fnd_proc *proc, uint32_t kind, void *object);
void handle_close_all(struct fnd_proc *proc);
uint32_t handle_free_count(const struct fnd_proc *proc);
int32_t handle_dup_into(struct fnd_proc *from, uint32_t h, struct fnd_proc *to);
void ipc_proc_exited(struct fnd_proc *proc); /* wakes WAIT on it */
void ipc_timed_out(struct fnd_thread *thread);
int32_t sys_handle_close(struct fnd_proc *proc, uint32_t h);
int32_t sys_handle_dup(struct fnd_proc *proc, uint32_t h);
int32_t sys_port_create(struct fnd_proc *proc);
int32_t sys_chan_create(struct fnd_proc *proc, uint32_t port, uint32_t badge);
int32_t sys_ipc_call(struct fnd_thread *thread, uint32_t ch, uint32_t req, uint32_t rep);
int32_t sys_ipc_recv(struct fnd_thread *thread, uint32_t port, uint32_t rcv);
int32_t sys_ipc_reply(struct fnd_thread *thread, uint32_t token, uint32_t msg);
int32_t sys_ipc_reply_recv(struct fnd_thread *thread, uint32_t token, uint32_t msg, uint32_t port,
                           uint32_t rcv);
int32_t sys_svc_register(struct fnd_proc *proc, uint32_t name, uint32_t port);
int32_t sys_svc_lookup(struct fnd_proc *proc, uint32_t name);
int32_t sys_wait(struct fnd_thread *thread, uint32_t h, uint32_t timeout, uint32_t code_ptr);
int32_t sys_proc_kill(struct fnd_proc *proc, uint32_t h);

/* MMIO (doc/spec_PeC.md, doc/spec_IRQC.md). */
#define FND_REG32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#define FND_REG16(address) (*(volatile uint16_t *)(uintptr_t)(address))
#define FND_IRQC_PENDING 0xFFFF0040u
#define FND_IRQC_ENABLE 0xFFFF0044u
#define FND_IRQ_TIMER 4u
#define FND_IRQ_IDC 2u

/* blk.c: blk_io (§10.4). */
int32_t sys_blk_io(struct fnd_thread *thread, uint32_t blk);
void blk_irq(void);
int blk_pending(void);
void blk_proc_exited(struct fnd_proc *proc);

#endif
