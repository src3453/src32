/* Deterministic MMIO clock for testing the real scheduler accounting. */
#include <string.h>
#include "internal.h"
#undef FND_REG32
static uint32_t clock_value;
#define FND_REG32(address) clock_value
#include "sched.c"

void cpt32_halt(void) {}
void proc_activate(struct fnd_proc *proc) { (void)proc; }
void ipc_timed_out(struct fnd_thread *thread) { (void)thread; }
void ipc_pec_poll(void) {}
static struct fnd_proc test_procs[2];
static struct fnd_thread test_threads[2];

void test_reset(uint32_t clock) {
    memset(test_procs, 0, sizeof test_procs);
    memset(test_threads, 0, sizeof test_threads);
    for (unsigned i = 0; i < 2; ++i)
        test_threads[i].proc = &test_procs[i];
    cpu_last = cpu_origin = clock_value = clock;
    cpu_kernel = cpu_idle = 0;
    fnd_current = &idle;
}
void test_clock(uint32_t value) { clock_value = value; }
void test_context(uint32_t index) { fnd_current = index < 2 ? &test_threads[index] : &idle; }
uint32_t test_cycles(uint32_t index) { return test_procs[index].cpu_cycles; }
