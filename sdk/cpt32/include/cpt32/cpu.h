#ifndef CPT32_CPU_H
#define CPT32_CPU_H
#include <stdint.h>

#ifdef __cplusplus
#define CPT32_NORETURN [[noreturn]]
#else
#define CPT32_NORETURN _Noreturn
#endif

/* Compiler intrinsics for tools/llvm-src32. They are lowered inline to SRC32
 * instructions; do not define functions with these names. Extension P is
 * described in doc/spec_CPU_priv.md. */

/* Emit the SRC32 HALT instruction. The CPU resumes after the call when an
 * enabled IRQ is accepted; an IRET returns to the following instruction.
 * Requires tools/llvm-src32; this is not a host-side CPU or sleep function.
 * Supervisor mode only. */
/* HALT命令でCPUを停止し、有効なIRQの受理後に再開します（Sモードのみ）。 */
void cpt32_halt(void);

/* Extension P system register numbers for cpt32_mfsr/cpt32_mtsr. */
#define CPT32_SR_STATUS 0x00u
#define CPT32_SR_EPC 0x01u
#define CPT32_SR_CAUSE 0x02u
#define CPT32_SR_USP 0x08u
#define CPT32_SR_UFP 0x09u
#define CPT32_SR_UGP 0x0Au
#define CPT32_SR_RGN_START(n) (0x10u + 4u * (n))
#define CPT32_SR_RGN_END(n) (0x11u + 4u * (n))
#define CPT32_SR_RGN_FLAGS(n) (0x12u + 4u * (n))
/* Extension V (doc/spec_CPU_vm.md); present when CPT32_FEATURE_V is set. */
#define CPT32_SR_VCTL 0x40u
#define CPT32_SR_PTBR 0x41u
#define CPT32_SR_VFAULT 0x42u
#define CPT32_SR_TLBFLUSH 0x43u
#define CPT32_SR_TLBINV 0x44u
#define CPT32_VCTL_EN 1u
/* CPU_FEATURES bits (doc/spec_CPU.md §4.5). */
#define CPT32_FEATURE_P 0x100u
#define CPT32_FEATURE_V 0x200u

/* STATUS bits and region permission bits. */
#define CPT32_STATUS_IE 0x01u
#define CPT32_STATUS_PIE 0x02u
#define CPT32_STATUS_U 0x04u
#define CPT32_STATUS_PU 0x08u
#define CPT32_STATUS_PSM 0x10u
#define CPT32_PERM_R 1u
#define CPT32_PERM_W 2u
#define CPT32_PERM_X 4u

/* CAUSE helpers: SYSCALL number and context byte, exception test. */
#define CPT32_CAUSE_IS_EXCEPTION(cause) (((cause) & 0x80000000u) != 0u)
#define CPT32_CAUSE_SYSCALL_NUMBER(cause) ((cause) & 0xFFu)
#define CPT32_CAUSE_SYSCALL_CONTEXT(cause) (((cause) >> 8) & 0xFFu)

/* SYSCALL number, context: integer constants 0..255 (instruction
 * immediates). a1..a4 are passed in R1..R4; the handler's R1 is returned.
 * Usable from user and supervisor mode. */
/* SYSCALL命令を発行します。番号・コンテキストは0〜255の定数、引数はR1〜R4、戻り値はR1。 */
uint32_t cpt32_syscall(uint32_t number, uint32_t context, uint32_t a1, uint32_t a2, uint32_t a3, uint32_t a4);
#define CPT32_SYSCALL0(number) cpt32_syscall((number), 0u, 0u, 0u, 0u, 0u)
#define CPT32_SYSCALL1(number, a1) cpt32_syscall((number), 0u, (a1), 0u, 0u, 0u)
#define CPT32_SYSCALL2(number, a1, a2) cpt32_syscall((number), 0u, (a1), (a2), 0u, 0u)
#define CPT32_SYSCALL3(number, a1, a2, a3) cpt32_syscall((number), 0u, (a1), (a2), (a3), 0u)
#define CPT32_SYSCALL4(number, a1, a2, a3, a4) cpt32_syscall((number), 0u, (a1), (a2), (a3), (a4))

/* CPU_FEATURES as the CPUID instruction reports it (doc/spec_CPU.md). */
/* CPUID命令が返すCPU_FEATURESを読みます。 */
uint32_t cpt32_cpu_features(void);

/* Read/write a system register; sr must be one of CPT32_SR_* as a constant.
 * Supervisor mode only. */
/* システムレジスタを読み書きします（srは定数、Sモードのみ）。 */
uint32_t cpt32_mfsr(uint32_t sr);
void cpt32_mtsr(uint32_t sr, uint32_t value);

/* Enter user mode at entry with R28 = user_stack_top, keeping the current
 * interrupt enable. Returns the code later passed to cpt32_return_to_kernel.
 * Program the protection regions first. entry must not return; it ends with a
 * SYSCALL whose handler calls cpt32_return_to_kernel. One user program at a
 * time (not reentrant). Supervisor mode only. */
/* entryからUモードで実行し、cpt32_return_to_kernelに渡された値を返します（Sモードのみ）。 */
uint32_t cpt32_run_user(void (*entry)(void), uint32_t user_stack_top);

/* From the SYSCALL handler: abandon the user program and make the pending
 * cpt32_run_user call return code. Supervisor mode only. */
/* SYSCALLハンドラから、ユーザープログラムを終了してcpt32_run_userへ戻ります。 */
CPT32_NORETURN void cpt32_return_to_kernel(uint32_t code);

/* Far jump that never returns: R1 = r1, R28 (SP) = sp, R29 (FP) = 0, then
 * PC = entry. Interrupt state and mode are unchanged. Used by loaders such as
 * the BIOS to hand a register argument and a fresh stack to the next stage. */
/* R1とSPを設定してentryへ分岐し、戻りません（ブートローダーの引き渡し用）。 */
CPT32_NORETURN void cpt32_jump(uint32_t entry, uint32_t r1, uint32_t sp);

/* SYSCALL trap frame: r[0] = CAUSE, r[n] = Rn at the SYSCALL (r[28] unused,
 * reads 0). Writing r[1] sets the value cpt32_syscall returns. */
typedef struct cpt32_trap_frame {
    uint32_t r[32];
} cpt32_trap_frame;

/* Define this function to handle SYSCALL. The compiler then installs the
 * SYSCALL vector before main(); the entry saves all registers in the frame,
 * calls it on the supervisor stack and returns with IRET. */
/* 定義するとSYSCALLベクタが登録され、SYSCALL時にフレームを渡して呼ばれます。 */
void syscall_dispatch(cpt32_trap_frame *frame);

/* Context-switching trap stub (doc/spec_Foundation.md §3). r[0] = CAUSE,
 * r[n] = Rn; r[28..30] = the interrupted mode's R28..R30 (USP/UFP/UGP after
 * a user trap). status: only PIE, PU and PSM are restored. */
typedef struct cpt32_ctx_frame {
    uint32_t r[32];
    uint32_t epc;
    uint32_t status;
} cpt32_ctx_frame;

/* Define kernel_trap (together with kernel_panic) to own every IRQ and
 * exception vector. Each trap saves the full context into *cpt32_ctx_current,
 * calls kernel_trap on a fresh kernel stack (--stack-top) with interrupts
 * off, and resumes the frame it returns. Program the protection regions
 * before returning another process's frame. Cannot be combined with
 * irqc_dispatch or syscall_dispatch. S-mode code must leave R30 unchanged. */
/* 定義すると全IRQ・例外ベクタを登録し、保存したフレームを渡して呼ばれます。戻り値のフレームへ復帰します。 */
cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *frame);

/* Called instead of kernel_trap for a trap taken while the kernel runs (or
 * before the first cpt32_ctx_resume). Must not return. */
/* カーネル実行中のトラップで呼ばれます。戻ってはいけません。 */
CPT32_NORETURN void kernel_panic(uint32_t cause, uint32_t epc);

/* Provided by the stub: the frame the next trap saves into, and 1 while the
 * kernel runs (0 while a resumed context runs). */
extern cpt32_ctx_frame *cpt32_ctx_current;
extern uint32_t cpt32_trap_depth;

/* Leave the kernel and resume frame through the stub's restore path; the
 * calling stack is abandoned. Use it once to start the first context.
 * Requires kernel_trap. Supervisor mode only. */
/* frameへ復帰し、戻りません（最初のコンテキストの開始用、Sモードのみ）。 */
CPT32_NORETURN void cpt32_ctx_resume(cpt32_ctx_frame *frame);

#endif
