import argparse
import os
import re
import struct
from dataclasses import dataclass

INSN_SIZE = 5

REG_ALIASES = {
    "SP": 28,
    "FP": 29,
    "GP": 30,
    "LR": 31,
}
REGS = {f"R{i}": i for i in range(32)} | REG_ALIASES

OP_INFO = {
    "NOP": (0x00, "R"),
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
    "LDB": (0x13, "M"),
    "LDH": (0x14, "M"),
    "STB": (0x15, "M"),
    "STH": (0x16, "M"),
    "CPUID": (0xDE, "R0"),
    "HALT": (0xDF, "R0"),
    "LDI": (0xE0, "LDI"),
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
    if not 0 <= value <= 0xFFFF_FFFF:
        raise ValueError(f"line {lineno}: {what} out of u32 range: {value}")
    return value


def be40(raw: int) -> bytes:
    return bytes([
        (raw >> 32) & 0xFF,
        (raw >> 24) & 0xFF,
        (raw >> 16) & 0xFF,
        (raw >> 8) & 0xFF,
        raw & 0xFF,
    ])


def enc_r(op: int, rd: int, rs1: int, rs2: int) -> bytes:
    raw = (op << 32) | (0b00 << 30) | (rd << 25) | (rs1 << 20) | (rs2 << 15)
    return be40(raw)


def enc_i(op: int, rd: int, rs1: int, imm16: int) -> bytes:
    raw = (
        (op << 32)
        | (0b01 << 30)
        | (rd << 25)
        | (rs1 << 20)
        | ((imm16 & 0xFFFF) << 4)
    )
    return be40(raw)


def enc_m(op: int, rd: int, base: int, off16: int) -> bytes:
    raw = (
        (op << 32)
        | (0b10 << 30)
        | (rd << 25)
        | (base << 20)
        | ((off16 & 0xFFFF) << 4)
    )
    return be40(raw)


def enc_ldi(rd: int, imm32: int) -> bytes:
    op = 0xE0 | (rd & 0x1F)
    raw = (op << 32) | (imm32 & 0xFFFF_FFFF)
    return be40(raw)


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
        include_stack: list[str] = []
        macro_stack: list[str] = []

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

                out.append(SourceLine(lineno, raw, src_name))

            return out

        return preprocess_source(text, source, base_dir)

    def parse_imm_or_label(self, token: str, lineno: int) -> int:
        if token in self.symbols:
            return self.symbols[token]
        try:
            return parse_number(token)
        except Exception as exc:
            raise ValueError(f"line {lineno}: undefined symbol '{token}'") from exc

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
        if len(data) != INSN_SIZE:
            raise ValueError("internal error: instruction must be 5 bytes")
        self.output += data
        self.pc += INSN_SIZE

    def branch_offset(self, target_token: str, lineno: int) -> int:
        target = self.parse_imm_or_label(target_token, lineno)
        return target - (self.pc + INSN_SIZE)

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
                rd = parse_reg(tokens[1], entry.lineno)
                base_token, off_token = parse_mem(tokens[2], entry.lineno)
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
                off = self.branch_offset(tokens[3], entry.lineno)
                off = check_i16(off, entry.lineno, "branch offset")
                # Immediate mode reuses rd field as rs2.
                self.emit_insn(enc_i(opcode, rs2, rs1, off))
                continue

            if kind == "J":
                if len(tokens) != 2:
                    raise ValueError(f"line {entry.lineno}: {op} expects 1 operand")
                off = self.branch_offset(tokens[1], entry.lineno)
                off = check_i16(off, entry.lineno, "jump offset")
                self.emit_insn(enc_i(opcode, 0, 0, off))
                continue

            if kind == "LDI":
                if len(tokens) != 3:
                    raise ValueError(f"line {entry.lineno}: LDI expects 2 operands")
                rd = parse_reg(tokens[1], entry.lineno)
                imm = self.parse_imm_or_label(tokens[2], entry.lineno)
                imm = check_u32(imm, entry.lineno)
                self.emit_insn(enc_ldi(rd, imm))
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
