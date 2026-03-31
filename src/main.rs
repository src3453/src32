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
    addr: u32,
    device: Box<dyn Device>,
    size: u32, // Size of the device's addressable space
} 
trait Device {
    // Device interface for memory-mapped I/O
    // All devices (RAM, ROM, MMIO peripherals) must implement this trait
    fn read(&self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, value: u8);
    fn size(&self) -> u32; // Return the size of the device's addressable space
}

struct RAM {
    data: Vec<u8>, // RAM storage as a device
}

impl Device for RAM { // Implement Device trait for RAM
    fn read(&self, addr: u32) -> u8 {
        self.data[addr as usize]
    }
    fn write(&mut self, addr: u32, value: u8) {
        self.data[addr as usize] = value;
    }

    fn size(&self) -> u32 {
        self.data.len() as u32
    }
}

impl RAM {
    fn new(size: usize) -> Self {
        RAM {
            data: vec![0; size],
        }
    }
}

struct Bus {
    devices: Vec<DeviceMap>, // List of memory-mapped devices
}

const MMIO_START: u32 = 0xFFFF0000; // Start of memory-mapped I/O region
const RAM_SIZE: usize = 0x10000; // 64KB of RAM for the system

impl Bus {
    fn new() -> Self {
        Bus {
            devices: Vec::new(),
        }
    }
    // Add a memory-mapped device to the bus
    // if size is 0, the device will not be mapped and will not be accessible
    fn add_device(&mut self, addr: u32, device: Box<dyn Device>) {
        let size = device.size();
        if size == 0 {
            return;
        }
        let new_end = addr.checked_add(size)
            .expect("Device range overflow");

        for m in &self.devices {
            let end = m.addr.checked_add(m.size)
                .expect("Existing device range overflow");

            if !(new_end <= m.addr || addr >= end) {
                panic!(
                    "Device overlap detected: new [0x{:08X}-0x{:08X}) overlaps with [0x{:08X}-0x{:08X})",
                    addr, new_end, m.addr, end
                );
            }
        }
        self.devices.push(DeviceMap { addr, size, device });
    }
    // Find the device responsible for a given address (if any)
    fn find_device(&self, addr: u32) -> Option<(&dyn Device, u32)> {
        for m in &self.devices {
            let end = m.addr.checked_add(m.size)
                .expect("Existing device range overflow");
            if addr >= m.addr && addr < end {
                return Some((&*m.device, addr - m.addr));
            }
        }
        None
    }
    // Mutable version of find_device for write operations
    fn find_device_mut(&mut self, addr: u32) -> Option<(&mut dyn Device, u32)> {
        for m in &mut self.devices {
            let end = m.addr.checked_add(m.size)
                .expect("Existing device range overflow");
            if addr >= m.addr && addr < end {
                return Some((&mut *m.device, addr - m.addr));
            }
        }
        None
    }
    fn read_u8(&self, addr: u32) -> u8 {
        if let Some((device, offset)) = self.find_device(addr) {
            device.read(offset)
        } else {
            panic!("Invalid I/O read: 0x{:08X}", addr);
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
    fn write_u8(&mut self, addr: u32, value: u8) {
        if let Some((device, offset)) = self.find_device_mut(addr) {
            device.write(offset, value);
        } else {
            panic!("Invalid I/O write: 0x{:08X}", addr);
        }
    }
    fn write_u32(&mut self, addr: u32, value: u32) {
        // Big-endian write (store most significant byte at first address)
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
    bus: Bus,       // System bus for memory and I/O access
}

impl CPU {
    fn new() -> Self {
        let mut bus = Bus::new();
        // Initialize RAM and add it to the bus
        let ram = Box::new(RAM::new(RAM_SIZE));
        bus.add_device(0, ram); // Map RAM to address 0
        CPU {
            reg: [0; 32],
            pc: 0,
            running: true,
            bus,
        }
    }
    fn read_reg(&self, reg: usize) -> u32 {
        if reg >= self.reg.len() {
            panic!("Invalid register index: {}", reg);
        }
        if reg != REG_ZERO {
            self.reg[reg]
        } else {
            0 // R0 always reads as zero
        }
    }
    fn write_reg(&mut self, reg: usize, value: u32) {
        if reg >= self.reg.len() {
            panic!("Invalid register index: {}", reg);
        }
        if reg == REG_ZERO { // Skip writes to R0
            return;
        }
        self.reg[reg] = value; // Write value to register (except R0)
    }
}
fn main() {
    println!("Placeholder");
}