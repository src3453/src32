/* Supervisor-mode kernel for the kernel_trap context-switch stub
 * (doc/spec_Foundation.md §3). Three phases:
 *   1. two cooperative user tasks switch on every SYS_YIELD;
 *   2. an S-mode idle context halts until the timer IRQ traps out of it;
 *   3. two user tasks that never yield are preempted by the timer.
 * The result (42 on success) ends in R1 at a HALT with interrupts off. */
#include <cpt32/cpu.h>

#include "trap_demo.h"

enum { TASK_A, TASK_B, TASK_C, TASK_D, TASKS };

/* PeC quad timer channel 0 (doc/spec_PeC.md) and IRQC (doc/spec_IRQC.md). */
#define TIMER_ENABLE 0x80040184u
#define TIMER_ACK 0x8004018Cu
#define TIMER0_CTRL 0x80040400u
#define TIMER0_COMPARE_LO 0x80040410u
#define TIMER0_COMPARE_HI 0x80040414u
#define TIMER0_RELOAD_LO 0x80040418u
#define TIMER0_RELOAD_HI 0x8004041Cu
#define TIMER0_PRESCALER 0x80040420u
#define TIMER_PERIODIC_IRQ 0x0Du /* periodic | ENABLE | IRQ_ENABLE */
#define TIMER_PERIOD 20000u      /* 48 MHz cycles */
#define IRQC_PENDING 0xFFFF0040u
#define IRQC_ENABLE 0xFFFF0044u
#define IRQ_TIMER 4u

#define REG32(address) (*(volatile uint32_t *)(address))
#define REG16(address) (*(volatile uint16_t *)(address))

static const uint32_t seeds[2] = {0x1234u, 0xBEEFu};
/* HALT, HALT: resumed with IE = 0, so the CPU stays stopped. */
static const uint32_t halt_code[2] = {0xFC000000u, 0xFC000000u};

static cpt32_ctx_frame frames[TASKS];
static cpt32_ctx_frame idle_frame, done_frame;
static uint32_t idle_stack[256] __attribute__((aligned(8)));
static uint32_t current = TASK_A;
static uint32_t alive_mix = 2;
static uint32_t failures;

/* Kernel-side mirror of task_mix, updated on every SYS_YIELD. */
static uint32_t mix_x[2], mix_y[2], mix_z[2], mix_i[2];
static uint32_t spin_last[2], spin_reports[2], preemptions;

static void check(int ok) {
    if (!ok)
        ++failures;
}

static void init_user(uint32_t task, uint32_t entry, int with_seed) {
    cpt32_ctx_frame *frame = &frames[task];
    for (int i = 0; i < 32; ++i)
        frame->r[i] = 0;
    /* Stack ABI: argument word 0 is at SP. */
    uint32_t sp = USER_STACK_TOP(task) - 8u;
    *(volatile uint32_t *)sp = with_seed ? seeds[task] : 0u;
    frame->r[28] = sp;
    frame->epc = entry;
    frame->status = CPT32_STATUS_PU | CPT32_STATUS_PIE;
    if (with_seed) {
        mix_x[task] = seeds[task];
        mix_y[task] = seeds[task] ^ 0x5A5Au;
        mix_z[task] = 7u;
        mix_i[task] = 0;
    }
}

static void idle_loop(void) {
    for (;;)
        cpt32_halt();
}

static void timer_start(void) {
    REG32(TIMER0_PRESCALER) = 0;
    REG32(TIMER0_COMPARE_HI) = 0;
    REG32(TIMER0_COMPARE_LO) = TIMER_PERIOD;
    REG32(TIMER0_RELOAD_HI) = 0;
    REG32(TIMER0_RELOAD_LO) = TIMER_PERIOD;
    REG32(TIMER0_CTRL) = TIMER_PERIODIC_IRQ;
    REG32(TIMER_ENABLE) = 1u;
    REG16(IRQC_ENABLE) = 1u << IRQ_TIMER;
}

static cpt32_ctx_frame *finish(void) {
    REG32(TIMER0_CTRL) = 0;
    REG16(IRQC_ENABLE) = 0;
    for (int i = 0; i < 32; ++i)
        done_frame.r[i] = 0;
    done_frame.r[1] = failures == 0u ? 42u : 0x100u + failures;
    done_frame.r[28] = (uint32_t)(uintptr_t)&idle_stack[256];
    done_frame.epc = (uint32_t)(uintptr_t)halt_code;
    done_frame.status = 0; /* S mode, IE = 0 after IRET */
    return &done_frame;
}

static cpt32_ctx_frame *run(uint32_t task) {
    current = task;
    return &frames[task];
}

static cpt32_ctx_frame *handle_syscall(cpt32_ctx_frame *frame, uint32_t number) {
    uint32_t t = current;
    switch (number) {
    case SYS_YIELD: {
        check(t < 2u && mix_i[t] < 4u);
        uint32_t x = mix_x[t] * 3u + mix_i[t];
        uint32_t y = (mix_y[t] << 1) ^ x;
        uint32_t z = mix_z[t] + y;
        check(frame->r[1] == x && frame->r[2] == y && frame->r[3] == z);
        mix_x[t] = x, mix_y[t] = y, mix_z[t] = z, ++mix_i[t];
        frame->r[1] = 0;
        return run(alive_mix == 2u ? 1u - t : t);
    }
    case SYS_EXIT:
        check(t < 2u && mix_i[t] == 4u);
        check(frame->r[1] == mix_x[t] + mix_y[t] + mix_z[t]);
        if (--alive_mix != 0u)
            return run(1u - t);
        /* Phase 2: idle in S mode with interrupts on until the timer fires. */
        for (int i = 0; i < 32; ++i)
            idle_frame.r[i] = 0;
        idle_frame.r[28] = (uint32_t)(uintptr_t)&idle_stack[256];
        idle_frame.epc = (uint32_t)(uintptr_t)idle_loop;
        idle_frame.status = CPT32_STATUS_PIE;
        init_user(TASK_C, (uint32_t)(uintptr_t)task_spin, 0);
        init_user(TASK_D, (uint32_t)(uintptr_t)task_spin, 0);
        current = TASKS;
        timer_start();
        return &idle_frame;
    case SYS_REPORT: {
        check(t == TASK_C || t == TASK_D);
        uint32_t s = t - TASK_C;
        check(frame->r[1] == spin_last[s] + SPIN_REPORT_MASK + 1u);
        spin_last[s] = frame->r[1];
        ++spin_reports[s];
        frame->r[1] = 0;
        if (spin_reports[0] >= 3u && spin_reports[1] >= 3u && preemptions >= 4u)
            return finish();
        return frame;
    }
    default:
        ++failures;
        return finish();
    }
}

cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *frame) {
    uint32_t cause = frame->r[0];
    check(frame == cpt32_ctx_current && cpt32_trap_depth == 1u);
    if (!CPT32_CAUSE_IS_EXCEPTION(cause)) {
        if (cause != IRQ_TIMER) {
            ++failures;
            return finish();
        }
        REG32(TIMER_ACK) = 1u;
        REG16(IRQC_PENDING) = 1u << IRQ_TIMER;
        if (current == TASKS) {
            /* Trapped out of the S-mode idle context. */
            check(frame == &idle_frame && (frame->status & CPT32_STATUS_PU) == 0u);
            check(frame->r[28] > (uint32_t)(uintptr_t)idle_stack);
            check(frame->r[28] <= (uint32_t)(uintptr_t)&idle_stack[256]);
            return run(TASK_C);
        }
        check(frame->status & CPT32_STATUS_PU);
        ++preemptions;
        return run(current == TASK_C ? TASK_D : TASK_C);
    }
    if ((cause & 0xFFFF0000u) != 0x80030000u || (frame->status & CPT32_STATUS_PU) == 0u) {
        ++failures; /* a user fault ends the demo */
        return finish();
    }
    return handle_syscall(frame, CPT32_CAUSE_SYSCALL_NUMBER(cause));
}

void kernel_panic(uint32_t cause, uint32_t epc) {
    (void)epc;
    cpt32_jump((uint32_t)(uintptr_t)halt_code, 0x200u | (cause >> 16 & 0xFFu),
               (uint32_t)(uintptr_t)&idle_stack[256]);
}

int main(void) {
    uint32_t code_start = (uint32_t)(uintptr_t)user_code_begin;
    uint32_t code_end = (uint32_t)(uintptr_t)user_code_end;
    uint32_t mix = (uint32_t)(uintptr_t)task_mix, spin = (uint32_t)(uintptr_t)task_spin;
    if (!(code_start < mix && mix < code_end && code_start < spin && spin < code_end))
        return 2; /* user.c must keep user_code_begin first and user_code_end last */

    /* Region 0: user code (R/X). Region 1: the four user stacks (R/W). */
    cpt32_mtsr(CPT32_SR_RGN_START(0), code_start);
    cpt32_mtsr(CPT32_SR_RGN_END(0), code_end);
    cpt32_mtsr(CPT32_SR_RGN_FLAGS(0), CPT32_PERM_R | CPT32_PERM_X);
    cpt32_mtsr(CPT32_SR_RGN_START(1), USER_STACKS_BASE);
    cpt32_mtsr(CPT32_SR_RGN_END(1), USER_STACKS_END);
    cpt32_mtsr(CPT32_SR_RGN_FLAGS(1), CPT32_PERM_R | CPT32_PERM_W);

    init_user(TASK_A, mix, 1);
    init_user(TASK_B, mix, 1);
    cpt32_ctx_resume(run(TASK_A));
}
