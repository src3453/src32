//! Extension V (doc/spec_CPU_vm.md) on every CPU kind.

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
const SR_VCTL: i16 = 0x40;
const SR_PTBR: i16 = 0x41;
const SR_VFAULT: i16 = 0x42;
const SR_TLBFLUSH: i16 = 0x43;
const SR_TLBINV: i16 = 0x44;

const V: u32 = 1;
const R: u32 = 2;
const W: u32 = 4;
const X: u32 = 8;
const U: u32 = 16;

const L1: u32 = 0x6000;
const L2_LOW: u32 = 0x7000;
const L2_HIGH: u32 = 0x5000;
const USER_CODE_PA: u32 = 0x1000;
const USER_CODE_VA: i16 = 0x2000;
const FAULT_HANDLER: u32 = 0x300;
const SYSCALL_HANDLER: u32 = 0x200;

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

fn ld(rd: u32, base: u32, offset: i16) -> u32 {
    i(0x01, rd, base, offset)
}

fn st(src: u32, base: u32, offset: i16) -> u32 {
    i(0x02, src, base, offset)
}

fn ldb(rd: u32, base: u32, offset: i16) -> u32 {
    i(0x13, rd, base, offset)
}

fn syscall(number: u8) -> u32 {
    i(0x23, 0, 0, i16::from(number))
}

fn jmp_self() -> u32 {
    i(0x09, 0, 0, -4)
}

const HALT: u32 = 0x3F << 26;
const IRET: u32 = 0x20 << 26;

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

/// S-mode boot code: install the tables, enable V and IRET into U.
fn kernel() -> Vec<u32> {
    vec![
        addi(1, 0, L1 as i16),
        mtsr(SR_PTBR, 1),
        addi(1, 0, 1),
        mtsr(SR_VCTL, 1),
        addi(1, 0, 0x1800),
        mtsr(SR_USP, 1),
        addi(28, 0, 0x0F00),
        addi(1, 0, USER_CODE_VA),
        mtsr(SR_EPC, 1),
        addi(1, 0, 0x08),
        mtsr(SR_STATUS, 1),
        IRET,
    ]
}

/// CAUSE in R11, EPC in R12, VFAULT in R13.
fn report_and_halt() -> Vec<u32> {
    vec![
        mfsr(11, SR_CAUSE),
        mfsr(12, SR_EPC),
        mfsr(13, SR_VFAULT),
        HALT,
    ]
}

fn pte(pa: u32, flags: u32) -> u32 {
    pa | flags
}

/// VA 0x2000 code (RX) -> 0x1000, 0x3000 RW -> 0x2000, 0x4000 RW -> 0x4000,
/// 0x5000 R -> 0x2000, 0x7000 RW without U -> 0x3000, 0x8000 RW without X ->
/// 0x3000. 0x6000 stays unmapped. VA 0xFFFFF000 -> 0x3000 RW U.
fn standard_map() -> Vec<(u32, u32, u32)> {
    vec![
        (L2_LOW, 2, pte(USER_CODE_PA, V | R | X | U)),
        (L2_LOW, 3, pte(0x2000, V | R | W | U)),
        (L2_LOW, 4, pte(0x4000, V | R | W | U)),
        (L2_LOW, 5, pte(0x2000, V | R | U)),
        (L2_LOW, 7, pte(0x3000, V | R | W)),
        (L2_LOW, 8, pte(0x3000, V | R | W | U)),
        (L2_HIGH, 1023, pte(0x3000, V | R | W | U)),
    ]
}

fn boot(kind: CpuKind, user: &[u32], handler: &[u32], map: &[(u32, u32, u32)]) -> AnyCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.load_program(0, &words(&kernel()));
    cpu.load_program(SYSCALL_HANDLER, &words(handler));
    cpu.load_program(FAULT_HANDLER, &words(&report_and_halt()));
    cpu.load_program(USER_CODE_PA, &words(user));
    cpu.write_mem_u32_be(SYSCALL_VECTOR, SYSCALL_HANDLER);
    cpu.write_mem_u32_be(BUS_ERROR_VECTOR, FAULT_HANDLER);
    cpu.write_mem_u32_be(ILLEGAL_INSTRUCTION_VECTOR, 0x400);
    cpu.write_mem_u32_be(L1, pte(L2_LOW, V));
    cpu.write_mem_u32_be(L1 + 4 * 1023, pte(L2_HIGH, V));
    for &(table, index, value) in map {
        cpu.write_mem_u32_be(table + 4 * index, value);
    }
    cpu.set_pc(0);
    cpu
}

fn run_fault(kind: CpuKind, user: &[u32], map: &[(u32, u32, u32)]) -> AnyCpu {
    let mut cpu = boot(kind, user, &[IRET], map);
    cpu.run(4000);
    assert!(!cpu.is_running(), "{kind:?} pc={:08X}", cpu.pc());
    cpu
}

#[test]
fn cpuid_reports_extension_v() {
    for &kind in KINDS {
        let mut cpu = boot(kind, &[], &[IRET], &[]);
        cpu.load_program(0, &words(&[r(0x3E, 0, 0, 0), HALT]));
        cpu.run(64);
        assert_ne!(cpu.read_reg(2) & 0x200, 0, "{kind:?}");
    }
}

#[test]
fn user_accesses_are_translated() {
    for &kind in KINDS {
        let user = [
            addi(2, 0, 0x3000),
            addi(3, 0, 0x55),
            st(3, 2, 8), // VA 0x3008 -> PA 0x2008
            ld(4, 2, 8),
            addi(5, 0, 0x5000),
            ldb(6, 5, 11), // read-only alias of the same frame
            syscall(1),
        ];
        let handler = [mfsr(7, SR_CAUSE), HALT];
        let mut cpu = boot(kind, &user, &handler, &standard_map());
        cpu.run(4000);
        assert_eq!(cpu.read_mem_u32_be(0x2008), 0x55, "{kind:?} frame");
        assert_eq!(cpu.read_mem_u32_be(0x3008), 0, "{kind:?} not identity");
        assert_eq!(cpu.read_reg(4), 0x55, "{kind:?} load");
        assert_eq!(cpu.read_reg(6), 0x55, "{kind:?} alias byte");
        assert_eq!(cpu.read_reg(7), 0x8003_0001, "{kind:?} ran to SYSCALL");
    }
}

#[test]
fn missing_and_denied_pages_raise_precise_page_faults() {
    let cases: [(&str, Vec<u32>, u32, u32, u32); 5] = [
        // (name, user code, CAUSE, EPC, VFAULT)
        (
            "unmapped read",
            vec![addi(2, 0, 0x6000), ld(4, 2, 4)],
            0x8006_0001,
            0x2004,
            0x6004,
        ),
        (
            "write to read-only",
            vec![addi(2, 0, 0x5000), st(2, 2, 0)],
            0x8006_000A,
            0x2004,
            0x5000,
        ),
        (
            "supervisor-only page",
            vec![addi(2, 0, 0x7000), ld(4, 2, 0)],
            0x8006_0009,
            0x2004,
            0x7000,
        ),
        (
            "fetch from non-executable",
            vec![i(0x3C, 2, 0, 0x8000u16 as i16), r(0x0B, 2, 0, 0)],
            0x8006_000C,
            0x8000,
            0x8000,
        ),
        (
            "reserved top page",
            vec![addi(2, 0, -4096), ld(4, 2, 0)],
            0x8006_0009,
            0x2004,
            0xFFFF_F000,
        ),
    ];
    for &kind in KINDS {
        for (name, user, cause, epc, vfault) in &cases {
            let cpu = run_fault(kind, user, &standard_map());
            assert_eq!(cpu.read_reg(11), *cause, "{kind:?} {name} CAUSE");
            assert_eq!(cpu.read_reg(12), *epc, "{kind:?} {name} EPC");
            assert_eq!(cpu.read_reg(13), *vfault, "{kind:?} {name} VFAULT");
        }
    }
}

#[test]
fn page_crossing_access_uses_both_frames_or_faults_whole() {
    for &kind in KINDS {
        // VA 0x3FFE..0x4001 spans PA 0x2FFE..0x2FFF and 0x4000..0x4001.
        let user = [
            addi(2, 0, 0x3FFE),
            addi(3, 0, 0x1234),
            st(3, 2, 0),
            ld(4, 2, 0),
            syscall(1),
        ];
        let mut cpu = boot(kind, &user, &[HALT], &standard_map());
        cpu.run(4000);
        assert_eq!(cpu.read_reg(4), 0x1234, "{kind:?} crossing load");
        assert_eq!(
            cpu.read_mem_u32_be(0x2FFC) & 0xFFFF,
            0x1234 >> 16,
            "{kind:?}"
        );
        assert_eq!(
            cpu.read_mem_u32_be(0x4000) >> 16,
            0x1234 & 0xFFFF,
            "{kind:?}"
        );

        // Second page unmapped: neither byte of the first page is written.
        let user = [addi(2, 0, 0x5FFE), addi(3, 0, -1), st(3, 2, 0)];
        let mut map = standard_map();
        map.push((L2_LOW, 5, pte(0x2000, V | R | W | U)));
        let mut cpu = run_fault(kind, &user, &map);
        assert_eq!(cpu.read_reg(11), 0x8006_0002, "{kind:?} CAUSE");
        assert_eq!(cpu.read_reg(13), 0x6000, "{kind:?} first faulting page");
        assert_eq!(cpu.read_mem_u32_be(0x2FFC), 0, "{kind:?} nothing written");
    }
}

#[test]
fn tlb_caches_until_invalidated() {
    for invalidate in [None, Some(SR_TLBINV), Some(SR_TLBFLUSH)] {
        for &kind in KINDS {
            // The first load primes the TLB. On the first SYSCALL the handler
            // repoints the PTE of VA 0x3000 at the frame 0x3000 and optionally
            // invalidates; the second SYSCALL halts.
            let user = [
                addi(2, 0, 0x3000),
                ld(4, 2, 0),
                syscall(1),
                ld(5, 2, 0),
                syscall(2),
            ];
            let mut handler = vec![
                ld(8, 0, 0x7F00), // call counter in S memory
                addi(9, 8, 1),
                st(9, 0, 0x7F00),
                i(0x07, 0, 8, 4), // BEQ R8, R0, +4: first call patches
                HALT,
                addi(9, 0, 0x3000 | (V | R | W | U) as i16),
                st(9, 0, (L2_LOW + 12) as i16),
                addi(8, 0, 0x3000),
            ];
            if let Some(sr) = invalidate {
                handler.push(mtsr(sr, 8));
            }
            handler.push(IRET);
            let mut cpu = boot(kind, &user, &handler, &standard_map());
            cpu.write_mem_u32_be(0x2000, 0xAAAA_0001);
            cpu.write_mem_u32_be(0x3000, 0xBBBB_0002);
            cpu.run(4000);
            assert!(!cpu.is_running(), "{kind:?} {invalidate:?}");
            assert_eq!(cpu.read_reg(4), 0xAAAA_0001, "{kind:?} first load");
            let expected = if invalidate.is_some() {
                0xBBBB_0002
            } else {
                0xAAAA_0001
            };
            assert_eq!(
                cpu.read_reg(5),
                expected,
                "{kind:?} {invalidate:?} second load"
            );
        }
    }
}

#[test]
fn disabled_v_leaves_extension_p_regions_in_charge() {
    for &kind in KINDS {
        // VCTL stays 0: the region allows 0x1000..0x2000, so a fetch there
        // works untranslated and an access outside faults as in Extension P.
        let kernel = [
            addi(1, 0, 0x1000),
            mtsr(0x10, 1),
            addi(1, 0, 0x2000),
            mtsr(0x11, 1),
            addi(1, 0, 7),
            mtsr(0x12, 1),
            addi(1, 0, 0x1800),
            mtsr(SR_USP, 1),
            addi(1, 0, 0x1000),
            mtsr(SR_EPC, 1),
            addi(1, 0, 0x08),
            mtsr(SR_STATUS, 1),
            IRET,
        ];
        let user = [addi(2, 0, 0x3000), ld(4, 2, 0)];
        let mut cpu = boot(kind, &user, &[IRET], &[]);
        cpu.load_program(0, &words(&kernel));
        cpu.run(4000);
        assert_eq!(cpu.read_reg(11), 0x8005_0001, "{kind:?} region fault");
    }
}
