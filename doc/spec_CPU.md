# SRC32 Specification
Revision 1.1 (2026-07-02)

# Changelog
- 1.1 (2026-07-02): Added Extension M (Multiplication and Division) instructions, and `SLTU` instruction in Extension A.
- 1.0 (2026-06-20): Initial release of the specification.

## 1. Overview

SRC32 is a 32-bit educational RISC CPU with fixed-length 32-bit instructions.
This repository includes:

- CPU emulator in Rust (`src/main.rs`)
- Assembler in Python (`asm.py`)

The goal is clarity and experimentation over micro-architectural complexity.

## 2. Registers

- General-purpose registers: `R0`-`R31` (32 registers, 32-bit)
- `R0` is hardwired to zero (reads as `0`, writes ignored)
- Convention-only aliases:
  - `R28`: `SP`
  - `R29`: `FP`
  - `R30`: `GP`
  - `R31`: `LR`
- Program Counter: `PC` (32-bit byte address)

## 3. Instruction Encoding

All instructions are 4 bytes (32 bits), big-endian bit layout.

### 3.1 Standard formats

- Register mode:
  - `[op:6][rd:5][rs1:5][rs2:5][unused:11]`
- Immediate mode:
  - `[op:6][rd:5][rs1:5][imm16:16]`
- Memory mode:
  - `[op:6][rd:5][base:5][off16:16]`

Bit positions:

- `op`: bits `31..26`
- `rd`: bits `25..21`
- `rs1/base`: bits `20..16`
- `rs2`: bits `15..11` (register mode only)
- `imm16/off16`: bits `15..0` (immediate/memory modes)

## 4. ISA

### 4.1 Base ISA

- `0x00 (reg)`: `NOP`: No operation
- `0x01 (mem)`: `LD rd, [base + off16]`: Load 32-bit word from memory at `base + off16` into `rd`
- `0x02 (mem)`: `ST [base + off16], rd`: Store 32-bit word from `rd` into memory at `base + off16`
- `0x03 (reg)`: `ADD rd, rs1, rs2`: Add `rs1` and `rs2`, store result in `rd`
- `0x04 (imm)`: `ADDI rd, rs1, imm16`: Add `rs1` and sign-extended `imm16`, store result in `rd`
- `0x05 (reg)`: `SUB rd, rs1, rs2`: Subtract `rs2` from `rs1`, store result in `rd`
- `0x06 (reg)`: `SLT rd, rs1, rs2` (signed): Set `rd = 1` if `rs1 < rs2`, else `rd = 0`
- `0x07 (imm)`: `BEQ rs1, rs2, off16`: Branch to `PC + 4 + sign_extend(off16)` if `rs1 == rs2`
- `0x08 (imm)`: `BNE rs1, rs2, off16`: Branch to `PC + 4 + sign_extend(off16)` if `rs1 != rs2`
- `0x09 (imm)`: `JMP off16`: Jump to `PC + 4 + sign_extend(off16)`
- `0x0A (imm)`: `JAL off16`: Jump and link: `R31 <- PC + 4`, then `PC <- PC + 4 + sign_extend(off16)`
- `0x0B (reg)`: `JR rd`: Jump to address in `rd`: `PC <- R[rd]`
- `0x3C (imm)`: `LDIL rd, imm16`: Load the low 16 bits into `rd`, preserving the upper 16 bits
- `0x3D (imm)`: `LDIH rd, imm16`: Load the high 16 bits into `rd`, preserving the lower 16 bits
- `0x3E (reg)`: `CPUID`: Write CPU ID/features to fixed registers:
  - `R1 <- CPU_ID`
  - `R2 <- CPU_FEATURES`
- `0x3F (reg)`: `HALT`: Stop execution loop
`BEQ`/`BNE` note:

- In encoding, `rs2` is stored in the `rd` field for immediate mode.

`CPUID` note:
- `CPU_ID` is a fixed 32-bit value identifying the CPU model.
  - `CPU_ID = 0x53524332` ("SRC2" in ASCII)
- `CPU_FEATURES` is a fixed 32-bit bitmask indicating supported extensions:
  - Bit 0: Base ISA
  - Bit 1: Extension A (ALU)
  - Bit 2: Extension L (Load/Store)
  - Bit 3: Extension M (Multiplication/Division)

### 4.2 Extension A (ALU)

- `0x0C (reg)`: `AND rd, rs1, rs2`: Bitwise AND of `rs1` and `rs2`, store result in `rd`
- `0x0D (reg)`: `OR rd, rs1, rs2`: Bitwise OR of `rs1` and `rs2`, store result in `rd`
- `0x0E (reg)`: `XOR rd, rs1, rs2`: Bitwise XOR of `rs1` and `rs2`, store result in `rd`
- `0x0F (reg)`: `SLL rd, rs1, rs2` (logical left): Shift `rs1` left by `rs2` bits, store result in `rd`
- `0x10 (reg)`: `SRL rd, rs1, rs2` (logical right): Shift `rs1` right by `rs2` bits, store result in `rd`
- `0x11 (reg)`: `SLA rd, rs1, rs2` (arithmetic left): Shift `rs1` left by `rs2` bits, store result in `rd`
- `0x12 (reg)`: `SRA rd, rs1, rs2` (arithmetic right): Shift `rs1` right by `rs2` bits, preserving sign, store result in `rd`
- `0x17 (reg)`: `SLTU rd, rs1, rs2` (unsigned): Set `rd = 1` if `rs1 < rs2` (unsigned), else `rd = 0` (Note: Added in revision 1.1)

### 4.3 Extension L (Extended Load/Store)
- `0x13 (mem)`: `LDB rd, [base + off16]` (load byte): Load the least significant byte from memory at `base + off16` into `rd`, zero-extended.
- `0x14 (mem)`: `LDH rd, [base + off16]` (load halfword): Load the least significant halfword from memory at `base + off16` into `rd`, zero-extended.
- `0x15 (mem)`: `STB [base + off16], rd` (store byte): Store the least significant byte of `rd` into memory at `base + off16`.
- `0x16 (mem)`: `STH [base + off16], rd` (store halfword): Store the least significant halfword of `rd` into memory at `base + off16`.

### 4.4 Extension M (Multiplication and Division) (Note: Added in revision 1.1)
- `0x18 (reg)`: `MUL rd, rs1, rs2`: Multiply `rs1` and `rs2`, store lower 32 bits in `rd`
- `0x19 (reg)`: `DIV rd, rs1, rs2`: Divide `rs1` by `rs2`, store quotient in `rd`. If `rs2` is zero, behavior is undefined (emulator may panic). TODO: define behavior for division by zero. (Hardware exception or trap may be implemented in future.)
- `0x1A (reg)`: `MOD rd, rs1, rs2`: Divide `rs1` by `rs2`, store remainder in `rd`. If `rs2` is zero, behavior is undefined (emulator may panic).
- `0x1B (reg)`: `MULH rd, rs1, rs2`: Multiply `rs1` and `rs2`, store upper 32 bits in `rd`
- `0x1C (reg)`: `DIVU rd, rs1, rs2`: Divide `rs1` by `rs2` (unsigned), store quotient in `rd`. If `rs2` is zero, behavior is undefined (emulator may panic).

## 5. Execution Semantics

Instruction size is always 4 bytes.

Default next PC:

- `next_pc = PC + 4`

Branch and jump targets:

- `target = next_pc + sign_extend(off16)`

Rules:

- `JAL`: `R31 <- next_pc`, then `PC <- target`
- `JR`: `PC <- R[rd]`
- `BEQ`: branch if `R[rs1] == R[rs2]`
- `BNE`: branch if `R[rs1] != R[rs2]`
- `SLT`: signed compare
- `LD`/`ST`: 32-bit big-endian accesses
- `CPUID`: writes ID/features to fixed registers
  - `R1 <- CPU_ID`
  - `R2 <- CPU_FEATURES`
- `HALT`: stop execution loop

## 6. Memory Model (Current Emulator)

- Address space: 32-bit
- Implemented RAM: `0x00000000..0x00FFFFFF` (16 MiB) (Can be extended)
- Bus performs bounds/device checks and panics on unmapped access.

Endianness:

- Byte-addressable memory
- 32-bit loads/stores are big-endian

## 7. Assembler Specification (`asm.py`)

### 7.1 Supported syntax

- Labels:
  - `start:`
- Instructions:
  - `ADD R1, R2, R3`
  - `ADDI R1, R2, -4`
  - `LD R1, [R2 + 16]`
  - `ST R3, [R4 - 8]`
  - `BEQ R1, R2, loop`
  - `JAL func`
  - `LDIH R5, 0x1234`
  - `LDIL R5, 0x5678`
- Directives:
  - `.ORG <address>`
  - `.BYTE <value>`
  - `.WORD <value>` (16-bit)
  - `.DWORD <value>` (32-bit)
  - `.DB <v1>, <v2>, ...` (numbers or quoted strings)
  - `.STRING "text"`

### 7.2 Number literals

- Decimal: `42`
- Hex: `0x2A`
- Binary: `0b101010`
- Optional sign for decimal/hex/binary where meaningful.

### 7.3 Branch offsets

For label operands of `BEQ`, `BNE`, `JMP`, `JAL`:

- `offset = label_address - (current_pc + 4)`

## 8. Build and Run

Rust CPU demo:

```bash
cargo run
```

Python assembler:

```bash
python asm.py input.s -o output.bin
```

## 9. Status

Implemented in this repository:

- Core CPU fetch/decode/execute loop
- Base ISA + Extension ALM instructions
- 16 MiB RAM-backed bus
- Two-pass assembler with labels/directives

Planned (future):

- Exception model
- MMIO devices
- ROM loader/boot flow
- Integration tests with assembled programs
