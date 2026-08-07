"""sol -> SRC32 assembly compiler (phase 2 start)."""

from __future__ import annotations

from collections import defaultdict

from sol_vm import Instruction, Program, SolVMError, compile_program


ENTRY_LABEL = "__solc_entry"
STACK_CACHE_REGS = tuple(f"R{i}" for i in range(1, 13))
STACK_SIZE_BYTES = 0x00100000


class SolCompileError(RuntimeError):
    pass


def _format_imm(value: int) -> str:
    masked = value & 0xFFFFFFFF
    if masked >= 0x80000000:
        return f"0x{masked:08X}"
    return hex(masked)


class _StackCacheEmitter:
    def __init__(self, lines: list[str]) -> None:
        self.lines = lines
        self.regs = STACK_CACHE_REGS
        self.capacity = len(self.regs)
        self.depth = 0
        self.head = 0

    def _slot_index(self, depth_from_top: int) -> int:
        return (self.head - depth_from_top) % self.capacity

    def _slot_reg(self, depth_from_top: int) -> str:
        return self.regs[self._slot_index(depth_from_top)]

    def _copy_reg(self, dst: str, src: str) -> None:
        if dst == src:
            return
        self.lines.append(f"    ADDI {dst}, {src}, 0")

    def _spill_reg(self, reg: str) -> None:
        self.lines.append("    ADDI R28, R28, -4")
        self.lines.append(f"    ST {reg}, [R28 + 0]")

    def _fill_new_bottom(self) -> None:
        if self.depth == 0:
            self.head = 0
            slot = 0
        else:
            slot = (self.head - self.depth) % self.capacity
        reg = self.regs[slot]
        self.lines.append(f"    LD {reg}, [R28 + 0]")
        self.lines.append("    ADDI R28, R28, 4")
        self.depth += 1

    def ensure_cached(self, count: int) -> None:
        if count > self.capacity:
            raise SolCompileError(f"stack cache request exceeds capacity: {count} > {self.capacity}")
        while self.depth < count:
            self._fill_new_bottom()

    def push_from(self, src: str) -> None:
        if self.depth == self.capacity:
            bottom_reg = self._slot_reg(self.depth - 1)
            self._spill_reg(bottom_reg)
            self.head = (self.head + 1) % self.capacity
            dst = self.regs[self.head]
            self._copy_reg(dst, src)
            return
        if self.depth == 0:
            self.head = 0
        else:
            self.head = (self.head + 1) % self.capacity
        dst = self.regs[self.head]
        self._copy_reg(dst, src)
        self.depth += 1

    def pop_to(self, dst: str) -> None:
        if self.depth == 0:
            self.lines.append(f"    LD {dst}, [R28 + 0]")
            self.lines.append("    ADDI R28, R28, 4")
            return
        top_reg = self._slot_reg(0)
        self._copy_reg(dst, top_reg)
        if self.depth == 1:
            self.depth = 0
            return
        self.head = (self.head - 1) % self.capacity
        self.depth -= 1

    def pop_discard(self) -> None:
        if self.depth == 0:
            self.lines.append("    ADDI R28, R28, 4")
            return
        if self.depth == 1:
            self.depth = 0
            return
        self.head = (self.head - 1) % self.capacity
        self.depth -= 1

    def peek_to(self, dst: str, depth_from_top: int = 0) -> None:
        if depth_from_top < 0:
            raise SolCompileError(f"invalid stack depth: {depth_from_top}")
        self.ensure_cached(depth_from_top + 1)
        self._copy_reg(dst, self._slot_reg(depth_from_top))

    def swap_top_two(self) -> None:
        self.ensure_cached(2)
        top_reg = self._slot_reg(0)
        second_reg = self._slot_reg(1)
        self.lines.append(f"    ADDI R13, {top_reg}, 0")
        self.lines.append(f"    ADDI {top_reg}, {second_reg}, 0")
        self.lines.append(f"    ADDI {second_reg}, R13, 0")

    def flush(self) -> None:
        for depth_from_top in range(self.depth - 1, -1, -1):
            self._spill_reg(self._slot_reg(depth_from_top))
        self.depth = 0

    def reset_empty(self) -> None:
        self.depth = 0
        self.head = 0


def _emit_instruction(lines: list[str], cache: _StackCacheEmitter, inst: Instruction, pc: int = 0, debug: bool = False, current_func: str | None = None, functions_map: dict[str, int] | None = None) -> None:
    op = inst.op
    if debug:
        lines.append(f"    ; {op} {inst.arg if inst.arg is not None else ''}".rstrip())

    if op == "push":
        assert isinstance(inst.arg, int)
        lines.append(f"    LDI R13, {_format_imm(inst.arg)}")
        cache.push_from("R13")
        return

    if op == "arg":
        # push argument by index from the current function frame
        assert isinstance(inst.arg, int)
        if current_func is None or functions_map is None:
            raise SolCompileError("'arg' emitted outside of function or missing functions_map")
        meta = functions_map.get(current_func)
        if meta is None:
            raise SolCompileError(f"unknown function in emitter: {current_func}")
        argcount = meta["argcount"]
        n_locals = meta["n_locals"]
        idx = inst.arg
        if idx < 0 or idx >= argcount:
            raise SolCompileError(f"argument index out of range for {current_func}: {idx}")
        # frame layout: saved frame base, saved return address, locals, then arguments in source order
        offset = 4 * (2 + n_locals + idx)
        lines.append(f"    LD R13, [R26 + {offset}]")
        cache.push_from("R13")
        return

    if op == "call":
        assert isinstance(inst.arg, str)
        cache.flush()
        func_name = inst.arg
        if functions_map is None or func_name not in functions_map:
            raise SolCompileError(f"call to unknown function in emitter: {func_name}")
        argcount = functions_map[func_name]["argcount"]
        n_locals = functions_map[func_name]["n_locals"]
        frame_size = 4 * (2 + n_locals + argcount)
        # allocate frame and copy caller's args into frame slots.
        lines.append("    ADDI R14, R28, 0")
        # ADDI R28, R28, -frame_size ; allocate
        lines.append(f"    ADDI R28, R28, -{frame_size}")
        # copy args from old stack (R2) to frame slots (R28)
        for j in range(argcount):
            old_off = 4 * (argcount - 1 - j)
            new_off = 4 * (2 + n_locals + j)
            lines.append(f"    LD R15, [R14 + {old_off}]")
            lines.append(f"    ST R15, [R28 + {new_off}]")
        # jump-and-link
        lines.append(f"    JAL {func_name}")
        return

    if op == "ret":
        cache.flush()
        # Return value is the top of the callee stack; preserve it across frame teardown.
        if current_func is None or functions_map is None:
            raise SolCompileError("ret emitted outside of function or missing functions_map")
        meta = functions_map.get(current_func)
        if meta is None:
            raise SolCompileError(f"unknown function metadata for {current_func}")
        frame_size = 4 * (2 + meta["n_locals"] + meta["argcount"])
        restore_size = frame_size + 4 * meta["argcount"]
        lines.append("    LD R13, [R28 + 0]")
        lines.append("    LD R31, [R26 + 4]")
        lines.append("    LD R15, [R26 + 0]")
        if restore_size != 0:
            lines.append(f"    ADDI R28, R28, {restore_size}")
        lines.append("    ADDI R26, R15, 0")
        lines.append("    ADDI R28, R28, -4")
        lines.append("    ST R13, [R28 + 0]")
        lines.append("    JR R31")
        return

    if op == "retn":
        cache.flush()
        # retn discards any callee-produced values and restores the caller stack past args.
        if current_func is None or functions_map is None:
            raise SolCompileError("retn emitted outside of function or missing functions_map")
        meta = functions_map.get(current_func)
        if meta is None:
            raise SolCompileError(f"unknown function metadata for {current_func}")
        frame_size = 4 * (2 + meta["n_locals"] + meta["argcount"])
        restore_size = frame_size + 4 * meta["argcount"]
        lines.append("    LD R31, [R26 + 4]")
        lines.append("    LD R15, [R26 + 0]")
        if restore_size != 0:
            lines.append(f"    ADDI R28, R28, {restore_size}")
        lines.append("    ADDI R26, R15, 0")
        lines.append("    JR R31")
        return

    if op == "local_addr":
        # push address of local var from the stable frame base
        assert isinstance(inst.arg, int)
        offset = 4 * (2 + inst.arg)
        lines.append(f"    ADDI R13, R26, {offset}")
        cache.push_from("R13")
        return

    if op in {"add", "sub", "mul", "div", "mod", "and", "or", "xor", "shl", "shr"}:
        cache.pop_to("R14")
        cache.pop_to("R13")
        if op == "shl":
            lines.append("    SLL R13, R13, R14")
        elif op == "shr":
            lines.append("    SRA R13, R13, R14")
        elif op == "mod":
            lines.append("    MOD R13, R13, R14")
        elif op == "and":
            lines.append("    AND R13, R13, R14")
        elif op == "or":
            lines.append("    OR R13, R13, R14")
        elif op == "xor":
            lines.append("    XOR R13, R13, R14")
        else:
            lines.append(f"    {op.upper()} R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "neg":
        cache.pop_to("R13")
        lines.append("    SUB R13, R0, R13")
        cache.push_from("R13")
        return

    if op == "not":
        cache.pop_to("R13")
        lines.append("    LDI R14, 0xFFFFFFFF")
        lines.append("    XOR R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "ld":
        cache.pop_to("R13")
        lines.append("    LD R13, [R13 + 0]")
        cache.push_from("R13")
        return

    if op == "st":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    ST R13, [R14 + 0]")
        return

    if op == "ldb":
        cache.pop_to("R13")
        lines.append("    LDB R13, [R13 + 0]")
        cache.push_from("R13")
        return

    if op == "ldh":
        cache.pop_to("R13")
        lines.append("    LDH R13, [R13 + 0]")
        cache.push_from("R13")
        return

    if op == "stb":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    STB [R14 + 0], R13")
        return

    if op == "sth":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    STH [R14 + 0], R13")
        return

    if op == "eq":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    XOR R13, R13, R14")
        lines.append("    LDI R15, 0x1")
        lines.append("    SLTU R13, R13, R15")
        lines.append("    XOR R13, R13, R15")
        cache.push_from("R13")
        return

    if op == "neq":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    XOR R13, R13, R14")
        lines.append("    SLTU R13, R0, R13")
        lines.append("    LDI R14, 0x1")
        lines.append("    XOR R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "lt":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    SLT R13, R13, R14")
        lines.append("    LDI R14, 0x1")
        lines.append("    XOR R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "gt":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    SLT R13, R14, R13")
        lines.append("    LDI R14, 0x1")
        lines.append("    XOR R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "le":
        cache.pop_to("R14")
        cache.pop_to("R13")
        false_label = f"__le_false_{pc}"
        end_label = f"__le_end_{pc}"
        lines.append("    SLT R13, R14, R13")
        lines.append(f"    BNE R13, R0, {false_label}")
        lines.append("    LDI R13, 0")
        lines.append(f"    JMP {end_label}")
        lines.append(f"{false_label}:")
        lines.append("    LDI R13, 1")
        lines.append(f"{end_label}:")
        cache.push_from("R13")
        return

    if op == "ge":
        cache.pop_to("R14")
        cache.pop_to("R13")
        false_label = f"__ge_false_{pc}"
        end_label = f"__ge_end_{pc}"
        lines.append("    SLT R13, R13, R14")
        lines.append(f"    BNE R13, R0, {false_label}")
        lines.append("    LDI R13, 0")
        lines.append(f"    JMP {end_label}")
        lines.append(f"{false_label}:")
        lines.append("    LDI R13, 1")
        lines.append(f"{end_label}:")
        cache.push_from("R13")
        return

    if op == "dup":
        cache.peek_to("R13", 0)
        cache.push_from("R13")
        return

    if op == "drop":
        cache.pop_discard()
        return

    if op == "swap":
        cache.swap_top_two()
        return

    if op == "over":
        cache.peek_to("R13", 1)
        cache.push_from("R13")
        return

    if op == "rot":
        cache.ensure_cached(3)
        reg_c = cache._slot_reg(0)
        reg_b = cache._slot_reg(1)
        reg_a = cache._slot_reg(2)
        lines.append(f"    ADDI R13, {reg_a}, 0")
        lines.append(f"    ADDI R14, {reg_b}, 0")
        lines.append(f"    ADDI R15, {reg_c}, 0")
        lines.append(f"    ADDI {reg_c}, R13, 0")
        lines.append(f"    ADDI {reg_b}, R15, 0")
        lines.append(f"    ADDI {reg_a}, R14, 0")
        return

    if op == "nip":
        cache.pop_to("R13")
        cache.pop_discard()
        cache.push_from("R13")
        return

    if op == "tuck":
        cache.pop_to("R14")
        cache.pop_to("R13")
        cache.push_from("R14")
        cache.push_from("R13")
        cache.push_from("R14")
        return

    if op == "sgn":
        cache.pop_to("R13")
        lines.append("    LDI R14, 31")
        lines.append("    SRA R13, R13, R14")
        lines.append("    LDI R14, 0x1")
        lines.append("    AND R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "stacksize":
        lines.append(f"    LDI R13, {_format_imm(STACK_SIZE_BYTES)}")
        cache.push_from("R13")
        return

    if op == "jmp":
        assert isinstance(inst.arg, str)
        cache.flush()
        cache.reset_empty()
        lines.append(f"    JMP {inst.arg}")
        return

    if op == "jz":
        assert isinstance(inst.arg, str)
        cache.pop_to("R13")
        cache.flush()
        cache.reset_empty()
        lines.append(f"    BEQ R13, R0, {inst.arg}")
        return

    if op == "jnz":
        assert isinstance(inst.arg, str)
        cache.pop_to("R13")
        cache.flush()
        cache.reset_empty()
        lines.append(f"    BNE R13, R0, {inst.arg}")
        return

    if op == "halt":
        cache.flush()
        cache.reset_empty()
        lines.append("    HALT")
        return

    raise SolCompileError(f"unsupported opcode for compile: {op}")


def emit_src32_from_program(program: Program, debug: bool=False, stack_top: int = 0x000FFFFC) -> str:
    if ENTRY_LABEL in program.labels:
        raise SolCompileError(f"label '{ENTRY_LABEL}' is reserved")

    labels_by_pc: dict[int, list[str]] = defaultdict(list)
    for label_name, pc in sorted(program.labels.items(), key=lambda x: (x[1], x[0])):
        labels_by_pc[pc].append(label_name)

    lines: list[str] = []
    lines.append(".ORG 0x00000000")
    lines.append(f"{ENTRY_LABEL}:")
    lines.append(f"    LDI R28, {_format_imm(stack_top)}")
    cache = _StackCacheEmitter(lines)

    current_func: str | None = None
    for pc, inst in enumerate(program.instructions):
        labels_here = labels_by_pc.get(pc, [])
        if labels_here:
            cache.flush()
            cache.reset_empty()
        for label_name in labels_here:
            lines.append(f"{label_name}:")
            # if this label is a function, switch current_func
            if label_name in program.functions:
                current_func = label_name
                lines.append("    ADDI R3, R26, 0")
                lines.append("    ST R3, [R28 + 0]")
                lines.append("    ADDI R3, R31, 0")
                lines.append("    ST R3, [R28 + 4]")
                lines.append("    ADDI R26, R28, 0")
        _emit_instruction(lines, cache, inst, pc=pc, debug=debug, current_func=current_func, functions_map=program.functions)

    if not program.instructions or program.instructions[-1].op != "halt":
        cache.flush()
        lines.append("    HALT")

    if program.read_only_data:
        emitted_orgs: set[int] = set()
        for addr, data in program.read_only_data:
            if addr not in emitted_orgs:
                lines.append(f".ORG {_format_imm(addr)}")
                emitted_orgs.add(addr)
            db_bytes = ", ".join(f"0x{byte:02X}" for byte in data)
            lines.append(f".DB {db_bytes}")
    return "\n".join(lines)


def compile_to_src32_asm(source: str, debug: bool=False, var_base: int = 0x00100000, stack_top: int = 0x000FFFFC, read_only_data_base: int = 0x00020000, source_path: str | None = None) -> str:
    try:
        program = compile_program(source, var_base=var_base, read_only_data_base=read_only_data_base, source_path=source_path)
    except SolVMError as exc:
        raise SolCompileError(str(exc)) from exc
    return emit_src32_from_program(program, debug=debug, stack_top=stack_top)
