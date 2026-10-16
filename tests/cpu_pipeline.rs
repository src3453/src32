use cpt32::bus::Bus;
use cpt32::cpu_pipeline::PipelineCpu;
use cpt32::devices::ram::connect_ram;

fn encode_r(op: u8, rd: u8, rs1: u8, rs2: u8) -> [u8; 4] {
    let raw =
        ((op as u32) << 26) | ((rd as u32) << 21) | ((rs1 as u32) << 16) | ((rs2 as u32) << 11);
    raw.to_be_bytes()
}

fn encode_i(op: u8, rd: u8, rs1: u8, imm: i16) -> [u8; 4] {
    let raw =
        ((op as u32) << 26) | ((rd as u32) << 21) | ((rs1 as u32) << 16) | (imm as u16 as u32);
    raw.to_be_bytes()
}

fn boot() -> PipelineCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    PipelineCpu::new(bus)
}

#[test]
fn alu_chain_forwards_without_stall() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 10)); // ADDI R1, R0, 10
    image.extend_from_slice(&encode_i(0x04, 2, 0, 20)); // ADDI R2, R0, 20
    image.extend_from_slice(&encode_r(0x03, 3, 1, 2)); // ADD R3, R1, R2 (=30)
    image.extend_from_slice(&encode_r(0x03, 4, 3, 1)); // ADD R4, R3, R1 (=40)
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(64);
    assert_eq!(cpu.read_reg(3), 30);
    assert_eq!(cpu.read_reg(4), 40);
}

#[test]
fn load_use_stalls_one_cycle_and_forwards() {
    let mut cpu = boot();
    cpu.write_mem_u32_be(0x100, 0x1234_5678);
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 0x100)); // ADDI R1, R0, 0x100
    image.extend_from_slice(&encode_i(0x01, 3, 1, 0)); // LD R3, [R1]
    image.extend_from_slice(&encode_r(0x03, 4, 3, 3)); // ADD R4, R3, R3 (load-use)
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(64);
    assert_eq!(cpu.read_reg(3), 0x1234_5678);
    assert_eq!(cpu.read_reg(4), 0x1234_5678u32.wrapping_mul(2));
    assert_eq!(cpu.stall_cycles(), 1);
}

#[test]
fn branch_taken_flushes_and_skips() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 1)); // ADDI R1, R0, 1
    image.extend_from_slice(&encode_i(0x04, 2, 0, 1)); // ADDI R2, R0, 1
    image.extend_from_slice(&encode_i(0x07, 2, 1, 4)); // BEQ R1, R2, +4 (taken)
    image.extend_from_slice(&encode_i(0x04, 3, 0, 0xAAu16 as i16)); // skipped
    image.extend_from_slice(&encode_i(0x04, 3, 0, 0x55)); // target
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(64);
    assert_eq!(cpu.flush_events(), 1);
    assert_eq!(cpu.instructions_executed(), 5);
}

#[test]
fn not_taken_branch_does_not_flush_or_skip_fallthrough() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 1)); // R1 = 1
    image.extend_from_slice(&encode_i(0x04, 2, 0, 2)); // R2 = 2
    image.extend_from_slice(&encode_i(0x07, 1, 2, 4)); // BEQ not taken
    image.extend_from_slice(&encode_i(0x04, 3, 0, 0x55)); // fall-through
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(64);
    assert_eq!(cpu.read_reg(3), 0x55);
    assert_eq!(cpu.flush_events(), 0);
    assert_eq!(cpu.instructions_executed(), 5);
}

#[test]
fn mul_div_match_scalar_with_extra_latency() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 21)); // R1 = 21
    image.extend_from_slice(&encode_i(0x04, 2, 0, 3)); // R2 = 3
    image.extend_from_slice(&encode_r(0x18, 3, 1, 2)); // MUL R3 = 63
    image.extend_from_slice(&encode_r(0x03, 4, 3, 1)); // ADD R4, R3, R1 = 84
    image.extend_from_slice(&encode_r(0x19, 5, 1, 2)); // DIV R5 = 7
    image.extend_from_slice(&encode_r(0x03, 6, 5, 2)); // ADD R6, R5, R2 = 10
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(256);
    assert_eq!(cpu.read_reg(3), 63);
    assert_eq!(cpu.read_reg(4), 84);
    assert_eq!(cpu.read_reg(5), 7);
    assert_eq!(cpu.read_reg(6), 10);
    assert_eq!(cpu.stall_cycles(), (MUL_EXTRA + DIV_EXTRA) as u128);
}

#[test]
fn every_multicycle_alu_op_stalls_exactly_and_forwards_to_its_consumer() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 21)); // R1 = 21
    image.extend_from_slice(&encode_i(0x04, 2, 0, 3)); // R2 = 3
    image.extend_from_slice(&encode_r(0x18, 3, 1, 2)); // MUL R3 = 63
    image.extend_from_slice(&encode_r(0x03, 4, 3, 1)); // dependent ADD = 84
    image.extend_from_slice(&encode_r(0x1B, 5, 1, 2)); // MULH R5 = 0
    image.extend_from_slice(&encode_r(0x03, 6, 5, 1)); // dependent ADD = 21
    image.extend_from_slice(&encode_r(0x19, 7, 1, 2)); // DIV R7 = 7
    image.extend_from_slice(&encode_r(0x03, 8, 7, 2)); // dependent ADD = 10
    image.extend_from_slice(&encode_r(0x1A, 9, 1, 2)); // MOD R9 = 0
    image.extend_from_slice(&encode_r(0x03, 10, 9, 2)); // dependent ADD = 3
    image.extend_from_slice(&encode_r(0x1C, 11, 1, 2)); // DIVU R11 = 7
    image.extend_from_slice(&encode_r(0x03, 12, 11, 2)); // dependent ADD = 10
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(512);

    assert_eq!(cpu.read_reg(4), 84);
    assert_eq!(cpu.read_reg(6), 21);
    assert_eq!(cpu.read_reg(8), 10);
    assert_eq!(cpu.read_reg(10), 3);
    assert_eq!(cpu.read_reg(12), 10);
    assert_eq!(cpu.stall_cycles(), (2 * MUL_EXTRA + 3 * DIV_EXTRA) as u128);
    assert_eq!(cpu.instructions_executed(), 13);
}

const MUL_EXTRA: u32 = 2; // MUL_LATENCY(3) - 1 execute tick
const DIV_EXTRA: u32 = 15; // DIV_LATENCY(16) - 1 execute tick

#[test]
fn jal_links_and_jr_returns() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x0A, 0, 0, 12)); // JAL +12 -> 0x10
    image.extend_from_slice(&encode_i(0x04, 3, 0, 0xAAu16 as i16)); // skipped
    image.extend_from_slice(&encode_i(0x04, 3, 0, 0xBBu16 as i16)); // skipped
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // skipped (0xC)
    image.extend_from_slice(&encode_i(0x04, 4, 0, 0x55)); // target 0x10: R4 = 0x55
    image.extend_from_slice(&encode_r(0x0B, 31, 0, 0)); // JR R31 -> 0x4
    cpu.load_program(0, &image);
    cpu.run(64);
    // JAL at 0x0: LR = 0x4, PC = 0x10. R4 = 0x55. JR R31 jumps back to 0x4:
    // R3 = 0xAA then 0xBB, then HALT at 0xC.
    assert_eq!(cpu.read_reg(31), 0x4);
    assert_eq!(cpu.read_reg(4), 0x55);
    assert_eq!(cpu.read_reg(3), 0xBB);
}

#[test]
fn store_then_load_roundtrip() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 0x200)); // R1 = 0x200
    image.extend_from_slice(&encode_i(0x04, 2, 0, 42)); // R2 = 42
    image.extend_from_slice(&encode_i(0x02, 2, 1, 0)); // ST [R1], R2
    image.extend_from_slice(&encode_i(0x01, 3, 1, 0)); // LD R3, [R1]
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(64);
    assert_eq!(cpu.read_reg(3), 42);
    assert_eq!(cpu.read_mem_u32_be(0x200), 42);
}

#[test]
fn store_forwards_the_latest_alu_result() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 0x200)); // R1 = address
    image.extend_from_slice(&encode_i(0x04, 2, 0, 0x1234)); // R2 = value
    image.extend_from_slice(&encode_i(0x02, 2, 1, 0)); // ST immediately consumes R2
    image.extend_from_slice(&encode_i(0x01, 3, 1, 0)); // LD to observe stored value
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(64);
    assert_eq!(cpu.read_mem_u32_be(0x200), 0x1234);
    assert_eq!(cpu.read_reg(3), 0x1234);
    assert_eq!(cpu.stall_cycles(), 0);
}

#[test]
fn byte_halfword_and_half_immediates() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 0x300)); // R1 = 0x300
    image.extend_from_slice(&encode_i(0x04, 2, 0, 0x12)); // R2 = 0x12
    image.extend_from_slice(&encode_i(0x04, 3, 0, 0x34)); // R3 = 0x34
    image.extend_from_slice(&encode_i(0x15, 2, 1, 0)); // STB [R1], R2
    image.extend_from_slice(&encode_i(0x16, 3, 1, 1)); // STH [R1+1], R3
    image.extend_from_slice(&encode_i(0x13, 4, 1, 0)); // LDB R4, [R1] = 0x12
    image.extend_from_slice(&encode_i(0x14, 5, 1, 1)); // LDH R5, [R1+1] = 0x0034
    image.extend_from_slice(&encode_i(0x3D, 6, 0, 0xABCDu16 as i16)); // LDIH R6
    image.extend_from_slice(&encode_i(0x3C, 6, 0, 0x1234)); // LDIL R6 -> 0xABCD1234
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(128);
    assert_eq!(cpu.read_reg(4), 0x12);
    assert_eq!(cpu.read_reg(5), 0x34);
    assert_eq!(cpu.read_reg(6), 0xABCD_1234);
}

fn encode_short(raw: u16) -> [u8; 2] {
    raw.to_be_bytes()
}

#[test]
fn short_mode_roundtrip() {
    let mut cpu = boot();
    let mut image = vec![0u8; 0x40];
    // Normal @0: JMPS to 0x20; HALT guard at 0x4.
    image[0x00..0x04].copy_from_slice(&encode_i(0x1D, 0, 0, 0x1C));
    image[0x04..0x08].copy_from_slice(&encode_r(0x3F, 0, 0, 0));
    // Short @0x20.
    image[0x20..0x22].copy_from_slice(&encode_short(0x9105)); // S.LDI R1, 5
    image[0x22..0x24].copy_from_slice(&encode_short(0x9207)); // S.LDI R2, 7
    image[0x24..0x26].copy_from_slice(&encode_short(0x1312)); // S.ADD R3, R1, R2
    image[0x26..0x28].copy_from_slice(&encode_short(0x5002)); // S.BZ R0, +2 (taken)
    image[0x28..0x2A].copy_from_slice(&encode_short(0x94AA)); // skipped
    image[0x2A..0x2C].copy_from_slice(&encode_short(0x9455)); // S.LDI R4, 0x55
    image[0x2C..0x2E].copy_from_slice(&encode_short(0xF000)); // S.RET
    // Normal @0x2E.
    image[0x2E..0x32].copy_from_slice(&encode_r(0x03, 5, 3, 4)); // ADD R5, R3, R4
    image[0x32..0x36].copy_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(128);
    assert_eq!(cpu.read_reg(1), 5);
    assert_eq!(cpu.read_reg(2), 7);
    assert_eq!(cpu.read_reg(3), 12);
    assert_eq!(cpu.read_reg(4), 0x55);
    assert_eq!(cpu.read_reg(5), 12 + 0x55);
    assert_eq!(cpu.instruction_mode(), cpt32::cpu::InstructionMode::Normal);
}

#[test]
fn cpuid_advertises_p_and_r0_stays_zero() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_r(0x3E, 0, 0, 0)); // CPUID
    image.extend_from_slice(&encode_r(0x03, 0, 1, 2)); // ADD R0, R1, R2 (ignored)
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(32);
    assert_eq!(cpu.read_reg(1), 0x5352_4332);
    assert_eq!(cpu.read_reg(2), 0x3F | 0x80); // Base|A|L|M|S|I + p
    assert_eq!(cpu.read_reg(0), 0);
}

#[test]
fn halt_wakes_on_irq_and_returns_past_halt() {
    let mut cpu = boot();
    let mut image = Vec::new();
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT @0
    image.extend_from_slice(&encode_i(0x04, 1, 0, 42)); // ADDI R1, 42 @4
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT @8
    cpu.load_program(0, &image);
    cpu.write_mem_u32_be(0xFFFF_0100, 0x8000_0000); // IRET slot (op 0x20)
    cpu.run(8);
    assert!(!cpu.is_running());
    cpu.write_mem_u8(0xFFFF_0045, 0x01); // enable IRQC source 0
    cpu.set_irq_source(0, true);
    cpu.set_irq_source(0, false);
    cpu.run(64);
    assert_eq!(cpu.read_reg(1), 42);
    assert!(!cpu.is_running());
}

fn run_scalar(image: &[u8], mem_init: &[(u32, u32)]) -> ([u32; 32], u32, Vec<(u32, u32)>) {
    use cpt32::cpu::Cpu;
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    for &(addr, val) in mem_init {
        cpu.write_mem_u32_be(addr, val);
    }
    cpu.load_program(0, image);
    for _ in 0..100 {
        cpu.run(64);
        if !cpu.is_running() {
            break;
        }
    }
    let mut regs = [0u32; 32];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = cpu.read_reg(i);
    }
    let pc = cpu.pc();
    let mem = mem_init
        .iter()
        .map(|&(addr, _)| (addr, cpu.read_mem_u32_be(addr)))
        .collect();
    (regs, pc, mem)
}

fn run_pipelined(image: &[u8], mem_init: &[(u32, u32)]) -> ([u32; 32], u32, Vec<(u32, u32)>) {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = PipelineCpu::new(bus);
    for &(addr, val) in mem_init {
        cpu.write_mem_u32_be(addr, val);
    }
    cpu.load_program(0, image);
    for _ in 0..100 {
        cpu.run(64);
        if !cpu.is_running() {
            break;
        }
    }
    let mut regs = [0u32; 32];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = cpu.read_reg(i);
    }
    let pc = cpu.pc();
    let mem = mem_init
        .iter()
        .map(|&(addr, _)| (addr, cpu.read_mem_u32_be(addr)))
        .collect();
    (regs, pc, mem)
}

#[test]
fn differential_mixed_program_matches_scalar() {
    // Exercises ALU chain w/ forwarding, load-use, store-load forwarding via
    // memory, taken + not-taken branches, JAL/JR, MUL/DIV/MOD, byte/halfword
    // accesses, LDIL/LDIH, SLT/SLTU, shifts, and CPUID (except R1/R2, which
    // differ by design: the pipeline advertises Extension p).
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 0x100)); // R1 = 0x100
    image.extend_from_slice(&encode_i(0x04, 2, 0, 7)); // R2 = 7
    image.extend_from_slice(&encode_i(0x01, 3, 1, 0)); // LD R3, [R1] (mem[0x100])
    image.extend_from_slice(&encode_r(0x03, 4, 3, 2)); // ADD R4, R3, R2 (load-use)
    image.extend_from_slice(&encode_r(0x18, 5, 4, 2)); // MUL R5, R4, R2
    image.extend_from_slice(&encode_r(0x19, 6, 5, 2)); // DIV R6, R5, R2
    image.extend_from_slice(&encode_r(0x1A, 7, 5, 2)); // MOD R7, R5, R2
    image.extend_from_slice(&encode_i(0x02, 6, 1, 4)); // ST [R1+4], R6
    image.extend_from_slice(&encode_i(0x01, 8, 1, 4)); // LD R8, [R1+4]
    image.extend_from_slice(&encode_i(0x07, 8, 6, 4)); // BEQ R8, R6, +4 (taken)
    image.extend_from_slice(&encode_i(0x04, 9, 0, 1)); // skipped
    image.extend_from_slice(&encode_i(0x04, 9, 0, 2)); // R9 = 2
    image.extend_from_slice(&encode_i(0x08, 2, 1, 4)); // BNE R1, R2, +4 (taken)
    image.extend_from_slice(&encode_i(0x04, 10, 0, 1)); // skipped
    image.extend_from_slice(&encode_i(0x04, 10, 0, 3)); // R10 = 3
    image.extend_from_slice(&encode_i(0x07, 1, 2, 4)); // BEQ R1, R2, +4 (not taken)
    image.extend_from_slice(&encode_i(0x04, 11, 0, 4)); // R11 = 4
    image.extend_from_slice(&encode_i(0x3D, 12, 0, 0x1234)); // LDIH R12
    image.extend_from_slice(&encode_i(0x3C, 12, 0, 0x5678)); // LDIL R12 = 0x12345678
    image.extend_from_slice(&encode_r(0x06, 13, 2, 1)); // SLT R13, R2, R1 (7 < 0x100)
    image.extend_from_slice(&encode_r(0x17, 14, 1, 2)); // SLTU R14, R1, R2 (no)
    image.extend_from_slice(&encode_r(0x0F, 15, 2, 2)); // SLL R15, R2, R2 (7<<7)
    image.extend_from_slice(&encode_r(0x12, 16, 15, 2)); // SRA R16, R15, R2
    image.extend_from_slice(&encode_i(0x15, 2, 1, 8)); // STB [R1+8], R2
    image.extend_from_slice(&encode_i(0x13, 17, 1, 8)); // LDB R17 = 7
    image.extend_from_slice(&encode_i(0x16, 12, 1, 10)); // STH [R1+10], R12
    image.extend_from_slice(&encode_i(0x14, 18, 1, 10)); // LDH R18 = 0x5678
    image.extend_from_slice(&encode_r(0x19, 19, 2, 0)); // DIV R19, R2, R0 = 0
    image.extend_from_slice(&encode_i(0x0A, 0, 0, 8)); // JAL +8 -> subroutine
    image.extend_from_slice(&encode_i(0x04, 20, 0, 9)); // R20 = 9 (after return)
    image.extend_from_slice(&encode_i(0x09, 0, 0, 8)); // JMP +8 -> HALT
    image.extend_from_slice(&encode_r(0x03, 21, 0, 0)); // subroutine: ADD R21, R0, R0
    image.extend_from_slice(&encode_r(0x21, 22, 0, 0)); // JALR R22? no: JR back
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    // Fix the subroutine return: overwrite the JALR slot with JR R31.
    let jr_pos = image.len() - 8;
    image[jr_pos..jr_pos + 4].copy_from_slice(&encode_r(0x0B, 31, 0, 0));

    let mem_init = [
        (0x100u32, 100u32),
        (0x104, 0u32),
        (0x108, 0u32),
        (0x10C, 0u32),
    ];
    let (s_regs, s_pc, s_mem) = run_scalar(&image, &mem_init);
    let (p_regs, p_pc, p_mem) = run_pipelined(&image, &mem_init);
    // R1/R2 excluded: CPUID is not in this program, so all regs must match.
    assert_eq!(s_regs, p_regs, "register file diverged");
    assert_eq!(s_pc, p_pc, "PC diverged");
    assert_eq!(s_mem, p_mem, "memory diverged");
}

#[test]
fn mulh_divu_and_division_edges() {
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x3D, 1, 0, 0x8000u16 as i16)); // LDIH R1, 0x8000
    image.extend_from_slice(&encode_i(0x3C, 1, 0, 0x0000)); // LDIL R1, 0 -> R1 = INT_MIN
    image.extend_from_slice(&encode_i(0x04, 2, 0, -1)); // R2 = -1
    image.extend_from_slice(&encode_r(0x19, 3, 1, 2)); // DIV INT_MIN / -1 = INT_MIN
    image.extend_from_slice(&encode_r(0x1A, 4, 1, 2)); // MOD INT_MIN % -1 = 0
    image.extend_from_slice(&encode_r(0x19, 5, 1, 0)); // DIV x, 0 = 0
    image.extend_from_slice(&encode_r(0x1C, 6, 1, 2)); // DIVU
    image.extend_from_slice(&encode_r(0x1B, 7, 1, 1)); // MULH INT_MIN * INT_MIN
    image.extend_from_slice(&encode_r(0x1A, 8, 5, 0)); // MOD x, 0 = 0
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    let mem_init: [(u32, u32); 0] = [];
    let (s_regs, _, _) = run_scalar(&image, &mem_init);
    let (p_regs, _, _) = run_pipelined(&image, &mem_init);
    assert_eq!(s_regs, p_regs, "register file diverged");
    assert_eq!(p_regs[3], 0x8000_0000);
    assert_eq!(p_regs[4], 0);
    assert_eq!(p_regs[5], 0);
    assert_eq!(p_regs[8], 0);
}
