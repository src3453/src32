//! Extension P (doc/spec_CPU_priv.md) on every CPU kind.

use cpt32::bus::Bus;
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::cpu::{BUS_ERROR_VECTOR, ILLEGAL_INSTRUCTION_VECTOR, SYSCALL_VECTOR};
use cpt32::devices::ram::connect_ram;

const KINDS: &[CpuKind] = &[
    CpuKind::Scalar,
    CpuKind::Cached,
    CpuKind::CachedRelaxed,
    CpuKind::Pipelined,
    #[cfg(feature = "jit")]
    CpuKind::Jit,
];

const SR_STATUS: i16 = 0x00;
const SR_EPC: i16 = 0x01;
const SR_CAUSE: i16 = 0x02;
const SR_USP: i16 = 0x08;
const SR_RGN0_START: i16 = 0x10;
const SR_RGN0_END: i16 = 0x11;
const SR_RGN0_FLAGS: i16 = 0x12;

const USER_BASE: i16 = 0x1000;
const USER_END: i16 = 0x2000;
const KERNEL_SP: i16 = 0x0F00;
const USER_SP: i16 = 0x1800;

fn r(op: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
    (op << 26) | (rd << 21) | (rs1 << 16) | (rs2 << 11)
}

fn i(op: u32, rd: u32, rs1: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (rs1 << 16) | u32::from(imm as u16)
}

fn addi(rd: u32, rs1: u32, imm: i16) -> u32 {
    i(0x04, rd, rs1, imm)
}

fn mfsr(rd: u32, sr: i16) -> u32 {
    i(0x24, rd, 0, sr)
}

fn mtsr(sr: i16, rs: u32) -> u32 {
    i(0x25, 0, rs, sr)
}

fn syscall(number: u8, context: u8) -> u32 {
    i(0x23, 0, 0, i16::from_be_bytes([context, number]))
}

const HALT: u32 = 0x3F << 26;
const IRET: u32 = 0x20 << 26;

/// Kernel at 0: grant R/W/X on USER_BASE..USER_END, set both stacks and S
/// R30, then IRET into U at `user_pc` with the given STATUS.
fn kernel(user_pc: i16, status: i16) -> Vec<u32> {
    vec![
        addi(1, 0, USER_BASE),
        mtsr(SR_RGN0_START, 1),
        addi(1, 0, USER_END),
        mtsr(SR_RGN0_END, 1),
        addi(1, 0, 7),
        mtsr(SR_RGN0_FLAGS, 1),
        addi(1, 0, USER_SP),
        mtsr(SR_USP, 1),
        addi(28, 0, KERNEL_SP),
        addi(30, 0, 0x600),
        addi(1, 0, user_pc),
        mtsr(SR_EPC, 1),
        addi(1, 0, status),
        mtsr(SR_STATUS, 1),
        IRET,
    ]
}

/// Records CAUSE in R11 and EPC in R12, then halts.
fn report_and_halt() -> Vec<u32> {
    vec![mfsr(11, SR_CAUSE), mfsr(12, SR_EPC), HALT]
}

fn boot(kind: CpuKind, pieces: &[(u32, Vec<u32>)]) -> AnyCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    for (base, words) in pieces {
        let image: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        cpu.load_program(*base, &image);
    }
    cpu.write_mem_u32_be(SYSCALL_VECTOR, 0x200);
    cpu.write_mem_u32_be(BUS_ERROR_VECTOR, 0x300);
    cpu.write_mem_u32_be(ILLEGAL_INSTRUCTION_VECTOR, 0x400);
    cpu.set_pc(0);
    cpu
}

#[test]
fn user_syscall_switches_bank_and_iret_resumes_after_it() {
    for &kind in KINDS {
        let user = vec![
            addi(28, 28, -4),
            addi(5, 0, 11),
            syscall(42, 3),
            addi(6, 0, 1),
            i(0x02, 5, 0, 0x3000), // ST [R0 + 0x3000], R5: outside the region
            HALT,
        ];
        let handler = vec![
            mfsr(7, SR_CAUSE),
            mfsr(8, SR_STATUS),
            mfsr(9, SR_USP),
            r(0x03, 10, 28, 0), // ADD R10, R28, R0 (S stack)
            IRET,
        ];
        let mut cpu = boot(
            kind,
            &[
                (0, kernel(USER_BASE, 0x08)),
                (0x200, handler),
                (0x300, report_and_halt()),
                (USER_BASE as u32, user),
            ],
        );
        cpu.run(4000);
        assert!(!cpu.is_running(), "{kind:?} pc={:08X}", cpu.pc());
        assert_eq!(cpu.read_reg(7), 0x8003_032A, "{kind:?} CAUSE");
        assert_eq!(cpu.read_reg(8), 0x08, "{kind:?} STATUS in handler");
        assert_eq!(cpu.read_reg(9), USER_SP as u32 - 4, "{kind:?} USP");
        assert_eq!(cpu.read_reg(10), KERNEL_SP as u32, "{kind:?} S stack");
        assert_eq!(cpu.read_reg(6), 1, "{kind:?} resumed after SYSCALL");
        assert_eq!(cpu.read_reg(11), 0x8005_0002, "{kind:?} write fault");
        assert_eq!(
            cpu.read_reg(12),
            USER_BASE as u32 + 16,
            "{kind:?} fault EPC"
        );
        assert_eq!(cpu.read_mem_u32_be(0x3000), 0, "{kind:?} denied store");
        assert_eq!(cpu.status() & 0x0C, 0x08, "{kind:?} S mode, PU=1");
    }
}

#[test]
fn user_privileged_instruction_and_denied_fetch_trap() {
    for &kind in KINDS {
        let mut cpu = boot(
            kind,
            &[
                (0, kernel(USER_BASE, 0x08)),
                (0x400, report_and_halt()),
                (USER_BASE as u32, vec![addi(1, 0, 1), HALT]),
            ],
        );
        cpu.run(4000);
        assert_eq!(cpu.read_reg(11), 0x8004_0000, "{kind:?} privileged");
        assert_eq!(cpu.read_reg(12), USER_BASE as u32 + 4, "{kind:?}");

        let mut cpu = boot(
            kind,
            &[(0, kernel(0x5000, 0x08)), (0x300, report_and_halt())],
        );
        cpu.run(4000);
        assert_eq!(cpu.read_reg(11), 0x8005_0004, "{kind:?} fetch fault");
        assert_eq!(cpu.read_reg(12), 0x5000, "{kind:?}");
    }
}

#[test]
fn illegal_instruction_traps_in_supervisor_mode() {
    for &kind in KINDS {
        let mut cpu = boot(
            kind,
            &[
                (0, vec![addi(1, 0, 5), r(0x26, 0, 0, 0)]),
                (0x400, report_and_halt()),
            ],
        );
        cpu.run(400);
        assert!(!cpu.is_running(), "{kind:?}");
        assert_eq!(cpu.read_reg(11), 0x8001_0000, "{kind:?}");
        assert_eq!(cpu.read_reg(12), 4, "{kind:?}");
        assert_eq!(cpu.status() & 0x0C, 0, "{kind:?} trap from S keeps PU=0");
    }
}

#[test]
fn supervisor_syscall_and_unknown_sr_trap() {
    for &kind in KINDS {
        let program = vec![syscall(7, 0), addi(2, 0, 9), mfsr(3, 0x03)];
        let handler = vec![mfsr(7, SR_CAUSE), IRET];
        let mut cpu = boot(
            kind,
            &[(0, program), (0x200, handler), (0x400, report_and_halt())],
        );
        cpu.run(400);
        assert_eq!(cpu.read_reg(7), 0x8003_0007, "{kind:?}");
        assert_eq!(cpu.read_reg(2), 9, "{kind:?}");
        assert_eq!(cpu.read_reg(11), 0x8001_0000, "{kind:?} unknown SR");
        assert_eq!(cpu.read_reg(12), 8, "{kind:?}");
        assert!(!cpu.irq_enabled(), "{kind:?}");
    }
}

#[test]
fn irq_from_user_mode_uses_supervisor_r30() {
    for &kind in KINDS {
        // User code overwrites its own R28/R30 and spins; the IRQ slot jumps
        // through R30, which must be the kernel's 0x600.
        let user = vec![addi(30, 0, 0x1100), addi(28, 0, 0x1200), i(0x09, 0, 0, -4)];
        let handler = vec![
            mfsr(9, SR_USP),
            r(0x03, 10, 28, 0),
            mfsr(11, SR_CAUSE),
            mfsr(12, SR_STATUS),
            HALT,
        ];
        let mut cpu = boot(
            kind,
            &[
                (0, kernel(USER_BASE, 0x0B)),
                (0x600, handler),
                (USER_BASE as u32, user),
            ],
        );
        cpu.write_mem_u32_be(0xFFFF_010C, r(0x0B, 30, 0, 0)); // IRQ3 slot: JR R30
        cpu.run(400);
        assert!(cpu.is_running(), "{kind:?}");
        cpu.set_irq_input(true, 3);
        cpu.run(400);
        assert!(!cpu.is_running(), "{kind:?} pc={:08X}", cpu.pc());
        assert_eq!(cpu.read_reg(9), 0x1200, "{kind:?} user R28 preserved");
        assert_eq!(cpu.read_reg(10), KERNEL_SP as u32, "{kind:?}");
        assert_eq!(cpu.read_reg(11), 3, "{kind:?}");
        assert_eq!(cpu.read_reg(12), 0x0A, "{kind:?} PIE=1, PU=1, IE=0");
    }
}
