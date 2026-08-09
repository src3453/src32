import argparse
import os
import re
import struct
from dataclasses import dataclass

INSN_SIZE = 4
SHORT_INSN_SIZE = 2

REG_ALIASES = {
    "SP": 28,
    "FP": 29,
    "GP": 30,
    "LR": 31,
}
REGS = {f"R{i}": i for i in range(32)} | REG_ALIASES

OP_INFO = {
    "NOP": (0x00, "R0"),
    "LD": (0x01, "M"),
    "ST": (0x02, "M"),
    "ADD": (0x03, "R"),
    "ADDI": (0x04, "I"),
    "SUB": (0x05, "R"),
    "SLT": (0x06, "R"),
    "BEQ": (0x07, "B"),
    "BNE": (0x08, "B"),
    "JMP": (0x09, "J"),
    "JAL": (0x0A, "J"),
    "JR": (0x0B, "R1"),
    "AND": (0x0C, "R"),
    "OR": (0x0D, "R"),
    "XOR": (0x0E, "R"),
    "SLL": (0x0F, "R"),
    "SRL": (0x10, "R"),
    "SLA": (0x11, "R"),
    "SRA": (0x12, "R"),
    "SLTU": (0x17, "R"),
    "MUL": (0x18, "R"),
    "DIV": (0x19, "R"),
    "MOD": (0x1A, "R"),
    "MULH": (0x1B, "R"),
    "DIVU": (0x1C, "R"),
    "JMPS": (0x1D, "J"),
    "JALS": (0x1E, "J"),
    "JRS": (0x1F, "R1"),
    "LDB": (0x13, "M"),
    "LDH": (0x14, "M"),
    "STB": (0x15, "M"),
    "STH": (0x16, "M"),
    "LDIL": (0x3C, "I2"),
    "LDIH": (0x3D, "I2"),
    "CPUID": (0x3E, "R0"),
    "HALT": (0x3F, "R0"),
    "S.MOV": (0x0, "SR2"),
    "S.ADD": (0x1, "SR3"),
    "S.ADDI": (0x2, "SI8S"),
    "S.LD": (0x3, "SR2"),
    "S.ST": (0x4, "SR2"),
    "S.BZ": (0x5, "SB8"),
    "S.BNZ": (0x6, "SB8"),
    "S.JR": (0x7, "SR1"),
    "S.JAL": (0x8, "SJ12"),
    "S.LDI": (0x9, "SI8U"),
    "S.RET": (0xF, "S0"),
}


def strip_comment(line: str) -> str:
    return line.split(";", 1)[0].split("#", 1)[0].strip()


def tokenize(line: str) -> list[str]:
    # Keep [..] and ".." as single tokens.
    return re.findall(r'\[.*?\]|".*?"|[^,\s]+', line)


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


def parse_reg(token: str, lineno: int) -> int:
    key = token.upper()
    if key not in REGS:
        raise ValueError(f"line {lineno}: invalid register '{token}'")
    return REGS[key]


def parse_short_reg(token: str, lineno: int) -> int:
    reg = parse_reg(token, lineno)
    if reg <= 14:
        return reg
    if reg == 31:
        return 15
    raise ValueError(f"line {lineno}: short mode register must be R0..R14 or R31/LR, got '{token}'")


def parse_mem(token: str, lineno: int) -> tuple[str, str]:
    # Format: [R1], [R1+8], [R1-8], [R1 + label]
    m = re.fullmatch(r"\[(.+)\]", token.strip())
    if not m:
        raise ValueError(f"line {lineno}: invalid memory operand '{token}'")
    expr = m.group(1).strip()

    # Split base and optional +/- offset.
    mm = re.fullmatch(r"([A-Za-z0-9_]+)\s*([+-])?\s*(.*)", expr)
    if not mm:
        raise ValueError(f"line {lineno}: invalid memory operand '{token}'")

    base = mm.group(1)
    sign = mm.group(2)
    offs = mm.group(3).strip()

    if sign is None or offs == "":
        return base, "0"

    if sign == "-":
        return base, f"-{offs}"
    return base, offs


def check_i16(value: int, lineno: int, what: str = "immediate") -> int:
    if not -32768 <= value <= 32767:
        raise ValueError(f"line {lineno}: {what} out of i16 range: {value}")
    return value


def check_u32(value: int, lineno: int, what: str = "immediate") -> int:
    if not -0x8000_0000 <= value <= 0xFFFF_FFFF:
        raise ValueError(f"line {lineno}: {what} out of signed/u32 range: {value}")
    return value & 0xFFFF_FFFF


def check_i8(value: int, lineno: int, what: str = "immediate") -> int:
    if not -128 <= value <= 127:
        raise ValueError(f"line {lineno}: {what} out of i8 range: {value}")
    return value


def check_u8(value: int, lineno: int, what: str = "immediate") -> int:
    if not 0 <= value <= 0xFF:
        raise ValueError(f"line {lineno}: {what} out of u8 range: {value}")
    return value


def check_i12(value: int, lineno: int, what: str = "immediate") -> int:
    if not -2048 <= value <= 2047:
        raise ValueError(f"line {lineno}: {what} out of i12 range: {value}")
    return value


def be32(raw: int) -> bytes:
    return bytes([
        (raw >> 24) & 0xFF,
        (raw >> 16) & 0xFF,
        (raw >> 8) & 0xFF,
        raw & 0xFF,
    ])


def be16(raw: int) -> bytes:
    return bytes([
        (raw >> 8) & 0xFF,
        raw & 0xFF,
    ])


def enc_r(op: int, rd: int, rs1: int, rs2: int) -> bytes:
    raw = (op << 26) | (rd << 21) | (rs1 << 16) | (rs2 << 11)
    return be32(raw)


def enc_i(op: int, rd: int, rs1: int, imm16: int) -> bytes:
    raw = (op << 26) | (rd << 21) | (rs1 << 16) | (imm16 & 0xFFFF)
    return be32(raw)


def enc_m(op: int, rd: int, base: int, off16: int) -> bytes:
    raw = (op << 26) | (rd << 21) | (base << 16) | (off16 & 0xFFFF)
    return be32(raw)


def enc_imm32(op: int, rd: int, imm32: int) -> bytes:
    if op == 0x3D:
        imm16 = (imm32 >> 16) & 0xFFFF
    elif op == 0x3C:
        imm16 = imm32 & 0xFFFF
    else:
        raise ValueError(f"unsupported imm32 opcode: 0x{op:02X}")
    raw = (op << 26) | (rd << 21) | imm16
    return be32(raw)


def enc_s_r2(op: int, rd: int, rs1: int) -> bytes:
    raw = ((op & 0x0F) << 12) | ((rd & 0x0F) << 8) | ((rs1 & 0x0F) << 4)
    return be16(raw)


def enc_s_r3(op: int, rd: int, rs1: int, rs2: int) -> bytes:
    raw = ((op & 0x0F) << 12) | ((rd & 0x0F) << 8) | ((rs1 & 0x0F) << 4) | (rs2 & 0x0F)
    return be16(raw)


def enc_s_i8(op: int, rd: int, imm8: int) -> bytes:
    raw = ((op & 0x0F) << 12) | ((rd & 0x0F) << 8) | (imm8 & 0xFF)
    return be16(raw)


def enc_s_i12(op: int, imm12: int) -> bytes:
    raw = ((op & 0x0F) << 12) | (imm12 & 0x0FFF)
    return be16(raw)


@dataclass
class SourceLine:
    lineno: int
    text: str
    source: str = "<input>"


class Assembler:
    def __init__(self) -> None:
        self.symbols: dict[str, int] = {}
        self.lines: list[SourceLine] = []
        self.output = bytearray()
        self.pc = 0

    def load(self, text: str) -> None:
        self.lines = [SourceLine(i + 1, line) for i, line in enumerate(text.splitlines())]

    def _read_text_file(self, path: str) -> str:
        with open(path, "r", encoding="utf-8") as f:
            return f.read()

    def _preprocess_text(self, text: str, source: str, base_dir: str) -> list[SourceLine]:
        macros: dict[str, tuple[list[str], list[str]]] = {}
        consts: dict[str, str] = {}
        include_stack: list[str] = []
        macro_stack: list[str] = []

        def apply_consts(line: str) -> str:
            out = line
            for name, value in consts.items():
                out = re.sub(rf"\b{re.escape(name)}\b", value, out)
            return out

        def preprocess_source(src_text: str, src_name: str, src_dir: str) -> list[SourceLine]:
            out: list[SourceLine] = []
            raw_lines = src_text.splitlines()
            i = 0

            while i < len(raw_lines):
                lineno = i + 1
                raw = raw_lines[i]
                stripped = strip_comment(raw)
                i += 1

                if not stripped:
                    continue

                tokens = tokenize(stripped)
                if not tokens:
                    continue

                op = tokens[0].upper()

                if op == ".INCLUDE":
                    if len(tokens) != 2:
                        raise ValueError(f"line {lineno}: .INCLUDE expects 1 operand")

                    path_token = tokens[1]
                    if path_token.startswith('"') and path_token.endswith('"'):
                        rel_path = path_token[1:-1]
                    else:
                        rel_path = path_token

                    include_path = rel_path
                    if not os.path.isabs(include_path):
                        include_path = os.path.join(src_dir, rel_path)
                    include_path = os.path.abspath(include_path)

                    if include_path in include_stack:
                        raise ValueError(
                            f"line {lineno}: cyclic .INCLUDE detected for '{include_path}'"
                        )

                    include_stack.append(include_path)
                    child_text = self._read_text_file(include_path)
                    child_dir = os.path.dirname(include_path)
                    out.extend(preprocess_source(child_text, include_path, child_dir))
                    include_stack.pop()
                    continue

                if op == ".DEFINE":
                    if len(tokens) < 2:
                        raise ValueError(f"line {lineno}: .DEFINE expects a macro name")
                    name = tokens[1]
                    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
                        raise ValueError(f"line {lineno}: invalid macro name '{name}'")
                    if name in macros:
                        raise ValueError(f"line {lineno}: duplicate macro '{name}'")

                    params = tokens[2:]
                    seen_params: set[str] = set()
                    for param in params:
                        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", param):
                            raise ValueError(
                                f"line {lineno}: invalid macro parameter '{param}'"
                            )
                        if param in seen_params:
                            raise ValueError(
                                f"line {lineno}: duplicate macro parameter '{param}'"
                            )
                        seen_params.add(param)

                    body: list[str] = []
                    found_end = False
                    while i < len(raw_lines):
                        body_lineno = i + 1
                        body_raw = raw_lines[i]
                        body_stripped = strip_comment(body_raw)
                        i += 1

                        body_tokens = tokenize(body_stripped) if body_stripped else []
                        if body_tokens and body_tokens[0].upper() == ".ENDDEF":
                            if len(body_tokens) != 1:
                                raise ValueError(
                                    f"line {body_lineno}: .ENDDEF takes no operands"
                                )
                            found_end = True
                            break

                        body.append(body_raw)

                    if not found_end:
                        raise ValueError(
                            f"line {lineno}: missing .ENDDEF for macro '{name}'"
                        )

                    macros[name] = (params, body)
                    continue

                if op == ".CONST":
                    mconst = re.fullmatch(
                        r"\.CONST\s+([A-Za-z_][A-Za-z0-9_]*)\s+(.+)",
                        stripped,
                    )
                    if not mconst:
                        raise ValueError(
                            f"line {lineno}: .CONST expects '<name> <replacement>'"
                        )

                    name = mconst.group(1)
                    replacement = mconst.group(2).strip()
                    if name in consts:
                        raise ValueError(f"line {lineno}: duplicate constant '{name}'")
                    consts[name] = replacement
                    continue

                replaced = apply_consts(stripped)
                tokens = tokenize(replaced)
                if not tokens:
                    continue

                first = tokens[0]
                if first in macros:
                    params, body = macros[first]
                    args = tokens[1:]
                    if len(args) != len(params):
                        raise ValueError(
                            f"line {lineno}: macro '{first}' expects {len(params)} operands"
                        )

                    if first in macro_stack:
                        raise ValueError(
                            f"line {lineno}: recursive macro expansion detected for '{first}'"
                        )

                    repl = dict(zip(params, args))
                    macro_stack.append(first)
                    for macro_line in body:
                        expanded_line = macro_line
                        for param, arg in repl.items():
                            expanded_line = re.sub(
                                rf"\b{re.escape(param)}\b",
                                arg,
                                expanded_line,
                            )
                        out.extend(
                            preprocess_source(
                                expanded_line,
                                src_name,
                                src_dir,
                            )
                        )
                    macro_stack.pop()
                    continue

                if op == ".ENDDEF":
                    raise ValueError(f"line {lineno}: stray .ENDDEF")

                out.append(SourceLine(lineno, replaced, src_name))

            return out

        return preprocess_source(text, source, base_dir)

    def parse_imm_or_label(self, token: str, lineno: int) -> int:
        if token in self.symbols:
            return self.symbols[token]
        try:
            return parse_number(token)
        except Exception as exc:
            raise ValueError(f"line {lineno}: undefined symbol '{token}'") from exc

    def parse_jump_target(self, token: str, lineno: int) -> tuple[int, bool]:
        # Jump targets may be labels, absolute numeric addresses, or relative
        # numeric offsets prefixed with R!.
        if token.startswith("R!"):
            try:
                return parse_number(token[2:]), True
            except Exception as exc:
                raise ValueError(f"line {lineno}: invalid relative jump target '{token}'") from exc
        return self.parse_imm_or_label(token, lineno), False

    def set_pc(self, new_pc: int, lineno: int) -> None:
        if new_pc < 0:
            raise ValueError(f"line {lineno}: negative .ORG value")
        if new_pc < len(self.output):
            raise ValueError(f"line {lineno}: .ORG overlaps existing output")
        while len(self.output) < new_pc:
            self.output.append(0)
        self.pc = new_pc

    def _split_label(self, line: str) -> tuple[str | None, str]:
        if ":" not in line:
            return None, line
        label, rest = line.split(":", 1)
        label = label.strip()
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", label):
            raise ValueError(f"invalid label '{label}'")
        return label, rest.strip()

    def pass1(self) -> None:
        self.pc = 0
        for entry in self.lines:
            line = strip_comment(entry.text)
            if not line:
                continue

            label, rest = self._split_label(line)
            if label is not None:
                if label in self.symbols:
                    raise ValueError(f"line {entry.lineno}: duplicate label '{label}'")
                self.symbols[label] = self.pc
                line = rest
                if not line:
                    continue

            tokens = tokenize(line)
            if not tokens:
                continue

            op = tokens[0].upper()
            if op == ".ORG":
                if len(tokens) != 2:
                    raise ValueError(f"line {entry.lineno}: .ORG expects 1 operand")
                self.pc = parse_number(tokens[1])
                continue
            if op == ".BYTE":
                self.pc += 1
                continue
            if op == ".WORD":
                self.pc += 2
                continue
            if op == ".DWORD":
                self.pc += 4
                continue
            if op == ".DB":
                for item in tokens[1:]:
                    if item.startswith('"') and item.endswith('"'):
                        self.pc += len(item[1:-1].encode())
                    else:
                        self.pc += 1
                continue
            if op == ".STRING":
                text_part = line.split(".STRING", 1)[1].strip()
                if not (text_part.startswith('"') and text_part.endswith('"')):
                    raise ValueError(f"line {entry.lineno}: .STRING expects a quoted string")
                self.pc += len(text_part[1:-1].encode())
                continue

            if op not in OP_INFO:
                raise ValueError(f"line {entry.lineno}: unknown instruction '{op}'")
            _, kind = OP_INFO[op]
            if kind in {"SR2", "SR3", "SI8S", "SI8U", "SB8", "SJ12", "SR1", "S0"}:
                self.pc += SHORT_INSN_SIZE
            else:
                self.pc += INSN_SIZE

    def emit8(self, value: int) -> None:
        self.output.append(value & 0xFF)
        self.pc += 1

    def emit16(self, value: int) -> None:
        self.output += struct.pack(">H", value & 0xFFFF)
        self.pc += 2

    def emit32(self, value: int) -> None:
        self.output += struct.pack(">I", value & 0xFFFF_FFFF)
        self.pc += 4

    def emit_insn(self, data: bytes) -> None:
        if len(data) not in {SHORT_INSN_SIZE, INSN_SIZE}:
            raise ValueError("internal error: instruction must be 2 or 4 bytes")
        self.output += data
        self.pc += len(data)

    def branch_offset(self, target_token: str, lineno: int, insn_size: int) -> int:
        target, is_relative = self.parse_jump_target(target_token, lineno)
        if is_relative:
            return target
        return target - (self.pc + insn_size)

    def pass2(self) -> None:
        self.pc = 0
        self.output = bytearray()

        for entry in self.lines:
            line = strip_comment(entry.text)
            if not line:
                continue

            _, rest = self._split_label(line)
            line = rest if rest or ":" in line else line
            if not line:
                continue

            tokens = tokenize(line)
            if not tokens:
                continue

            op = tokens[0].upper()

            if op == ".ORG":
                self.set_pc(parse_number(tokens[1]), entry.lineno)
                continue
            if op == ".BYTE":
                self.emit8(self.parse_imm_or_label(tokens[1], entry.lineno))
                continue
            if op == ".WORD":
                self.emit16(self.parse_imm_or_label(tokens[1], entry.lineno))
                continue
            if op == ".DWORD":
                self.emit32(self.parse_imm_or_label(tokens[1], entry.lineno))
                continue
            if op == ".DB":
                for item in tokens[1:]:
                    if item.startswith('"') and item.endswith('"'):
                        for b in item[1:-1].encode():
                            self.emit8(b)
                    else:
                        self.emit8(self.parse_imm_or_label(item, entry.lineno))
                continue
            if op == ".STRING":
                text_part = line.split(".STRING", 1)[1].strip()
                text = text_part[1:-1]
                for b in text.encode():
                    self.emit8(b)
                continue

            opcode, kind = OP_INFO[op]

            if kind == "R0":
                self.emit_insn(enc_r(opcode, 0, 0, 0))
                continue

            if kind == "R1":
                if len(tokens) != 2:
                    raise ValueError(f"line {entry.lineno}: {op} expects 1 operand")
                rd = parse_reg(tokens[1], entry.lineno)
                self.emit_insn(enc_r(opcode, rd, 0, 0))
                continue

            if kind == "R":
                if len(tokens) != 4:
                    raise ValueError(f"line {entry.lineno}: {op} expects 3 operands")
                rd = parse_reg(tokens[1], entry.lineno)
                rs1 = parse_reg(tokens[2], entry.lineno)
                rs2 = parse_reg(tokens[3], entry.lineno)
                self.emit_insn(enc_r(opcode, rd, rs1, rs2))
                continue

            if kind == "I2":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: {op} expects 2 operands")
                rd = parse_reg(tokens[1], entry.lineno)
                imm = self.parse_imm_or_label(tokens[2], entry.lineno)
                imm = check_u32(imm, entry.lineno)
                self.emit_insn(enc_imm32(opcode, rd, imm))
                continue

            if kind == "I":
                if len(tokens) != 4:
                    raise ValueError(f"line {entry.lineno}: {op} expects 3 operands")
                rd = parse_reg(tokens[1], entry.lineno)
                rs1 = parse_reg(tokens[2], entry.lineno)
                imm = self.parse_imm_or_label(tokens[3], entry.lineno)
                imm = check_i16(imm, entry.lineno)
                self.emit_insn(enc_i(opcode, rd, rs1, imm))
                continue

            if kind == "M":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: {op} expects 2 operands")
                if op in {"ST", "STB", "STH"} and tokens[1].startswith("["):
                    mem_token = tokens[1]
                    rd_token = tokens[2]
                elif op in {"ST", "STB", "STH"}:
                    rd_token = tokens[1]
                    mem_token = tokens[2]
                else:
                    rd_token = tokens[1]
                    mem_token = tokens[2]

                rd = parse_reg(rd_token, entry.lineno)
                base_token, off_token = parse_mem(mem_token, entry.lineno)
                base = parse_reg(base_token, entry.lineno)
                off = self.parse_imm_or_label(off_token, entry.lineno)
                off = check_i16(off, entry.lineno, "offset")
                self.emit_insn(enc_m(opcode, rd, base, off))
                continue

            if kind == "B":
                if len(tokens) != 4:
                    raise ValueError(f"line {entry.lineno}: {op} expects 3 operands")
                rs1 = parse_reg(tokens[1], entry.lineno)
                rs2 = parse_reg(tokens[2], entry.lineno)
                off = self.branch_offset(tokens[3], entry.lineno, INSN_SIZE)
                off = check_i16(off, entry.lineno, "branch offset")
                # Immediate mode reuses rd field as rs2.
                self.emit_insn(enc_i(opcode, rs2, rs1, off))
                continue

            if kind == "J":
                if len(tokens) != 2:
                    raise ValueError(f"line {entry.lineno}: {op} expects 1 operand")
                off = self.branch_offset(tokens[1], entry.lineno, INSN_SIZE)
                off = check_i16(off, entry.lineno, "jump offset")
                self.emit_insn(enc_i(opcode, 0, 0, off))
                continue

            if kind == "SR2":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: {op} expects 2 operands")
                rd = parse_short_reg(tokens[1], entry.lineno)
                rs1 = parse_short_reg(tokens[2], entry.lineno)
                self.emit_insn(enc_s_r2(opcode, rd, rs1))
                continue

            if kind == "SR3":
                if len(tokens) != 4:
                    raise ValueError(f"line {entry.lineno}: {op} expects 3 operands")
                rd = parse_short_reg(tokens[1], entry.lineno)
                rs1 = parse_short_reg(tokens[2], entry.lineno)
                rs2 = parse_short_reg(tokens[3], entry.lineno)
                self.emit_insn(enc_s_r3(opcode, rd, rs1, rs2))
                continue

            if kind == "SR1":
                if len(tokens) != 2:
                    raise ValueError(f"line {entry.lineno}: {op} expects 1 operand")
                rd = parse_short_reg(tokens[1], entry.lineno)
                self.emit_insn(enc_s_r2(opcode, rd, 0))
                continue

            if kind == "SI8S":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: {op} expects 2 operands")
                rd = parse_short_reg(tokens[1], entry.lineno)
                imm = self.parse_imm_or_label(tokens[2], entry.lineno)
                imm = check_i8(imm, entry.lineno)
                self.emit_insn(enc_s_i8(opcode, rd, imm))
                continue

            if kind == "SI8U":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: {op} expects 2 operands")
                rd = parse_short_reg(tokens[1], entry.lineno)
                imm = self.parse_imm_or_label(tokens[2], entry.lineno)
                imm = check_u8(imm, entry.lineno)
                self.emit_insn(enc_s_i8(opcode, rd, imm))
                continue

            if kind == "SB8":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: {op} expects 2 operands")
                rd = parse_short_reg(tokens[1], entry.lineno)
                off = self.branch_offset(tokens[2], entry.lineno, SHORT_INSN_SIZE)
                off = check_i8(off, entry.lineno, "branch offset")
                self.emit_insn(enc_s_i8(opcode, rd, off))
                continue

            if kind == "SJ12":
                if len(tokens) != 2:
                    raise ValueError(f"line {entry.lineno}: {op} expects 1 operand")
                off = self.branch_offset(tokens[1], entry.lineno, SHORT_INSN_SIZE)
                off = check_i12(off, entry.lineno, "jump offset")
                self.emit_insn(enc_s_i12(opcode, off))
                continue

            if kind == "S0":
                if len(tokens) != 1:
                    raise ValueError(f"line {entry.lineno}: {op} expects no operands")
                self.emit_insn(enc_s_i8(opcode, 0, 0))
                continue

            raise ValueError(f"line {entry.lineno}: unsupported instruction form for {op}")

    def assemble(self, text: str, source: str = "<input>", base_dir: str | None = None) -> bytes:
        self.symbols.clear()
        root_dir = base_dir or os.getcwd()
        self.lines = self._preprocess_text(text, source, root_dir)
        self.pass1()
        self.pass2()
        return bytes(self.output)


def main() -> None:
    parser = argparse.ArgumentParser(description="SRC32 assembler")
    parser.add_argument("input", help="input assembly file")
    parser.add_argument("-o", "--output", help="output binary path")
    args = parser.parse_args()

    with open(args.input, "r", encoding="utf-8") as f:
        text = f.read()

    assembler = Assembler()
    binary = assembler.assemble(
        text,
        source=os.path.abspath(args.input),
        base_dir=os.path.dirname(os.path.abspath(args.input)),
    )

    out = args.output or (args.input + ".bin")
    with open(out, "wb") as f:
        f.write(binary)

    print(f"Assembled {len(binary)} bytes -> {out}")


if __name__ == "__main__":
    main()
