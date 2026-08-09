import argparse
from dataclasses import dataclass

INSN_SIZE = 4
SHORT_INSN_SIZE = 2

MODE_NORMAL = "normal"
MODE_SHORT = "short"


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


def sign_i12(value: int) -> int:
    value &= 0x0FFF
    return value - 0x1000 if value & 0x0800 else value


def sign_i8(value: int) -> int:
    value &= 0xFF
    return value - 0x100 if value & 0x80 else value


def reg_name(index: int) -> str:
    return f"R{index}"


def short_reg_name(index: int) -> str:
    if index == 15:
        return "R31"
    return f"R{index}"


def mem_operand(base: int, off: int) -> str:
    if off == 0:
        return f"[{reg_name(base)}]"
    if off < 0:
        return f"[{reg_name(base)} - {-off}]"
    return f"[{reg_name(base)} + {off}]"


def bytes_to_u32(chunk: bytes) -> int:
    if len(chunk) != INSN_SIZE:
        raise ValueError("internal error: instruction chunk must be 4 bytes")
    return (
        (chunk[0] << 24)
        | (chunk[1] << 16)
        | (chunk[2] << 8)
        | chunk[3]
    )


def bytes_to_u16(chunk: bytes) -> int:
    if len(chunk) != SHORT_INSN_SIZE:
        raise ValueError("internal error: instruction chunk must be 2 bytes")
    return (chunk[0] << 8) | chunk[1]


@dataclass
class DecodedInsn:
    pc: int
    raw: int
    asm: str
    size: int
    mode: str
    next_mode: str
    target: int | None = None


def decode_normal(pc: int, raw: int) -> DecodedInsn:
    op = (raw >> 26) & 0x3F
    rd = (raw >> 21) & 0x1F
    rs1 = (raw >> 16) & 0x1F
    rs2 = (raw >> 11) & 0x1F
    imm16 = sign_i16(raw & 0xFFFF)
    next_pc = (pc + INSN_SIZE) & 0xFFFF_FFFF
    target = (next_pc + imm16) & 0xFFFF_FFFF

    base = DecodedInsn(pc=pc, raw=raw, asm="", size=INSN_SIZE, mode=MODE_NORMAL, next_mode=MODE_NORMAL)

    if op == 0x00:
        base.asm = "NOP"
        return base
    if op == 0x01:
        base.asm = f"LD {reg_name(rd)}, {mem_operand(rs1, imm16)}"
        return base
    if op == 0x02:
        base.asm = f"ST {mem_operand(rs1, imm16)}, {reg_name(rd)}"
        return base
    if op == 0x03:
        base.asm = f"ADD {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x04:
        base.asm = f"ADDI {reg_name(rd)}, {reg_name(rs1)}, {imm16}"
        return base
    if op == 0x05:
        base.asm = f"SUB {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x06:
        base.asm = f"SLT {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x07:
        base.asm = f"BEQ {reg_name(rs1)}, {reg_name(rd)}, __TARGET__"
        base.target = target
        return base
    if op == 0x08:
        base.asm = f"BNE {reg_name(rs1)}, {reg_name(rd)}, __TARGET__"
        base.target = target
        return base
    if op == 0x09:
        base.asm = "JMP __TARGET__"
        base.target = target
        return base
    if op == 0x0A:
        base.asm = "JAL __TARGET__"
        base.target = target
        return base
    if op == 0x0B:
        base.asm = f"JR {reg_name(rd)}"
        return base
    if op == 0x0C:
        base.asm = f"AND {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x0D:
        base.asm = f"OR {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x0E:
        base.asm = f"XOR {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x0F:
        base.asm = f"SLL {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x10:
        base.asm = f"SRL {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x11:
        base.asm = f"SLA {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x12:
        base.asm = f"SRA {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x13:
        base.asm = f"LDB {reg_name(rd)}, {mem_operand(rs1, imm16)}"
        return base
    if op == 0x14:
        base.asm = f"LDH {reg_name(rd)}, {mem_operand(rs1, imm16)}"
        return base
    if op == 0x15:
        base.asm = f"STB {mem_operand(rs1, imm16)}, {reg_name(rd)}"
        return base
    if op == 0x16:
        base.asm = f"STH {mem_operand(rs1, imm16)}, {reg_name(rd)}"
        return base
    if op == 0x17:
        base.asm = f"SLTU {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x18:
        base.asm = f"MUL {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x19:
        base.asm = f"DIV {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x1A:
        base.asm = f"MOD {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x1B:
        base.asm = f"MULH {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x1C:
        base.asm = f"DIVU {reg_name(rd)}, {reg_name(rs1)}, {reg_name(rs2)}"
        return base
    if op == 0x1D:
        base.asm = "JMPS __TARGET__"
        base.target = target
        base.next_mode = MODE_SHORT
        return base
    if op == 0x1E:
        base.asm = "JALS __TARGET__"
        base.target = target
        base.next_mode = MODE_SHORT
        return base
    if op == 0x1F:
        base.asm = f"JRS {reg_name(rd)}"
        base.next_mode = MODE_SHORT
        return base
    if op == 0x3C:
        base.asm = f"LDIL {reg_name(rd)}, 0x{raw & 0xFFFF:04X}"
        return base
    if op == 0x3D:
        base.asm = f"LDIH {reg_name(rd)}, 0x{raw & 0xFFFF:04X}"
        return base
    if op == 0x3E:
        base.asm = "CPUID"
        return base
    if op == 0x3F:
        base.asm = "HALT"
        return base

    db = ", ".join(f"0x{x:02X}" for x in raw.to_bytes(4, byteorder="big"))
    base.asm = f".DB {db}"
    return base


def decode_short(pc: int, raw: int) -> DecodedInsn:
    op = (raw >> 12) & 0x0F
    rd = (raw >> 8) & 0x0F
    rs1 = (raw >> 4) & 0x0F
    rs2 = raw & 0x0F
    imm8 = sign_i8(raw & 0xFF)
    imm8_u = raw & 0xFF
    imm12 = sign_i12(raw & 0x0FFF)
    next_pc = (pc + SHORT_INSN_SIZE) & 0xFFFF_FFFF

    base = DecodedInsn(pc=pc, raw=raw, asm="", size=SHORT_INSN_SIZE, mode=MODE_SHORT, next_mode=MODE_SHORT)

    if op == 0x0:
        base.asm = f"S.MOV {short_reg_name(rd)}, {short_reg_name(rs1)}"
        return base
    if op == 0x1:
        base.asm = f"S.ADD {short_reg_name(rd)}, {short_reg_name(rs1)}, {short_reg_name(rs2)}"
        return base
    if op == 0x2:
        base.asm = f"S.ADDI {short_reg_name(rd)}, {imm8}"
        return base
    if op == 0x3:
        base.asm = f"S.LD {short_reg_name(rd)}, {short_reg_name(rs1)}"
        return base
    if op == 0x4:
        base.asm = f"S.ST {short_reg_name(rd)}, {short_reg_name(rs1)}"
        return base
    if op == 0x5:
        base.target = (next_pc + imm8) & 0xFFFF_FFFF
        base.asm = f"S.BZ {short_reg_name(rd)}, __TARGET__"
        return base
    if op == 0x6:
        base.target = (next_pc + imm8) & 0xFFFF_FFFF
        base.asm = f"S.BNZ {short_reg_name(rd)}, __TARGET__"
        return base
    if op == 0x7:
        base.asm = f"S.JR {short_reg_name(rd)}"
        return base
    if op == 0x8:
        base.target = (next_pc + imm12) & 0xFFFF_FFFF
        base.asm = "S.JAL __TARGET__"
        return base
    if op == 0x9:
        base.asm = f"S.LDI {short_reg_name(rd)}, 0x{imm8_u:02X}"
        return base
    if op == 0xF:
        base.asm = "S.RET"
        base.next_mode = MODE_NORMAL
        return base

    b0 = (raw >> 8) & 0xFF
    b1 = raw & 0xFF
    base.asm = f".DB 0x{b0:02X}, 0x{b1:02X}"
    return base


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


def disassemble(data: bytes, base: int = 0, show_addr: bool = False, start_mode: str = MODE_NORMAL) -> str:
    decoded: list[DecodedInsn] = []

    mode = MODE_SHORT if start_mode == MODE_SHORT else MODE_NORMAL
    i = 0
    while i < len(data):
        pc = (base + i) & 0xFFFF_FFFF
        if mode == MODE_NORMAL:
            if i + INSN_SIZE > len(data):
                break
            chunk = data[i : i + INSN_SIZE]
            insn = decode_normal(pc, bytes_to_u32(chunk))
        else:
            if i + SHORT_INSN_SIZE > len(data):
                break
            chunk = data[i : i + SHORT_INSN_SIZE]
            insn = decode_short(pc, bytes_to_u16(chunk))

        decoded.append(insn)
        i += insn.size
        mode = insn.next_mode

    trailing = data[i:]
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
            if insn.size == 4:
                rawb = insn.raw.to_bytes(4, byteorder="big")
                bytes_text = " ".join(f"{b:02X}" for b in rawb)
                asm = f"{asm:<34} ; {insn.pc:08X} [{insn.mode}] {bytes_text}"
            else:
                rawb = insn.raw.to_bytes(2, byteorder="big")
                bytes_text = " ".join(f"{b:02X}" for b in rawb)
                asm = f"{asm:<34} ; {insn.pc:08X} [{insn.mode}] {bytes_text}"

        lines.append(asm)

    if trailing:
        if decoded:
            lines.append("")
        lines.append("; trailing bytes (not enough for a full instruction)")
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
    parser.add_argument(
        "--start-mode",
        choices=[MODE_NORMAL, MODE_SHORT],
        default=MODE_NORMAL,
        help="decode start mode (default: normal)",
    )
    args = parser.parse_args()

    base = parse_number(args.base)
    if not 0 <= base <= 0xFFFF_FFFF:
        raise ValueError(f"--base out of u32 range: {base}")

    with open(args.input, "rb") as f:
        data = f.read()

    text = f"; Disassembly of {args.input} (base address: 0x{base:08X})\n\n"
    text += disassemble(data, base=base, show_addr=args.show_addr, start_mode=args.start_mode)

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
