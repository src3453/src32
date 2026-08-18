use cpt32::cpu::Cpu;
use cpt32::bus::Bus;
use cpt32::devices::ram::connect_ram;

fn encode_r(op: u8, rd: u8, rs1: u8, rs2: u8) -> [u8; 4] {
    let raw = ((op as u32) << 26)
        | ((rd as u32) << 21)
        | ((rs1 as u32) << 16)
        | ((rs2 as u32) << 11);
    raw.to_be_bytes()
}

fn encode_i(op: u8, rd: u8, rs1: u8, imm: i16) -> [u8; 4] {
    let raw = ((op as u32) << 26)
        | ((rd as u32) << 21)
        | ((rs1 as u32) << 16)
        | (imm as u16 as u32);
    raw.to_be_bytes()
}

fn encode_ldi32(rd: u8, imm: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&encode_i(0x3D, rd, 0, (imm >> 16) as u16 as i16));
    bytes.extend_from_slice(&encode_i(0x3C, rd, 0, imm as u16 as i16));
    bytes
}

fn encode_cpuid() -> [u8; 4] {
    encode_r(0x3E, 0, 0, 0)
}

fn encode_iret() -> [u8; 4] {
    encode_r(0x20, 0, 0, 0)
}

fn encode_short(raw: u16) -> [u8; 2] {
    raw.to_be_bytes()
}

fn s_ldi(rd: u8, imm: u8) -> [u8; 2] {
    encode_short(((0x9u16) << 12) | (((rd & 0x0F) as u16) << 8) | imm as u16)
}

fn s_add(rd: u8, rs1: u8, rs2: u8) -> [u8; 2] {
    encode_short(
        ((0x1u16) << 12)
            | (((rd & 0x0F) as u16) << 8)
            | (((rs1 & 0x0F) as u16) << 4)
            | ((rs2 & 0x0F) as u16),
    )
}

fn s_ret() -> [u8; 2] {
    encode_short(0xF000)
}

#[test]
fn arithmetic_and_halt() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_ldi32(1, 5));
    image.extend_from_slice(&encode_ldi32(2, 7));
    image.extend_from_slice(&encode_r(0x03, 3, 1, 2)); // ADD R3, R1, R2
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(32);

    assert_eq!(cpu.read_reg(3), 12);
}

#[test]
fn branch_taken() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_ldi32(1, 1));
    image.extend_from_slice(&encode_ldi32(2, 1));
    image.extend_from_slice(&encode_i(0x07, 2, 1, 4)); // BEQ R1, R2, +4
    image.extend_from_slice(&encode_ldi32(3, 0xAA)); // skipped if branch taken
    image.extend_from_slice(&encode_ldi32(3, 0x55)); // target
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(64);

    assert_eq!(cpu.read_reg(3), 0x55);
}

#[test]
fn step_once_and_disassemble() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_i(0x3C, 1, 0, 0x002A)); // LDIL R1, 42
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);

    assert_eq!(cpu.disassemble_at(0), "LDIL R1, 0x002A");
    assert!(cpu.step_once());
    assert_eq!(cpu.pc(), 4);
    assert_eq!(cpu.read_reg(1), 42);
}

#[test]
fn cpuid_reports_updated_features() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_cpuid());
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(16);

    assert_eq!(cpu.read_reg(1), 0x5352_4332);
    assert_eq!(cpu.read_reg(2), 0x3F);
}

#[test]
fn irq_is_edge_triggered_and_iret_restores_state() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();
    image.extend_from_slice(&encode_r(0x03, 1, 1, 1));
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0));
    image.extend_from_slice(&encode_r(0x03, 2, 2, 2));
    image.extend_from_slice(&encode_iret());
    cpu.load_program(0, &image);

    cpu.set_irq_input(true, 3);
    assert!(cpu.step_once());
    assert_eq!(cpu.epc(), 4);
    assert_eq!(cpu.irq_cause(), 3);
    assert_eq!(cpu.pc(), 0xFFFF_010C);
    assert!(!cpu.irq_enabled());
}

#[test]
fn cpu_reserved_vectors_and_registers_are_mmio_mapped() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);

    cpu.write_mem_u32_be(0xFFFF_0000, 0x0000_1234);
    cpu.write_mem_u32_be(0xFFFF_0004, 0x0000_5678);
    cpu.write_mem_u32_be(0xFFFF_0204, 0xCAFE_BABE);

    assert_eq!(cpu.read_mem_u32_be(0xFFFF_0000), 0x0000_1234);
    assert_eq!(cpu.read_mem_u32_be(0xFFFF_0004), 0x0000_5678);
    assert_eq!(cpu.read_mem_u32_be(0xFFFF_0204), 0xCAFE_BABE);
    assert_eq!(cpu.read_mem_u32_be(0xFFFF_0280), cpu.pc());
}

#[test]
fn debugger_bus_error_does_not_panic_or_enter_vector() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let pc = cpu.pc();

    assert_eq!(cpu.read_mem_u8(0x9000_0000), 0);
    assert_eq!(cpu.read_debug_mem_u8(0x9000_0000), None);
    cpu.write_mem_u8(0x9000_0000, 0x12);

    assert_eq!(cpu.pc(), pc);
    assert!(cpu.step_once());
    assert_eq!(cpu.pc(), pc + 4);
}

#[test]
fn extension_m_and_sltu() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_ldi32(1, 1));
    image.extend_from_slice(&encode_ldi32(2, 2));
    image.extend_from_slice(&encode_r(0x17, 3, 1, 2)); // SLTU R3, R1, R2
    image.extend_from_slice(&encode_r(0x17, 4, 2, 1)); // SLTU R4, R2, R1
    image.extend_from_slice(&encode_ldi32(5, 0xFFFF_FFFF));
    image.extend_from_slice(&encode_ldi32(6, 2));
    image.extend_from_slice(&encode_r(0x18, 7, 5, 6)); // MUL R7, R5, R6
    image.extend_from_slice(&encode_r(0x1B, 8, 5, 6)); // MULH R8, R5, R6
    image.extend_from_slice(&encode_ldi32(10, 0xFFFF_FFF7)); // -9
    image.extend_from_slice(&encode_ldi32(11, 4));
    image.extend_from_slice(&encode_r(0x19, 12, 10, 11)); // DIV R12, R10, R11
    image.extend_from_slice(&encode_r(0x1A, 13, 10, 11)); // MOD R13, R10, R11
    image.extend_from_slice(&encode_ldi32(14, 9));
    image.extend_from_slice(&encode_ldi32(15, 4));
    image.extend_from_slice(&encode_r(0x1C, 16, 14, 15)); // DIVU R16, R14, R15
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(128);

    assert_eq!(cpu.read_reg(3), 1);
    assert_eq!(cpu.read_reg(4), 0);
    assert_eq!(cpu.read_reg(7), 0xFFFF_FFFE);
    assert_eq!(cpu.read_reg(8), 0xFFFF_FFFF);
    assert_eq!(cpu.read_reg(12), 0xFFFF_FFFE);
    assert_eq!(cpu.read_reg(13), 0xFFFF_FFFF);
    assert_eq!(cpu.read_reg(16), 2);
}

#[test]
fn division_by_zero_returns_zero_without_panicking() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_ldi32(1, 123));
    image.extend_from_slice(&encode_ldi32(2, 0));
    image.extend_from_slice(&encode_r(0x19, 3, 1, 2)); // DIV
    image.extend_from_slice(&encode_r(0x1A, 4, 1, 2)); // MOD
    image.extend_from_slice(&encode_r(0x1C, 5, 1, 2)); // DIVU
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(64);

    assert_eq!(cpu.read_reg(3), 0);
    assert_eq!(cpu.read_reg(4), 0);
    assert_eq!(cpu.read_reg(5), 0);
}

#[test]
fn short_mode_jmps_executes_and_returns_to_normal() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_i(0x1D, 0, 0, 0)); // JMPS +0 (enter short mode)
    image.extend_from_slice(&s_ldi(1, 5)); // S.LDI R1, 5
    image.extend_from_slice(&s_ldi(2, 7)); // S.LDI R2, 7
    image.extend_from_slice(&s_add(3, 1, 2)); // S.ADD R3, R1, R2
    image.extend_from_slice(&s_ret()); // S.RET (back to normal)
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(64);

    assert_eq!(cpu.read_reg(3), 12);
    assert_eq!(cpu.pc(), 16);
}

#[test]
fn short_mode_reg15_maps_to_lr() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_i(0x1D, 0, 0, 0)); // JMPS +0
    image.extend_from_slice(&s_ldi(15, 0x2A)); // S.LDI R15(short) -> R31
    image.extend_from_slice(&s_ret());
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0));

    cpu.load_program(0, &image);
    cpu.run(32);

    assert_eq!(cpu.read_reg(31), 0x2A);
}
