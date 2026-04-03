use src32::cpu::Cpu;

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

#[test]
fn arithmetic_and_halt() {
    let mut cpu = Cpu::new();
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
    let mut cpu = Cpu::new();
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
