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
    functions: dict[str, int]  # function name -> arg count


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
        # keep semicolon as its own token so constructs like function bodies can be delimited
        clean = _strip_comment(line).replace(";", " ; ")
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


def compile_program(source: str, var_base: int = 0x00100000) -> Program:
    tokens = tokenize(source)
    instructions: list[Instruction] = []
    labels: dict[str, int] = {}

    # directive-managed symbols
    constants: dict[str, int] = {}
    variables: dict[str, int] = {}
    # next variable address (4-byte aligned) - configurable base
    next_var_addr = var_base

    # function definitions collected for inlining
    functions: dict[str, dict] = {}
    _local_counter = 0

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
        "ret",
        "halt",
    }

    # First pass: collect function definitions and remove them from token stream
    i = 0
    cleaned_tokens: list[str] = []
    while i < len(tokens):
        tok = tokens[i]
        if tok == "fn":
            # parse: fn name (arg1 arg2 ...) : <body tokens...> ;
            if i + 1 >= len(tokens):
                raise SolVMError("fn requires a name")
            name = tokens[i + 1]
            if not LABEL_RE.match(name):
                raise SolVMError(f"invalid function name: {name}")
            # parse args
            j = i + 2
            if j >= len(tokens) or tokens[j] != "(":
                raise SolVMError("fn requires argument list in parentheses")
            j += 1
            args: list[str] = []
            while j < len(tokens) and tokens[j] != ")":
                args.append(tokens[j])
                j += 1
            if j >= len(tokens) or tokens[j] != ")":
                raise SolVMError("unterminated function argument list")
            j += 1
            if j >= len(tokens) or tokens[j] != ":":
                raise SolVMError("fn requires ':' after argument list")
            j += 1
            # collect body until ';'
            body_tokens: list[str] = []
            while j < len(tokens) and tokens[j] != ";":
                body_tokens.append(tokens[j])
                j += 1
            if j >= len(tokens) or tokens[j] != ";":
                raise SolVMError("function body not terminated with ';'")
            # process local declarations inside body: collect locals and optional inits
            locals_list: list[tuple[str, int | None]] = []
            k = 0
            while k < len(body_tokens):
                if body_tokens[k] == "local":
                    if k + 1 >= len(body_tokens):
                        raise SolVMError("local requires a name")
                    local_name = body_tokens[k + 1]
                    if not LABEL_RE.match(local_name):
                        raise SolVMError(f"invalid local name: {local_name}")
                    # optional init value
                    init_value = None
                    consumed = 2
                    if k + 2 < len(body_tokens) and NUMBER_RE.match(body_tokens[k + 2]):
                        init_value = parse_number(body_tokens[k + 2])
                        consumed = 3
                    local_index = len(locals_list)
                    locals_list.append((local_name, init_value))
                    # replace occurrences of the local name in body with a marker
                    marker = f"__LOCAL_{local_index}"
                    for idx in range(len(body_tokens)):
                        if body_tokens[idx] == local_name:
                            body_tokens[idx] = marker
                    # remove the local declaration tokens
                    del body_tokens[k:k+consumed]
                    continue
                k += 1
            # replace occurrences of arg names with a special ARG marker so the body parser
            # can compile them into runtime 'arg' instructions
            for idx_arg, arg in enumerate(args):
                marker = f"__ARG_{idx_arg}"
                for idx in range(len(body_tokens)):
                    if body_tokens[idx] == arg:
                        body_tokens[idx] = marker
            # store function metadata (args, body tokens, locals)
            if name in functions:
                raise SolVMError(f"duplicate function: {name}")
            functions[name] = {"args": args, "body": body_tokens, "locals": locals_list}
            i = j + 1
            continue
        # not a function definition; keep token
        cleaned_tokens.append(tok)
        i += 1

    # No inlining: keep cleaned tokens as main program tokens
    tokens = cleaned_tokens

    # now parse main tokens into instructions
    i = 0
    while i < len(tokens):
        tok = tokens[i]

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

        # function call
        if tok in functions:
            instructions.append(Instruction("call", tok))
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

        # special ARG markers inside main program should be an error
        if tok.startswith("__ARG_"):
            raise SolVMError("argument marker found in main program")

        # numbers
        if not NUMBER_RE.match(tok):
            raise SolVMError(f"unknown word: {tok}")
        value = parse_number(tok)
        instructions.append(Instruction("push", value))
        i += 1

    # append function bodies after main program so they are not executed unless called
    for fname, fdata in functions.items():
        # mark label for function entry
        if fname in labels:
            raise SolVMError(f"label/function name conflict: {fname}")
        labels[fname] = len(instructions)
        body_tokens = fdata["body"]
        locals_list = fdata.get("locals", [])
        argcount = len(fdata["args"])
        n_locals = len(locals_list)
        # At function entry, optional local initializers must run (frame is allocated by caller)
        # Emit initialization sequences for locals with initial values: push init; push local_addr; st
        for local_index, (_, init_value) in enumerate(locals_list):
            if init_value is not None:
                # push init_value, push address of local, st
                instructions.append(Instruction("push", init_value))
                instructions.append(Instruction("local_addr", local_index))
                instructions.append(Instruction("st"))
        # parse body tokens into instructions
        j = 0
        while j < len(body_tokens):
            bt = body_tokens[j]
            # labels inside function are not supported in this simple implementation
            if bt in {"jmp", "jz", "jnz"}:
                if j + 1 >= len(body_tokens):
                    raise SolVMError(f"missing label operand for {bt} in function {fname}")
                label_token = body_tokens[j + 1]
                if not label_token.startswith("@"):
                    raise SolVMError(f"label operand must start with @: {label_token}")
                label_name = label_token[1:]
                if not LABEL_RE.match(label_name):
                    raise SolVMError(f"invalid label name: {label_token}")
                instructions.append(Instruction(bt, label_name))
                j += 2
                continue

            # ARG markers
            if isinstance(bt, str) and bt.startswith("__ARG_"):
                idx_str = bt.split("__ARG_")[-1]
                try:
                    idx = int(idx_str)
                except ValueError:
                    raise SolVMError(f"invalid ARG marker in function {fname}: {bt}")
                instructions.append(Instruction("arg", idx))
                j += 1
                continue

            # LOCAL markers
            if isinstance(bt, str) and bt.startswith("__LOCAL_"):
                idx_str = bt.split("__LOCAL_")[-1]
                try:
                    lidx = int(idx_str)
                except ValueError:
                    raise SolVMError(f"invalid LOCAL marker in function {fname}: {bt}")
                # push address of local var
                instructions.append(Instruction("local_addr", lidx))
                j += 1
                continue

            # simple ops and others reuse some of the above parsing rules
            if bt in simple_ops:
                instructions.append(Instruction(bt))
                j += 1
                continue

            if bt.startswith("!"):
                raise SolVMError(f"directives inside function not supported: {bt}")

            if bt.startswith(">"):
                name = bt[1:]
                if not LABEL_RE.match(name):
                    raise SolVMError(f"invalid variable name for store: {name}")
                if name not in variables:
                    raise SolVMError(f"undefined variable: {name}")
                addr = variables[name]
                instructions.append(Instruction("push", addr))
                instructions.append(Instruction("st"))
                j += 1
                continue

            if LABEL_RE.match(bt) and bt in variables:
                addr = variables[bt]
                instructions.append(Instruction("push", addr))
                instructions.append(Instruction("ld"))
                j += 1
                continue

            if not NUMBER_RE.match(bt):
                raise SolVMError(f"unknown word in function {fname}: {bt}")
            value = parse_number(bt)
            instructions.append(Instruction("push", value))
            j += 1

    # validate jump targets
    for inst in instructions:
        if inst.op in {"jmp", "jz", "jnz"} and isinstance(inst.arg, str):
            if inst.arg not in labels:
                raise SolVMError(f"undefined label: {inst.arg}")

    # build functions mapping name -> metadata for VM/emitter
    functions_map = {name: {"argcount": len(data["args"]), "n_locals": len(data.get("locals", []))} for name, data in functions.items()}

    return Program(instructions=instructions, labels=labels, functions=functions_map)


class SolVM:
    def __init__(self, *, max_steps: int = 1_000_000):
        self.max_steps = max_steps
        self.stack: list[int] = []
        self.memory: dict[int, int] = {}
        self.pc = 0
        self.halted = False
        self.program: Optional[Program] = None
        # call stack for function frames. Each frame is dict with 'ret_pc' and 'args' list
        self.call_stack: list[dict] = []

    def reset(self) -> None:
        self.stack = []
        self.memory = {}
        self.pc = 0
        self.halted = False
        self.program = None
        # runtime stack pointer (R28) initial default; matches compiler/emitter default
        self.r28 = 0x000FFFFC
        self.call_stack = []

    def load(self, source: str) -> None:
        self.program = compile_program(source)
        self.pc = 0
        self.halted = False
        # reset runtime stack pointer per program (keep same default)
        self.r28 = 0x000FFFFC
        self.call_stack = []

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

    def run_source(self, source: str) -> list[int]:
        self.load(source)
        return self.run()

    def execute_instruction(self, inst: Instruction, labels: dict[str, int]) -> None:
        op = inst.op

        if op == "push":
            assert isinstance(inst.arg, int)
            self.stack.append(inst.arg)
            self.pc += 1
            return

        if op == "arg":
            # push function argument by index from current call frame memory
            assert isinstance(inst.arg, int)
            if not self.call_stack:
                raise SolVMError("'arg' used outside of function frame")
            frame = self.call_stack[-1]
            func_name = frame.get("func_name")
            if func_name is None:
                raise SolVMError("internal error: frame has no func_name for arg access")
            func_meta = self.program.functions.get(func_name)
            if func_meta is None:
                raise SolVMError(f"unknown function metadata for {func_name}")
            argcount = func_meta["argcount"]
            n_locals = func_meta["n_locals"]
            idx = inst.arg
            if idx < 0 or idx >= argcount:
                raise SolVMError(f"argument index out of range: {idx}")
            addr = frame["frame_base"] + 4 * (n_locals + idx)
            self.stack.append(self._read_mem(addr, 4))
            self.pc += 1
            return

        if op == "local_addr":
            # push address of local variable (frame-relative)
            assert isinstance(inst.arg, int)
            if not self.call_stack:
                raise SolVMError("local used outside of function frame")
            frame = self.call_stack[-1]
            addr = frame["frame_base"] + 4 * inst.arg
            self.stack.append(addr)
            self.pc += 1
            return

        if op == "call":
            # start a function call: allocate frame in memory, copy args from caller stack into frame, push frame and jump
            assert isinstance(inst.arg, str)
            func_name = inst.arg
            if self.program is None:
                raise SolVMError("no program loaded")
            if func_name not in self.program.functions:
                raise SolVMError(f"unknown function: {func_name}")
            func_meta = self.program.functions[func_name]
            argcount = func_meta["argcount"]
            n_locals = func_meta["n_locals"]
            frame_size = 4 * (n_locals + argcount)
            # pop arguments from data stack
            args = [0] * argcount
            for j in range(argcount - 1, -1, -1):
                args[j] = self._pop()
            # allocate frame by moving R28
            new_r28 = (self.r28 - frame_size) & 0xFFFFFFFF
            # write args into frame memory
            for j in range(argcount):
                addr = new_r28 + 4 * (n_locals + j)
                self._write_mem(addr, args[j], 4)
            # create frame record
            frame = {"ret_pc": self.pc + 1, "frame_base": new_r28, "func_name": func_name}
            # update R28
            self.r28 = new_r28
            self.call_stack.append(frame)
            # jump to function entry
            if func_name not in labels:
                raise SolVMError(f"undefined function label: {func_name}")
            self.pc = labels[func_name]
            return

        if op == "ret":
            # return from function: pop frame and restore R28 and pc
            if not self.call_stack:
                raise SolVMError("ret without call frame")
            frame = self.call_stack.pop()
            # restore R28 (frame_base + frame_size)
            func_meta = self.program.functions.get(frame.get("func_name") if frame.get("func_name") else "")
            if func_meta is None:
                # fallback: no meta, keep R28 unchanged
                pass
            else:
                n_locals = func_meta["n_locals"]
                argcount = func_meta["argcount"]
                frame_size = 4 * (n_locals + argcount)
                self.r28 = (frame["frame_base"] + frame_size) & 0xFFFFFFFF
            self.pc = frame["ret_pc"]
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
