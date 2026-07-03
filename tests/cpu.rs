use cpt32::cpu::Cpu;
use cpt32::bus::Bus;
use cpt32::devices::ram::connect_ram;

fn encode_r(op: u8, rd: u8, rs1: u8, rs2: u8) -> [u8; 5] {
    let raw = ((op as u64) << 32)
        | ((0u64) << 30)
        | ((rd as u64) << 25)
        | ((rs1 as u64) << 20)
        | ((rs2 as u64) << 15);
    [
        ((raw >> 32) & 0xFF) as u8,
        ((raw >> 24) & 0xFF) as u8,
        ((raw >> 16) & 0xFF) as u8,
        ((raw >> 8) & 0xFF) as u8,
        (raw & 0xFF) as u8,
    ]
}

fn encode_i(op: u8, rd: u8, rs1: u8, imm: i16) -> [u8; 5] {
    let imm_bits = (imm as u16) as u64;
    let raw = ((op as u64) << 32)
        | ((1u64) << 30)
        | ((rd as u64) << 25)
        | ((rs1 as u64) << 20)
        | (imm_bits << 4);
    [
        ((raw >> 32) & 0xFF) as u8,
        ((raw >> 24) & 0xFF) as u8,
        ((raw >> 16) & 0xFF) as u8,
        ((raw >> 8) & 0xFF) as u8,
        (raw & 0xFF) as u8,
    ]
}

fn encode_cpuid() -> [u8; 5] {
    encode_r(0xDE, 0, 0, 0)
}

#[test]
fn arithmetic_and_halt() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&[0xE1, 0x00, 0x00, 0x00, 0x05]); // LDI R1, 5
    image.extend_from_slice(&[0xE2, 0x00, 0x00, 0x00, 0x07]); // LDI R2, 7
    image.extend_from_slice(&encode_r(0x03, 3, 1, 2)); // ADD R3, R1, R2
    image.extend_from_slice(&encode_r(0xDF, 0, 0, 0)); // HALT

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

    image.extend_from_slice(&[0xE1, 0x00, 0x00, 0x00, 0x01]); // LDI R1, 1
    image.extend_from_slice(&[0xE2, 0x00, 0x00, 0x00, 0x01]); // LDI R2, 1
    image.extend_from_slice(&encode_i(0x07, 2, 1, 5)); // BEQ R1, R2, +5
    image.extend_from_slice(&[0xE3, 0x00, 0x00, 0x00, 0xAA]); // skipped if branch taken
    image.extend_from_slice(&[0xE3, 0x00, 0x00, 0x00, 0x55]); // target
    image.extend_from_slice(&encode_r(0xDF, 0, 0, 0)); // HALT

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

    image.extend_from_slice(&[0xE1, 0x00, 0x00, 0x00, 0x2A]); // LDI R1, 42
    image.extend_from_slice(&encode_r(0xDF, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);

    assert_eq!(cpu.disassemble_at(0), "LDI R1, 0x0000002A");
    assert!(cpu.step_once());
    assert_eq!(cpu.pc(), 5);
    assert_eq!(cpu.read_reg(1), 42);
}

#[test]
fn cpuid_reports_updated_features() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&encode_cpuid());
    image.extend_from_slice(&encode_r(0xDF, 0, 0, 0)); // HALT

    cpu.load_program(0, &image);
    cpu.run(16);

    assert_eq!(cpu.read_reg(1), 0x5352_4332);
    assert_eq!(cpu.read_reg(2), 0x0F);
}

#[test]
fn extension_m_and_sltu() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    let mut image = Vec::new();

    image.extend_from_slice(&[0xE1, 0x00, 0x00, 0x00, 0x01]); // LDI R1, 1
    image.extend_from_slice(&[0xE2, 0x00, 0x00, 0x00, 0x02]); // LDI R2, 2
    image.extend_from_slice(&encode_r(0x17, 3, 1, 2)); // SLTU R3, R1, R2
    image.extend_from_slice(&encode_r(0x17, 4, 2, 1)); // SLTU R4, R2, R1
    image.extend_from_slice(&[0xE5, 0xFF, 0xFF, 0xFF, 0xFF]); // LDI R5, -1
    image.extend_from_slice(&[0xE6, 0x00, 0x00, 0x00, 0x02]); // LDI R6, 2
    image.extend_from_slice(&encode_r(0x18, 7, 5, 6)); // MUL R7, R5, R6
    image.extend_from_slice(&encode_r(0x1B, 8, 5, 6)); // MULH R8, R5, R6
    image.extend_from_slice(&[0xEA, 0xFF, 0xFF, 0xFF, 0xF7]); // LDI R10, -9
    image.extend_from_slice(&[0xEB, 0x00, 0x00, 0x00, 0x04]); // LDI R11, 4
    image.extend_from_slice(&encode_r(0x19, 12, 10, 11)); // DIV R12, R10, R11
    image.extend_from_slice(&encode_r(0x1A, 13, 10, 11)); // MOD R13, R10, R11
    image.extend_from_slice(&[0xEE, 0x00, 0x00, 0x00, 0x09]); // LDI R14, 9
    image.extend_from_slice(&[0xEF, 0x00, 0x00, 0x00, 0x04]); // LDI R15, 4
    image.extend_from_slice(&encode_r(0x1C, 16, 14, 15)); // DIVU R16, R14, R15
    image.extend_from_slice(&encode_r(0xDF, 0, 0, 0)); // HALT

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
