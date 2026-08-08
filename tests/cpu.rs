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
    assert_eq!(cpu.read_reg(2), 0x0F);
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
