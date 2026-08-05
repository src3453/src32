"""sol VM (HLE) for development-time validation."""

from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Optional


LABEL_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
NUMBER_RE = re.compile(r"^-?[0-9]+u?$|^0[xX][0-9a-fA-F]+u?$")
INT32_MIN = -(2**31)
INT32_MAX = 2**31 - 1
UINT32_MAX = 2**32 - 1


class SolVMError(RuntimeError):
    pass


@dataclass(frozen=True)
class Instruction:
    op: str
    arg: Optional[int | str] = None


@dataclass(frozen=True)
class Program:
    instructions: list[Instruction]
    labels: dict[str, int]


def to_i32(value: int) -> int:
    value &= UINT32_MAX
    if value > INT32_MAX:
        return value - (2**32)
    return value


def _strip_comment(line: str) -> str:
    comment_start = line.find("#")
    if comment_start >= 0:
        return line[:comment_start]
    return line


def tokenize(source: str) -> list[str]:
    tokens: list[str] = []
    for line in source.splitlines():
        clean = _strip_comment(line).replace(";", " ")
        pieces = clean.split()
        tokens.extend(pieces)
    return tokens


def parse_number(token: str) -> int:
    is_unsigned = token.endswith("u")
    core = token[:-1] if is_unsigned else token
    if core == "":
        raise SolVMError("invalid numeric literal: empty")

    try:
        is_hex = core.startswith(("0x", "0X"))
        if is_hex:
            value = int(core, 16)
        else:
            value = int(core, 10)
    except ValueError as exc:
        raise SolVMError(f"invalid numeric literal: {token}") from exc

    if is_hex:
        if value < 0 or value > UINT32_MAX:
            raise SolVMError(f"hex literal out of range: {token}")
    elif is_unsigned:
        if value < 0 or value > UINT32_MAX:
            raise SolVMError(f"unsigned literal out of range: {token}")
    elif value < INT32_MIN or value > INT32_MAX:
        raise SolVMError(f"signed literal out of range: {token}")
    return to_i32(value)


import os

def compile_program(source: str, var_base: int = 0x00100000, source_path: str | None = None, included_paths: set | None = None) -> Program:
    # prepare include tracking and base directory
    included_paths = set() if included_paths is None else set(included_paths)
    base_dir = os.path.dirname(os.path.abspath(source_path)) if source_path else None

    def expand_includes(tokens: list[str], current_base: str | None, seen: set) -> list[str]:
        out: list[str] = []
        i = 0
        while i < len(tokens):
            tok = tokens[i]
            if tok == "!include":
                if i + 1 >= len(tokens):
                    raise SolVMError("!include requires a filename")
                fname_tok = tokens[i + 1]
                # strip optional surrounding quotes
                if fname_tok.startswith('"') and fname_tok.endswith('"') and len(fname_tok) >= 2:
                    fname = fname_tok[1:-1]
                else:
                    fname = fname_tok
                if current_base:
                    include_path = os.path.abspath(os.path.join(current_base, fname))
                else:
                    include_path = os.path.abspath(fname)
                if include_path in seen:
                    raise SolVMError(f"circular include detected: {include_path}")
                if not os.path.exists(include_path):
                    raise SolVMError(f"include file not found: {include_path}")
                seen.add(include_path)
                included_text = open(include_path, "r", encoding="utf-8").read()
                included_tokens = tokenize(included_text)
                included_dir = os.path.dirname(include_path)
                expanded = expand_includes(included_tokens, included_dir, seen)
                out.extend(expanded)
                i += 2
                continue
            out.append(tok)
            i += 1
        return out

    tokens = tokenize(source)
    tokens = expand_includes(tokens, base_dir, included_paths)

    instructions: list[Instruction] = []
    labels: dict[str, int] = {}

    # directive-managed symbols
    constants: dict[str, int] = {}
    variables: dict[str, int] = {}
    # macros: simple textual substitution of single tokens -> list of tokens
    macros: dict[str, list[str]] = {}
    # next variable address (4-byte aligned) - configurable base
    next_var_addr = var_base

    simple_ops = {
        "add",
        "sub",
        "mul",
        "div",
        "dup",
        "drop",
        "swap",
        "ld",
        "st",
        "ldb",
        "ldh",
        "stb",
        "sth",
        "eq",
        "neq",
        "lt",
        "gt",
        "le",
        "ge",
        "halt",
    }

    i = 0
    # helper to recursively expand macros for a single token
    def expand_token_recursive(tok: str, seen: set[str]) -> list[str]:
        if tok not in macros:
            return [tok]
        if tok in seen:
            raise SolVMError(f"circular macro detected: {tok}")
        seen.add(tok)
        out: list[str] = []
        for part in macros[tok]:
            if part in macros:
                out.extend(expand_token_recursive(part, seen))
            else:
                out.append(part)
        seen.remove(tok)
        return out

    while i < len(tokens):
        tok = tokens[i]

        # expand macros on-the-fly
        if tok in macros:
            expanded = expand_token_recursive(tok, set())
            # replace current token with expanded tokens
            tokens[i:i+1] = expanded
            # do not advance i; process the newly inserted tokens
            continue

        # labels
        if tok.startswith("@"):
            label_name = tok[1:]
            if not LABEL_RE.match(label_name):
                raise SolVMError(f"invalid label name: {tok}")
            if label_name in labels:
                raise SolVMError(f"duplicate label: {label_name}")
            labels[label_name] = len(instructions)
            i += 1
            continue

        # jumps with label operand
        if tok in {"jmp", "jz", "jnz"}:
            if i + 1 >= len(tokens):
                raise SolVMError(f"missing label operand for {tok}")
            label_token = tokens[i + 1]
            if not label_token.startswith("@"):
                raise SolVMError(f"label operand must start with @: {label_token}")
            label_name = label_token[1:]
            if not LABEL_RE.match(label_name):
                raise SolVMError(f"invalid label name: {label_token}")
            instructions.append(Instruction(tok, label_name))
            i += 2
            continue

        # compiler directives starting with '!'
        if tok.startswith("!"):
            if tok == "!const":
                # !const NAME VALUE
                if i + 2 >= len(tokens):
                    raise SolVMError("!const requires a name and a value")
                name = tokens[i + 1]
                if not LABEL_RE.match(name):
                    raise SolVMError(f"invalid constant name: {name}")
                val_tok = tokens[i + 2]
                if not NUMBER_RE.match(val_tok):
                    raise SolVMError(f"invalid constant value: {val_tok}")
                value = parse_number(val_tok)
                if name in constants:
                    raise SolVMError(f"duplicate constant: {name}")
                constants[name] = value
                i += 3
                continue

            if tok == "!var":
                # !var NAME [VALUE]
                if i + 1 >= len(tokens):
                    raise SolVMError("!var requires a name and optional initial value")
                name = tokens[i + 1]
                if not LABEL_RE.match(name):
                    raise SolVMError(f"invalid variable name: {name}")
                # Check for optional initial value
                init_value = 0
                consumed = 2
                if i + 2 < len(tokens) and NUMBER_RE.match(tokens[i + 2]):
                    val_tok = tokens[i + 2]
                    init_value = parse_number(val_tok)
                    consumed = 3
                if name in variables:
                    raise SolVMError(f"duplicate variable: {name}")
                addr = next_var_addr
                next_var_addr += 4
                variables[name] = addr
                # initialize memory: push init_value, push addr, st
                instructions.append(Instruction("push", init_value))
                instructions.append(Instruction("push", addr))
                instructions.append(Instruction("st"))
                i += consumed
                continue

            if tok == "!define":
                # !define NAME VALUE
                if i + 2 >= len(tokens):
                    raise SolVMError("!define requires a name and a value")
                name = tokens[i + 1]
                if not LABEL_RE.match(name):
                    raise SolVMError(f"invalid macro name: {name}")
                value_tok = tokens[i + 2]
                if name in macros:
                    raise SolVMError(f"duplicate macro: {name}")
                # store replacement as list of tokens (single-token macro for now)
                macros[name] = [value_tok]
                i += 3
                continue

            # other directives not yet supported
            raise SolVMError(f"unsupported directive: {tok}")

        # store operator: >name  (store top-of-stack into variable 'name')
        if tok.startswith(">"):
            name = tok[1:]
            if not LABEL_RE.match(name):
                raise SolVMError(f"invalid variable name for store: {name}")
            if name not in variables:
                raise SolVMError(f"undefined variable: {name}")
            addr = variables[name]
            # to store: stack has value on top; push address then call st
            instructions.append(Instruction("push", addr))
            instructions.append(Instruction("st"))
            i += 1
            continue

        # simple operations
        if tok in simple_ops:
            instructions.append(Instruction(tok))
            i += 1
            continue

        # constants
        if tok in constants:
            instructions.append(Instruction("push", constants[tok]))
            i += 1
            continue

        # variable read: name -> push addr; ld
        if LABEL_RE.match(tok) and tok in variables:
            addr = variables[tok]
            instructions.append(Instruction("push", addr))
            instructions.append(Instruction("ld"))
            i += 1
            continue

        # numbers
        if not NUMBER_RE.match(tok):
            raise SolVMError(f"unknown word: {tok}")
        value = parse_number(tok)
        instructions.append(Instruction("push", value))
        i += 1

    # validate jump targets
    for inst in instructions:
        if inst.op in {"jmp", "jz", "jnz"} and isinstance(inst.arg, str):
            if inst.arg not in labels:
                raise SolVMError(f"undefined label: {inst.arg}")

    return Program(instructions=instructions, labels=labels)


class SolVM:
    def __init__(self, *, max_steps: int = 1_000_000):
        self.max_steps = max_steps
        self.stack: list[int] = []
        self.memory: dict[int, int] = {}
        self.pc = 0
        self.halted = False
        self.program: Optional[Program] = None

    def reset(self) -> None:
        self.stack = []
        self.memory = {}
        self.pc = 0
        self.halted = False
        self.program = None

    def load(self, source: str, source_path: str | None = None) -> None:
        self.program = compile_program(source, source_path=source_path)
        self.pc = 0
        self.halted = False

    def _pop(self) -> int:
        if not self.stack:
            raise SolVMError("stack underflow")
        return self.stack.pop()

    def _read_mem(self, address: int, size: int) -> int:
        value = 0
        for shift in range(0, size * 8, 8):
            byte = self.memory.get(address + (shift // 8), 0)
            value = (value << 8) | byte
        return to_i32(value)

    def _write_mem(self, address: int, value: int, size: int) -> None:
        value &= 0xFFFFFFFF
        for offset in range(size):
            byte_shift = 8 * (size - 1 - offset)
            byte = (value >> byte_shift) & 0xFF
            self.memory[address + offset] = byte

    def run(self) -> list[int]:
        if self.program is None:
            raise SolVMError("no program loaded")
        program = self.program
        steps = 0
        while self.pc < len(program.instructions) and not self.halted:
            steps += 1
            if steps > self.max_steps:
                raise SolVMError("execution exceeded step limit")
            inst = program.instructions[self.pc]
            self.execute_instruction(inst, program.labels)
        return self.stack

    def run_source(self, source: str, source_path: str | None = None) -> list[int]:
        self.load(source, source_path=source_path)
        return self.run()

    def execute_instruction(self, inst: Instruction, labels: dict[str, int]) -> None:
        op = inst.op

        if op == "push":
            assert isinstance(inst.arg, int)
            self.stack.append(inst.arg)
            self.pc += 1
            return

        if op == "add":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a + b))
            self.pc += 1
            return

        if op == "sub":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a - b))
            self.pc += 1
            return

        if op == "mul":
            b = self._pop()
            a = self._pop()
            self.stack.append(to_i32(a * b))
            self.pc += 1
            return

        if op == "div":
            b = self._pop()
            a = self._pop()
            if b == 0:
                raise SolVMError("division by zero")
            self.stack.append(to_i32(int(a / b)))
            self.pc += 1
            return

        if op == "dup":
            if not self.stack:
                raise SolVMError("stack underflow")
            self.stack.append(self.stack[-1])
            self.pc += 1
            return

        if op == "drop":
            self._pop()
            self.pc += 1
            return

        if op == "swap":
            if len(self.stack) < 2:
                raise SolVMError("stack underflow")
            self.stack[-1], self.stack[-2] = self.stack[-2], self.stack[-1]
            self.pc += 1
            return

        if op == "ld":
            address = self._pop()
            self.stack.append(self._read_mem(address, 4))
            self.pc += 1
            return

        if op == "st":
            address = self._pop()
            value = self._pop()
            self._write_mem(address, value, 4)
            self.pc += 1
            return

        if op == "ldb":
            address = self._pop()
            self.stack.append(self._read_mem(address, 1))
            self.pc += 1
            return

        if op == "ldh":
            address = self._pop()
            self.stack.append(self._read_mem(address, 2))
            self.pc += 1
            return

        if op == "stb":
            address = self._pop()
            value = self._pop()
            self._write_mem(address, value, 1)
            self.pc += 1
            return

        if op == "sth":
            address = self._pop()
            value = self._pop()
            self._write_mem(address, value, 2)
            self.pc += 1
            return

        if op == "eq":
            b = self._pop()
            a = self._pop()
            self.stack.append(1 if a == b else 0)
            self.pc += 1
            return

        if op == "neq":
            b = self._pop()
            a = self._pop()
            self.stack.append(1 if a != b else 0)
            self.pc += 1
            return

        if op == "lt":
            b = self._pop()
            a = self._pop()
            self.stack.append(1 if a < b else 0)
            self.pc += 1
            return

        if op == "gt":
            b = self._pop()
            a = self._pop()
            self.stack.append(1 if a > b else 0)
            self.pc += 1
            return

        if op == "le":
            b = self._pop()
            a = self._pop()
            self.stack.append(1 if a <= b else 0)
            self.pc += 1
            return

        if op == "ge":
            b = self._pop()
            a = self._pop()
            self.stack.append(1 if a >= b else 0)
            self.pc += 1
            return

        if op == "jmp":
            assert isinstance(inst.arg, str)
            self.pc = labels[inst.arg]
            return

        if op == "jz":
            assert isinstance(inst.arg, str)
            cond = self._pop()
            self.pc = labels[inst.arg] if cond == 0 else self.pc + 1
            return

        if op == "jnz":
            assert isinstance(inst.arg, str)
            cond = self._pop()
            self.pc = labels[inst.arg] if cond != 0 else self.pc + 1
            return

        if op == "halt":
            self.halted = True
            self.pc += 1
            return

        raise SolVMError(f"unknown opcode: {op}")
