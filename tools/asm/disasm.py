import argparse
from dataclasses import dataclass

INSN_SIZE = 5


def parse_number(text: str) -> int:
    t = text.strip().replace("_", "")
    sign = 1
    if t.startswith("+"):
        t = t[1:]
    elif t.startswith("-"):
        sign = -1
        t = t[1:]

    if t.startswith("0x"):
        return sign * int(t, 16)
    if t.startswith("0b"):
        return sign * int(t, 2)
    return sign * int(t, 10)


def sign_i16(value: int) -> int:
    return value - 0x10000 if value & 0x8000 else value


def reg_name(index: int) -> str:
    return f"R{index}"


def mem_operand(base: int, off: int) -> str:
    if off == 0:
        return f"[{reg_name(base)}]"
    if off < 0:
        return f"[{reg_name(base)} - {-off}]"
    return f"[{reg_name(base)} + {off}]"


def bytes_to_u40(chunk: bytes) -> int:
    if len(chunk) != INSN_SIZE:
        raise ValueError("internal error: instruction chunk must be 5 bytes")
    return (
        (chunk[0] << 32)
        | (chunk[1] << 24)
        | (chunk[2] << 16)
        | (chunk[3] << 8)
        | chunk[4]
    )


@dataclass
class DecodedInsn:
    pc: int
    raw: int
    asm: str
    target: int | None = None


def decode_one(pc: int, raw: int) -> DecodedInsn:
    op = (raw >> 32) & 0xFF

    # LDI has a dedicated encoding and does not use mode bits.
    if 0xE0 <= op <= 0xFF:
        rd = op & 0x1F
        imm = raw & 0xFFFF_FFFF
        return DecodedInsn(pc=pc, raw=raw, asm=f"LDI {reg_name(rd)}, 0x{imm:08X}")

    mode = (raw >> 30) & 0x03
    rd = (raw >> 25) & 0x1F
    rs1 = (raw >> 20) & 0x1F
    rs2 = (raw >> 15) & 0x1F
    imm16 = sign_i16((raw >> 4) & 0xFFFF)
    next_pc = (pc + INSN_SIZE) & 0xFFFF_FFFF
    target = (next_pc + imm16) & 0xFFFF_FFFF

    if op == 0x00 and mode == 0:
        return DecodedInsn(pc=pc, raw=raw, asm="NOP")
    if op == 0x01 and mode == 2:
        return DecodedInsn(pc=pc, raw=raw, asm=f"LD {reg_name(rd)}, {mem_operand(rs1, imm16)}")
    if op == 0x02 and mode == 2:
        return DecodedInsn(pc=pc, raw=raw, asm=f"ST {mem_operand(rs1, imm16)}, {reg_name(rd)}")
    if op == 0x03 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"ADD {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x04 and mode == 1:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"ADDI {reg_name(rd)}, {reg_name(rs1)}, {imm16}",
        )
    if op == 0x05 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SUB {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x06 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SLT {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x07 and mode == 1:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"BEQ {reg_name(rs1)}, {reg_name(rd)}, __TARGET__",
            target=target,
        )
    if op == 0x08 and mode == 1:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"BNE {reg_name(rs1)}, {reg_name(rd)}, __TARGET__",
            target=target,
        )
    if op == 0x09 and mode == 1:
        return DecodedInsn(pc=pc, raw=raw, asm="JMP __TARGET__", target=target)
    if op == 0x0A and mode == 1:
        return DecodedInsn(pc=pc, raw=raw, asm="JAL __TARGET__", target=target)
    if op == 0x0B and mode == 0:
        return DecodedInsn(pc=pc, raw=raw, asm=f"JR {reg_name(rd)}")
    if op == 0x0C and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"AND {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x0D and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"OR {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x0E and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"XOR {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x0F and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SLL {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x10 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SRL {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x11 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SLA {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x12 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SRA {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x17 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"SLTU {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x18 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"MUL {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x19 and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"DIV {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x1A and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"MOD {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x1B and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"MULH {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x1C and mode == 0:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"DIVU {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}",
        )
    if op == 0x13 and mode == 2:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"LDB {reg_name(rd)}, {mem_operand(rs1, imm16)}",
        )
    if op == 0x14 and mode == 2:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"LDH {reg_name(rd)}, {mem_operand(rs1, imm16)}",
        )
    if op == 0x15 and mode == 2:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"STB {mem_operand(rs1, imm16)}, {reg_name(rd)}",
        )
    if op == 0x16 and mode == 2:
        return DecodedInsn(
            pc=pc,
            raw=raw,
            asm=f"STH {mem_operand(rs1, imm16)}, {reg_name(rd)}",
        )
    if op == 0xDE and mode == 0:
        return DecodedInsn(pc=pc, raw=raw, asm="CPUID")
    if op == 0xDF and mode == 0:
        return DecodedInsn(pc=pc, raw=raw, asm="HALT")

    b = [
        (raw >> 32) & 0xFF,
        (raw >> 24) & 0xFF,
        (raw >> 16) & 0xFF,
        (raw >> 8) & 0xFF,
        raw & 0xFF,
    ]
    db = ", ".join(f"0x{x:02X}" for x in b)
    return DecodedInsn(pc=pc, raw=raw, asm=f".DB {db}")


def build_labels(decoded: list[DecodedInsn]) -> dict[int, str]:
    pcs = {insn.pc for insn in decoded}
    targets = sorted(
        {
            insn.target
            for insn in decoded
            if insn.target is not None and insn.target in pcs
        }
    )
    return {addr: f"L_{addr:08X}" for addr in targets}


def disassemble(data: bytes, base: int = 0, show_addr: bool = False) -> str:
    decoded: list[DecodedInsn] = []

    full_size = (len(data) // INSN_SIZE) * INSN_SIZE
    trailing = data[full_size:]

    for i in range(0, full_size, INSN_SIZE):
        chunk = data[i : i + INSN_SIZE]
        pc = (base + i) & 0xFFFF_FFFF
        decoded.append(decode_one(pc, bytes_to_u40(chunk)))

    labels = build_labels(decoded)

    lines: list[str] = []
    if base != 0:
        lines.append(f".ORG 0x{base:08X}")
        lines.append("")

    for insn in decoded:
        if insn.pc in labels:
            lines.append(f"{labels[insn.pc]}:")

        asm = insn.asm
        if insn.target is not None:
            target_text = labels.get(insn.target, f"0x{insn.target:08X}")
            asm = asm.replace("__TARGET__", target_text)

        if show_addr:
            b0 = (insn.raw >> 32) & 0xFF
            b1 = (insn.raw >> 24) & 0xFF
            b2 = (insn.raw >> 16) & 0xFF
            b3 = (insn.raw >> 8) & 0xFF
            b4 = insn.raw & 0xFF
            asm = f"{asm:<34} ; {insn.pc:08X}: {b0:02X} {b1:02X} {b2:02X} {b3:02X} {b4:02X}"

        lines.append(asm)

    if trailing:
        if decoded:
            lines.append("")
        lines.append("; trailing bytes (not enough for a full 5-byte instruction)")
        db = ", ".join(f"0x{x:02X}" for x in trailing)
        lines.append(f".DB {db}")

    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description="SRC32 disassembler")
    parser.add_argument("input", help="input binary file")
    parser.add_argument("-o", "--output", help="output assembly file path")
    parser.add_argument(
        "--base",
        default="0",
        help="base address for the first instruction (default: 0)",
    )
    parser.add_argument(
        "--show-addr",
        action="store_true",
        help="append PC and original bytes as comments",
    )
    args = parser.parse_args()

    base = parse_number(args.base)
    if not 0 <= base <= 0xFFFF_FFFF:
        raise ValueError(f"--base out of u32 range: {base}")

    with open(args.input, "rb") as f:
        data = f.read()

    text = f"; Disassembly of {args.input} (base address: 0x{base:08X})\n\n"
    text += disassemble(data, base=base, show_addr=args.show_addr)

    out = args.output
    if out:
        with open(out, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        print(f"Disassembled {len(data)} bytes -> {out}")
    else:
        print(text, end="")
        print()
        print(f"Disassembled {len(data)} bytes")

    


if __name__ == "__main__":
    main()