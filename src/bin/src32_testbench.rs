//! Headless SRC32 execution harness for compiler pipeline smoke tests.
use std::env;
use std::fs;

use cpt32::bus::Bus;
use cpt32::cpu::Cpu;
use cpt32::devices::ram::connect_ram;

fn main() {
    let path = env::args()
        .nth(1)
        .expect("usage: src32_testbench PROGRAM.bin [expected]");
    let expected: u32 = env::args()
        .nth(2)
        .unwrap_or_else(|| "0".to_string())
        .parse()
        .expect("expected must be an unsigned integer");
    let image = fs::read(&path).expect("failed to read program binary");

    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = Cpu::new(bus);
    cpu.load_program(0, &image);
    cpu.run(1_000_000);

    let result = cpu.read_reg(1);
    assert!(
        !cpu.is_running(),
        "program did not halt (pc=0x{:08X})",
        cpu.pc()
    );
    assert_eq!(
        result, expected,
        "R1 mismatch: got {result}, expected {expected}"
    );
    println!("PASS: {path} => R1={result}");
}
