// SRC32: Scalable RISC 32-bit CPU
// Minimal and Scalable CPU ISA for learning and experimentation
/*
Instruction Set Architecture (ISA):
Registers:
- 32 general-purpose registers (R0-R31), PC (Program Counter), and special registers for control and status
- R0 is hardwired to zero (always reads as 0, writes are ignored)
- R1-R31 are general-purpose registers for arithmetic, logic, and memory operations
- R28: SP (Stack Pointer), R29: FP (Frame Pointer), 
- R30: GP (Global Pointer), R31: LR (Link Register)
Instruction Format:
- 40-bit (5 bytes) fixed-length instructions
Mode 0 (Register): [opcode (8 bits) | 00 (mode) | rd (5 bits) | rs1 (5 bits) | rs2 (5 bits) | unused (all zero, 15 bits)]
Mode 1 (Immediate): [opcode (8 bits) | 01 (mode) | rd (5 bits) | rs1 (5 bits) | imm (16 bits) | unused (all zero, 4 bits)]
Mode 2 (Memory): [opcode (8 bits) | 10 (mode) | rd (5 bits) | base (5 bits) | offset (16 bits) | unused (all zero, 4 bits)]
Mode 3 (Extention): Reserved for extended instructions
LDI: [111 (opcode) | rd (5 bits) | imm (32 bits)]
Note: 0xE0~0xFF is reserved for LDI instruction (since rd uses LSB 5 bits)
Operations:
- 0x00: NOP (No Operation)
- 0x01: MOV rd, rs1 (Move register)
*/
struct RAM {
    data: Vec<u8>, // Simple byte-addressable memory
}
struct Bus {
    memory: RAM, // Simple memory model (byte-addressable)
}
enum Instruction {

}
struct CPU {
    reg: [u32; 32], // 32 general-purpose registers
    pc: u32,        // Program Counter
    flags: u32,     // Status flags (e.g., zero, carry, overflow)
    running: bool,  // CPU running state
}

impl CPU {

}
fn main() {
    println!("Placeholder");
}