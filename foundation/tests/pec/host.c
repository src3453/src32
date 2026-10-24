/* Real IPC dispatch + driver with controlled READY transitions. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "../../kernel/internal.h"
static uint8_t memory[16384], rx[8], tx[8];
static uint32_t ready, rx_left, tx_len, rng_reads;
static uint8_t read8(uint32_t a) {
    if (a == 0x8004001Cu) return (uint8_t)ready;
    if (a == 0x80040001u) return rx_left ? 2u : 0u;
    if (a == 0x80040000u) return rx[--rx_left];
    assert((a >= 0x80040010u && a <= 0x80040013u) ||
           (a >= 0x80040018u && a <= 0x8004001Bu));
    ++rng_reads;
    return (uint8_t)a;
}
static void write8(uint32_t a, uint8_t v) { assert(a == 0x80040000u); tx[tx_len++] = v; }
static void copy_to(const struct fnd_proc *p, uint32_t dst, const void *src, uint32_t n) {
    (void)p; assert(dst + n <= sizeof memory); memcpy(memory + dst, src, n);
}
static void copy_from(void *dst, const struct fnd_proc *p, uint32_t src, uint32_t n) {
    (void)p; assert(src + n <= sizeof memory); memcpy(dst, memory + src, n);
}
#define mm_copy_to_user copy_to
#define mm_copy_from_user copy_from
#define PEC_READ8(a) read8(a)
#define PEC_WRITE8(a, v) write8(a, v)
#include "../../kernel/pec.c"
#include "../../kernel/ipc.c"

struct fnd_thread fnd_threads[FND_MAX_THREADS];
void sched_block(struct fnd_thread *t, uint32_t reason, uint32_t timeout) {
    assert(timeout == FND_FOREVER); t->state = T_BLOCKED; t->reason = reason;
}
void sched_wake(struct fnd_thread *t, int32_t result) {
    assert(result == FND_OK); t->state = T_READY; t->reason = B_NONE;
}
int proc_user_range_ok(const struct fnd_proc *p, uint32_t a, uint32_t n, uint32_t perm) {
    (void)p; (void)perm; return a <= sizeof memory && n <= sizeof memory - a;
}
/* Other built-in drivers are outside this test. */
int32_t fb_open(uint32_t f) { (void)f; return FND_E_UNSUPPORTED; }
int32_t sgu_open(uint32_t f) { return fb_open(f); }
int32_t sgc_open(uint32_t f) { return fb_open(f); }
void fb_close(void) {}
void sgu_close(void) {}
void sgc_close(void) {}
int32_t fb_call(struct fnd_proc *p, const fnd_msg *m, fnd_msg *r, uint32_t c) {
    (void)p; (void)m; (void)r; (void)c; return FND_E_UNSUPPORTED;
}
int32_t sgu_call(struct fnd_proc *p, const fnd_msg *m, fnd_msg *r, uint32_t c) { return fb_call(p,m,r,c); }
int32_t sgc_call(struct fnd_proc *p, const fnd_msg *m, fnd_msg *r, uint32_t c) { return fb_call(p,m,r,c); }
void proc_release(struct fnd_proc *p) { (void)p; }
int mm_vm = 1;
void kmem_copy(uint32_t d, uint32_t s, uint32_t n) { (void)d; (void)s; (void)n; assert(0); }
void mm_vm_copy_to(const struct fnd_proc *p, uint32_t d, const void *s, uint32_t n) { copy_to(p,d,s,n); }
void mm_vm_copy_from(void *d, const struct fnd_proc *p, uint32_t s, uint32_t n) { copy_from(d,p,s,n); }
void mm_vm_copy(const struct fnd_proc *to, uint32_t d, const struct fnd_proc *from, uint32_t s, uint32_t n) {
    (void)to; (void)from; memmove(memory+d, memory+s, n);
}
void proc_exit(struct fnd_proc *p, int32_t code, uint32_t epc) { (void)p; (void)code; (void)epc; assert(0); }
struct fnd_thread *sched_alloc_thread(void) { assert(0); return 0; }
void sched_make_ready(struct fnd_thread *t) { (void)t; assert(0); }

static fnd_msg request, reply;
static void call(struct fnd_thread *t, uint32_t h) {
    copy_to(t->proc, 128, &request, sizeof request);
    copy_to(t->proc, 256, &reply, sizeof reply);
    t->state = T_RUNNING;
    assert(sys_ipc_call(t, h, 128, 256) == FND_OK);
    copy_from(&reply, t->proc, 256, sizeof reply);
}
int main(void) {
    struct fnd_proc p = {0};
    struct fnd_thread *t = &fnd_threads[0]; t->proc = &p;
    const char *names[] = {"dev.trng", "dev.prng", "dev.uart0"};
    p.caps = FND_CAP_SVC;
    for (unsigned i = 0; i < 3; ++i) {
        copy_to(&p, 4096, names[i], (uint32_t)strlen(names[i]) + 1u);
        assert(sys_svc_register(&p, 4096, 0) == FND_E_EXISTS);
        int32_t h = sys_svc_lookup(&p, 4096); assert(h > 0);
        assert(sys_handle_close(&p, (uint32_t)h) == FND_OK);
    }
    uint32_t factory = (uint32_t)kernel_channel(&p, 7u);
    request = (fnd_msg){.op=FND_DEV_OPEN, .a0=FND_OPEN_READ};
    call(t, factory); assert(reply.op == FND_OK && (reply.flags & FND_MSG_XFER_HANDLE));
    uint32_t trng = reply.a3;
    request = (fnd_msg){.op=FND_ST_READ, .a0=5};
    reply = (fnd_msg){.buf=512, .len=8};
    call(t, trng);
    assert(t->state == T_BLOCKED && t->reason == B_PEC_READ && rng_reads == 0);
    ipc_pec_poll(); assert(t->state == T_BLOCKED && rng_reads == 0);
    /* PRNG completes while TRNG is blocked, even with TRNG_READY == 0. */
    fnd_msg m = {.op=FND_ST_READ, .a0=7}, r = {.buf=1024};
    assert(pec_call(&p, 10, FND_OPEN_READ, &m, &r, 7) == FND_OK && r.len == 7);
    assert(rng_reads == 8 && memory[1024] == 0x10 && memory[1028] == 0x10);
    ready = 1; ipc_pec_poll(); assert(t->state == T_READY);
    copy_from(&reply, &p, 256, sizeof reply);
    assert(reply.op == FND_OK && reply.len == 5 && memory[512] == 0x18 && memory[516] == 0x18);
    request.a0 = 0; reply = (fnd_msg){.buf=512, .len=8}; ready = 0;
    call(t, trng); assert(t->state == T_RUNNING && reply.len == 0);
    /* UART RX waits, resumes with a short read, and preserves a NUL byte. */
    uint32_t uart = (uint32_t)kernel_channel(&p, 12);
    ((struct fnd_chan *)p.handles[uart-1].object)->badge = FND_OPEN_READ | FND_OPEN_WRITE;
    request.a0 = 8; reply = (fnd_msg){.buf=512, .len=8};
    call(t, uart); assert(t->state == T_BLOCKED);
    rx_left = 2; rx[1] = 'U'; rx[0] = 0; ipc_pec_poll();
    copy_from(&reply, &p, 256, sizeof reply);
    assert(t->state == T_READY && reply.len == 2 && memory[512] == 'U' && memory[513] == 0);
    request = (fnd_msg){.op=FND_ST_WRITE, .buf=512, .len=2};
    call(t, uart); assert(reply.op == FND_OK && reply.a0 == 2 && tx_len == 2 && tx[0] == 'U');
    /* A killed waiter is never accessed after its memory is reclaimed. */
    request = (fnd_msg){.op=FND_ST_READ, .a0=1}; reply = (fnd_msg){.buf=512,.len=1};
    call(t, trng); assert(t->state == T_BLOCKED); t->state = T_DEAD;
    ready = 1; uint32_t before = rng_reads; ipc_pec_poll(); assert(rng_reads == before);
    assert(pec_open(8, FND_OPEN_WRITE) == FND_E_ACCESS);
    assert(pec_open(10, 0) == FND_E_BAD_ARG);
    assert(pec_open(12, FND_OPEN_WRITE) == FND_OK);
    m.op = FND_ST_WRITE; m.len = 0;
    assert(pec_call(&p, 8, FND_OPEN_READ, &m, &r, 0) == FND_E_ACCESS);
    m.op = FND_ST_STAT;
    assert(pec_call(&p, 8, FND_OPEN_READ, &m, &r, 63) == FND_E_LIMIT);
    assert(pec_call(&p, 8, FND_OPEN_READ, &m, &r, 64) == FND_OK);
    fnd_stat_t st; copy_from(&st, &p, r.buf, sizeof st);
    assert(st.type == FND_TYPE_DEVICE && st.flags == FND_STAT_READ_ONLY && st.mode == 0x24);
    m.op = FND_ST_SEEK;
    assert(pec_call(&p, 12, 3, &m, &r, 0) == FND_E_UNSUPPORTED);
    /* IPC rejects bad reply buffers before consuming hardware state. */
    request = (fnd_msg){.op=FND_ST_READ,.a0=1};
    reply = (fnd_msg){.buf=sizeof memory,.len=1};
    copy_to(&p, 128, &request, sizeof request);
    copy_to(&p, 256, &reply, sizeof reply);
    before = rng_reads;
    assert(sys_ipc_call(t, trng, 128, 256) == FND_E_ACCESS && rng_reads == before);
    request.flags = FND_MSG_XFER_HANDLE; request.a3 = trng;
    reply = (fnd_msg){.buf=512,.len=1};
    copy_to(&p, 128, &request, sizeof request);
    copy_to(&p, 256, &reply, sizeof reply);
    assert(sys_ipc_call(t, trng, 128, 256) == FND_E_BAD_ARG && rng_reads == before);
    puts("pec host passed");
    return 0;
}
