use src32::cpu::Cpu;

fn main() {
    let mut cpu = Cpu::new();

    // Small demo: LDI R3, 40; LDI R4, 2; ADD R5, R3, R4; HALT
    let program: [u8; 20] = [
        0xE3, 0x00, 0x00, 0x00, 0x28, // LDI R3, 40
        0xE4, 0x00, 0x00, 0x00, 0x02, // LDI R4, 2
        0x03, 0x0A, 0x32, 0x00, 0x00, // ADD R5, R3, R4
        0xDF, 0x00, 0x00, 0x00, 0x00, // HALT
    ];

    cpu.reset(0);
    cpu.load_program(0, &program);
    cpu.run(64);

    println!("R5 = {}", cpu.read_reg(5));
}
