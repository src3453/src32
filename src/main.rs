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
SRC32: Base level operations
    Load/Store:
    - 0x00: NOP (No Operation)
    - 0x01: LD rd, [base + offset] (Load from memory)
    - 0x02: ST rd, [base + offset] (Store to memory)
    - 0xE0-0xFF: LDI rd, imm (Load Immediate)
    ALU:
    - 0x03: ADD rd, rs1, rs2 (Add)
    - 0x04: ADDI rd, rs1, imm (Add Immediate)
    - 0x05: SUB rd, rs1, rs2 (Subtract)
    - 0x06: SLT rd, rs1, rs2 (Set if Less Than)
    Branch:
    - 0x07: BEQ rs1, rs2, offset (Branch if Equal)
    - 0x08: BNE rs1, rs2, offset (Branch if Not Equal)
    - 0x09: JMP offset (Jump PC-relative address)
    Function Call:
    - 0x0A: JAL offset (Jump and Link)
    - 0x0B: JR rd (Jump Register)
    Special:
    - 0xDE: CPUID (Read CPU ID and features)
    - 0xDF: HALT (Stop execution)
JAL Behavior:
    R31 (LR) = PC + 5
    PC = PC + 5 + sign_extend(offset)
BEQ/BNE Behavior:
    if (rs1 == rs2 for BEQ, rs1 != rs2 for BNE):
    PC = PC + 5 + sign_extend(offset)
else:
    PC = PC + 5
SRC32-A: Advanced ALU Instructions
    - 0x0C: AND rd, rs1, rs2 (Bitwise AND)
    - 0x0D: OR rd, rs1, rs2 (Bitwise OR)
    - 0x0E: XOR rd, rs1, rs2 (Bitwise XOR)
    - 0x0F: SLL rd, rs1, rs2 (Shift Left Logical)
    - 0x10: SRL rd, rs1, rs2 (Shift Right Logical)
    - 0x11: SLA rd, rs1, rs2 (Shift Left Arithmetic)
    - 0x12: SRA rd, rs1, rs2 (Shift Right Arithmetic)
*/
struct DeviceMap {
    // Memory-mapped I/O device mapping
    addr_start: u32,
    addr_end: u32,
    device: Box<dyn Device>,
} 
trait Device {
    // Device interface for memory-mapped I/O
    fn read(&self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, value: u8);
}

struct Bus {
    memory: Vec<u8>, // Simple memory model (byte-addressable)
    devices: Vec<DeviceMap>, // List of memory-mapped devices
}

const MMIO_START: u32 = 0xFFFF0000; // Start of memory-mapped I/O region

impl Bus {
    fn new(size: usize) -> Self {
        Bus {
            memory: vec![0; size],
            devices: Vec::new(),
        }
    }
    fn read_mmio(&self, addr: u32) -> u8 {
        // Handle MMIO read (for simplicity, return 0)
        match addr {
            0xFFFF0000 => {
                // UART Data Register (read returns received byte or 0 if empty)
                0 // Placeholder: always return 0 (no data)
            }
            _ => panic!("Unhandled MMIO read: 0x{:08X}", addr),
        }
    }
    fn read_u8(&self, addr: u32) -> u8 {
        if addr >= MMIO_START {
            self.read_mmio(addr)
        } else if addr < self.memory.len() as u32 {
            self.memory[addr as usize]
        } else {
            panic!("Memory read out of bounds: 0x{:08X}", addr);
        }
    }
    fn read_u32(&self, addr: u32) -> u32 {
        // Big-endian read (combine 4 bytes into a 32-bit word)
        let b0 = self.read_u8(addr) as u32;
        let b1 = self.read_u8(addr + 1) as u32;
        let b2 = self.read_u8(addr + 2) as u32;
        let b3 = self.read_u8(addr + 3) as u32;
        (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
    }
    fn write_mmio(&mut self, addr: u32, value: u8) {
        // Handle MMIO write
        match addr {
            0xFFFF0000 => {
                // UART Data Register (write sends byte to output)
                print!("{}", value as char); // Output character to console
            }
            _ => panic!("Unhandled MMIO write: 0x{:08X}", addr),
        }
    }
    fn write_u8(&mut self, addr: u32, value: u8) {
        if addr >= MMIO_START {
            self.write_mmio(addr, value);
        } else if addr < self.memory.len() as u32 {
            self.memory[addr as usize] = value;
        } else {
            panic!("Memory write out of bounds: 0x{:08X}", addr);
        }
    }
    fn write_u32(&mut self, addr: u32, value: u32) {
        // Big-endian write (store least significant byte at lowest address)
        self.write_u8(addr + 0, ((value >> 24) & 0xFF) as u8);
        self.write_u8(addr + 1, ((value >> 16) & 0xFF) as u8);
        self.write_u8(addr + 2, ((value >> 8) & 0xFF) as u8);
        self.write_u8(addr + 3, (value & 0xFF) as u8);
    }
}
enum Instruction {
    // SRC32 Base Instructions
    NOP,
    LD { rd: u8, base: u8, offset: i16 },
    ST { rd: u8, base: u8, offset: i16 },
    LDI { rd: u8, imm: u32 },
    ADD { rd: u8, rs1: u8, rs2: u8 },
    ADDI { rd: u8, rs1: u8, imm: i16 },
    SUB { rd: u8, rs1: u8, rs2: u8 },
    SLT { rd: u8, rs1: u8, rs2: u8 },
    BEQ { rs1: u8, rs2: u8, offset: i16 },
    BNE { rs1: u8, rs2: u8, offset: i16 },
    JMP { offset: i16 },
    JAL { offset: i16 },
    JR { rd: u8 },
    CPUID,
    HALT,
    // Extention A: Advanced ALU Instructions
    AND { rd: u8, rs1: u8, rs2: u8 },
    OR { rd: u8, rs1: u8, rs2: u8 },
    XOR { rd: u8, rs1: u8, rs2: u8 },
    SLL { rd: u8, rs1: u8, rs2: u8 },
    SRL { rd: u8, rs1: u8, rs2: u8 },
    SLA { rd: u8, rs1: u8, rs2: u8 },
    SRA { rd: u8, rs1: u8, rs2: u8 },
}

const REG_ZERO: usize = 0; // R0 is hardwired to zero
const EXT_BASE: u32 = 0x01; // Extension Flag bit in opcode (bit 0)
const EXT_A: u32 = 0x02; // Extension A flag (bit 0)
const CPU_FEATURES: u32 = EXT_BASE | EXT_A; // CPU Extension support flags

struct CPU {
    reg: [u32; 32], // 32 general-purpose registers
    pc: u32,        // Program Counter
    running: bool,  // CPU running state
}

impl CPU {

}
fn main() {
    println!("Placeholder");
}