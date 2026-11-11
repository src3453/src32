//! SRC32 HALT and Extension P intrinsics (`sdk/cpt32/include/cpt32/cpu.h`,
//! `doc/spec_CPU_priv.md`).
//!
//! The functions declared here are not linked: tools/llvm-src32 expands each
//! call in place. The SYSCALL number/context and the system register number are
//! instruction immediates, so the wrappers take them as const generics; they
//! are `#[inline(always)]` so the backend sees constants at every call.

unsafe extern "C" {
    fn cpt32_halt();
    fn cpt32_syscall(number: u32, context: u32, a1: u32, a2: u32, a3: u32, a4: u32) -> u32;
    fn cpt32_cpu_features() -> u32;
    fn cpt32_mfsr(sr: u32) -> u32;
    fn cpt32_mtsr(sr: u32, value: u32);
    fn cpt32_jump(entry: u32, r1: u32, sp: u32) -> !;
}

/// CPU_FEATURES bits (`doc/spec_CPU.md` §4.5).
pub const FEATURE_P: u32 = 0x100;
pub const FEATURE_V: u32 = 0x200;

/// System register numbers for [`mfsr`] / [`mtsr`].
pub mod sr {
    pub const STATUS: u32 = 0x00;
    pub const EPC: u32 = 0x01;
    pub const CAUSE: u32 = 0x02;
    pub const USP: u32 = 0x08;
    pub const UFP: u32 = 0x09;
    pub const UGP: u32 = 0x0A;
    pub const VCTL: u32 = 0x40;
    pub const PTBR: u32 = 0x41;
    pub const VFAULT: u32 = 0x42;
    pub const TLBFLUSH: u32 = 0x43;
    pub const TLBINV: u32 = 0x44;
}

/// HALT: stops the CPU until an enabled IRQ is accepted (supervisor mode only).
#[inline(always)]
pub fn halt() {
    unsafe { cpt32_halt() }
}

/// SYSCALL `N`, `CTX` (0..=255) with R1..R4 = a1..a4; returns the handler's R1.
#[inline(always)]
pub fn syscall<const N: u32, const CTX: u32>(a1: u32, a2: u32, a3: u32, a4: u32) -> u32 {
    const { assert!(N <= 255 && CTX <= 255) };
    unsafe { cpt32_syscall(N, CTX, a1, a2, a3, a4) }
}

/// CPU_FEATURES as CPUID reports it.
#[inline(always)]
pub fn cpu_features() -> u32 {
    unsafe { cpt32_cpu_features() }
}

/// MFSR (supervisor mode only).
#[inline(always)]
pub fn mfsr<const SR: u32>() -> u32 {
    unsafe { cpt32_mfsr(SR) }
}

/// MTSR (supervisor mode only).
///
/// # Safety
/// Changes the privilege, protection or address translation state.
#[inline(always)]
pub unsafe fn mtsr<const SR: u32>(value: u32) {
    unsafe { cpt32_mtsr(SR, value) }
}

/// R1 = `r1`, SP = `sp`, FP = 0, then jump to `entry` (a loader's hand-off).
///
/// # Safety
/// `entry` must be code that expects this state; the current stack is abandoned.
#[inline(always)]
pub unsafe fn jump(entry: u32, r1: u32, sp: u32) -> ! {
    unsafe { cpt32_jump(entry, r1, sp) }
}
