"""sol -> SRC32 assembly compiler (phase 2 start)."""

from __future__ import annotations

from collections import defaultdict
import re

from sol_vm import Instruction, Program, SolVMError, compile_program


ENTRY_LABEL = "__solc_entry"
STACK_CACHE_REGS = tuple(f"R{i}" for i in range(1, 13))
STACK_SIZE_BYTES = 0x00100000
SHORT_MODE_SWITCH_COST = 2


class SolCompileError(RuntimeError):
    pass


def _format_imm(value: int) -> str:
    masked = value & 0xFFFFFFFF
    if masked >= 0x80000000:
        return f"0x{masked:08X}"
    return hex(masked)


def _emit_load_imm32(lines: list[str], reg: str, value: int) -> None:
    imm = _format_imm(value)
    lines.append(f"    ADDI {reg}, R0, 0")
    lines.append(f"    LDIH {reg}, {imm}")
    lines.append(f"    LDIL {reg}, {imm}")


def _emit_short_trampoline(lines: list[str], short_instructions: list[str], label_tag: str) -> None:
    # Enter short mode at the next instruction, execute a tiny short block,
    # then return to normal mode immediately.
    lines.append("    JMPS R!0")
    for insn in short_instructions:
        lines.append(f"    {insn}")
    lines.append("    S.RET")


def _short_reg_encodable(reg: str) -> bool:
    if reg == "R31":
        return True
    if not reg.startswith("R"):
        return False
    try:
        idx = int(reg[1:])
    except ValueError:
        return False
    return 0 <= idx <= 14


def _next_short_tag(short_tag_counter: list[int], prefix: str) -> str:
    tag = f"{prefix}_{short_tag_counter[0]}"
    short_tag_counter[0] += 1
    return tag


def _emit_reg_move(
    lines: list[str],
    dst: str,
    src: str,
    *,
    use_short_mode: bool,
    short_tag_counter: list[int],
    tag_prefix: str,
) -> None:
    if dst == src:
        return
    if use_short_mode and _short_reg_encodable(dst) and _short_reg_encodable(src):
        _emit_short_trampoline(
            lines,
            [f"S.MOV {dst}, {src}"],
            _next_short_tag(short_tag_counter, tag_prefix),
        )
        return
    lines.append(f"    ADDI {dst}, {src}, 0")


_SHORT_INSN_RE = re.compile(r"^\s*S\.[A-Z]+\b.*$")


def _parse_short_trampoline_block(lines: list[str], start: int) -> tuple[list[str], int] | None:
    if start >= len(lines) or lines[start].strip() not in {"JMPS 0", "JMPS R!0"}:
        return None

    body_lines: list[str] = []
    i = start + 1
    while i < len(lines):
        line = lines[i]
        if line.strip() == "S.RET":
            if not body_lines:
                return None
            if not all(_SHORT_INSN_RE.match(body_line) for body_line in body_lines):
                return None
            return body_lines, i
        if not _SHORT_INSN_RE.match(line):
            return None
        body_lines.append(line)
        i += 1

    return None


def _is_ignorable_between_short_blocks(line: str) -> bool:
    stripped = line.strip()
    if not stripped:
        return True
    return stripped.startswith(";")


def _coalesce_short_trampolines(lines: list[str]) -> list[str]:
    out: list[str] = []
    i = 0
    while i < len(lines):
        block = _parse_short_trampoline_block(lines, i)
        if block is None:
            out.append(lines[i])
            i += 1
            continue

        mov_lines, end_index = block
        bodies = [mov_lines]
        separator_lines: list[str] = []
        j = end_index + 1
        while True:
            k = j
            gap_lines: list[str] = []
            while k < len(lines) and _is_ignorable_between_short_blocks(lines[k]):
                gap_lines.append(lines[k])
                k += 1

            next_block = _parse_short_trampoline_block(lines, k)
            if next_block is None:
                separator_lines.extend(gap_lines)
                j = k
                break
            next_body_lines, next_end_index = next_block
            separator_lines.extend(gap_lines)
            bodies.append(next_body_lines)
            j = next_end_index + 1

        if len(bodies) == 1:
            out.extend(lines[i : end_index + 1])
            out.extend(separator_lines)
            i = j
            continue

        out.append("    JMPS R!0")
        for body_lines in bodies:
            out.extend(body_lines)
        out.append("    S.RET")
        out.extend(separator_lines)
        i = j

    return out


def _plan_short_mode(instructions: list[Instruction], use_short_mode: bool) -> list[bool]:
    plan = [False] * len(instructions)
    if not use_short_mode:
        return plan

    i = 0
    while i < len(instructions):
        profile = instructions[i].profile
        if profile is None or profile.barrier or not profile.shortable:
            i += 1
            continue

        j = i
        short_weight = 0
        normal_weight = 0
        while j < len(instructions):
            next_profile = instructions[j].profile
            if next_profile is None or next_profile.barrier or not next_profile.shortable:
                break
            short_weight += next_profile.short_weight
            normal_weight += next_profile.normal_weight
            j += 1

        if normal_weight - short_weight > SHORT_MODE_SWITCH_COST:
            for k in range(i, j):
                plan[k] = True

        i = j if j > i else i + 1

    return plan


def _emit_load_small_u8_with_short(lines: list[str], reg: str, value: int, label_tag: str) -> bool:
    if reg not in {f"R{i}" for i in range(15)} | {"R31"}:
        return False
    if not (0 <= value <= 0xFF):
        return False
    _emit_short_trampoline(lines, [f"S.LDI {reg}, 0x{value:02X}"], label_tag)
    return True


class _StackCacheEmitter:
    def __init__(self, lines: list[str], use_short_mode: bool, short_tag_counter: list[int]) -> None:
        self.lines = lines
        self.use_short_mode = use_short_mode
        self.short_tag_counter = short_tag_counter
        self.regs = STACK_CACHE_REGS
        self.capacity = len(self.regs)
        self.depth = 0
        self.head = 0

    def _slot_index(self, depth_from_top: int) -> int:
        return (self.head - depth_from_top) % self.capacity

    def _slot_reg(self, depth_from_top: int) -> str:
        return self.regs[self._slot_index(depth_from_top)]

    def _copy_reg(self, dst: str, src: str) -> None:
        _emit_reg_move(
            self.lines,
            dst,
            src,
            use_short_mode=self.use_short_mode,
            short_tag_counter=self.short_tag_counter,
            tag_prefix="mov",
        )

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
        self._copy_reg("R13", top_reg)
        self._copy_reg(top_reg, second_reg)
        self._copy_reg(second_reg, "R13")

    def flush(self) -> None:
        for depth_from_top in range(self.depth - 1, -1, -1):
            self._spill_reg(self._slot_reg(depth_from_top))
        self.depth = 0

    def reset_empty(self) -> None:
        self.depth = 0
        self.head = 0


def _emit_instruction(lines: list[str], cache: _StackCacheEmitter, inst: Instruction, pc: int = 0, debug: bool = False, current_func: str | None = None, functions_map: dict[str, int] | None = None, use_short_mode: bool = True, short_tag_counter: list[int] | None = None) -> None:
    if short_tag_counter is None:
        short_tag_counter = [0]
    op = inst.op
    if debug:
        lines.append(f"    ; {op} {inst.arg if inst.arg is not None else ''}".rstrip())

    if op == "push":
        assert isinstance(inst.arg, int)
        if use_short_mode and _emit_load_small_u8_with_short(lines, "R13", inst.arg, f"push_{pc}"):
            pass
        elif inst.arg == 0:
            _emit_reg_move(
                lines,
                "R13",
                "R0",
                use_short_mode=use_short_mode,
                short_tag_counter=short_tag_counter,
                tag_prefix="push_zero",
            )
        else:
            _emit_load_imm32(lines, "R13", inst.arg)
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
        _emit_load_imm32(lines, "R14", 0xFFFFFFFF)
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
        _emit_load_imm32(lines, "R15", 0x1)
        lines.append("    SLTU R13, R13, R15")
        lines.append("    XOR R13, R13, R15")
        cache.push_from("R13")
        return

    if op == "neq":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    XOR R13, R13, R14")
        lines.append("    SLTU R13, R0, R13")
        _emit_load_imm32(lines, "R14", 0x1)
        lines.append("    XOR R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "lt":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    SLT R13, R13, R14")
        _emit_load_imm32(lines, "R14", 0x1)
        lines.append("    XOR R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "gt":
        cache.pop_to("R14")
        cache.pop_to("R13")
        lines.append("    SLT R13, R14, R13")
        _emit_load_imm32(lines, "R14", 0x1)
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
        _emit_load_imm32(lines, "R13", 0)
        lines.append(f"    JMP {end_label}")
        lines.append(f"{false_label}:")
        _emit_load_imm32(lines, "R13", 1)
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
        _emit_load_imm32(lines, "R13", 0)
        lines.append(f"    JMP {end_label}")
        lines.append(f"{false_label}:")
        _emit_load_imm32(lines, "R13", 1)
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
        cache._copy_reg("R13", reg_a)
        cache._copy_reg("R14", reg_b)
        cache._copy_reg("R15", reg_c)
        cache._copy_reg(reg_c, "R13")
        cache._copy_reg(reg_b, "R15")
        cache._copy_reg(reg_a, "R14")
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
        _emit_load_imm32(lines, "R14", 31)
        lines.append("    SRA R13, R13, R14")
        _emit_load_imm32(lines, "R14", 0x1)
        lines.append("    AND R13, R13, R14")
        cache.push_from("R13")
        return

    if op == "stacksize":
        _emit_load_imm32(lines, "R13", STACK_SIZE_BYTES)
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


def emit_src32_from_program(program: Program, debug: bool=False, stack_top: int = 0x000FFFFC, use_short_mode: bool = True) -> str:
    if ENTRY_LABEL in program.labels:
        raise SolCompileError(f"label '{ENTRY_LABEL}' is reserved")

    labels_by_pc: dict[int, list[str]] = defaultdict(list)
    for label_name, pc in sorted(program.labels.items(), key=lambda x: (x[1], x[0])):
        labels_by_pc[pc].append(label_name)

    lines: list[str] = []
    short_tag_counter = [0]
    short_plan = _plan_short_mode(program.instructions, use_short_mode)
    lines.append(".ORG 0x00000000")
    lines.append(f"{ENTRY_LABEL}:")
    _emit_load_imm32(lines, "R28", stack_top)
    cache = _StackCacheEmitter(lines, use_short_mode=use_short_mode, short_tag_counter=short_tag_counter)

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
                _emit_reg_move(
                    lines,
                    "R3",
                    "R26",
                    use_short_mode=use_short_mode,
                    short_tag_counter=short_tag_counter,
                    tag_prefix="fn_save_fb",
                )
                lines.append("    ST R3, [R28 + 0]")
                _emit_reg_move(
                    lines,
                    "R3",
                    "R31",
                    use_short_mode=use_short_mode,
                    short_tag_counter=short_tag_counter,
                    tag_prefix="fn_save_lr",
                )
                lines.append("    ST R3, [R28 + 4]")
                _emit_reg_move(
                    lines,
                    "R26",
                    "R28",
                    use_short_mode=use_short_mode,
                    short_tag_counter=short_tag_counter,
                    tag_prefix="fn_set_fb",
                )
        _emit_instruction(
            lines,
            cache,
            inst,
            pc=pc,
            debug=debug,
            current_func=current_func,
            functions_map=program.functions,
            use_short_mode=use_short_mode and short_plan[pc],
            short_tag_counter=short_tag_counter,
        )

    if not program.instructions or program.instructions[-1].op != "halt":
        cache.flush()
        lines.append("    HALT")

    lines = _coalesce_short_trampolines(lines)

    if program.read_only_data:
        emitted_orgs: set[int] = set()
        for addr, data in program.read_only_data:
            if addr not in emitted_orgs:
                lines.append(f".ORG {_format_imm(addr)}")
                emitted_orgs.add(addr)
            db_bytes = ", ".join(f"0x{byte:02X}" for byte in data)
            lines.append(f".DB {db_bytes}")
    return "\n".join(lines)


def compile_to_src32_asm(source: str, debug: bool=False, var_base: int = 0x00100000, stack_top: int = 0x000FFFFC, read_only_data_base: int = 0x00020000, source_path: str | None = None, use_short_mode: bool = True) -> str:
    try:
        program = compile_program(source, var_base=var_base, read_only_data_base=read_only_data_base, source_path=source_path)
    except SolVMError as exc:
        raise SolCompileError(str(exc)) from exc
    return emit_src32_from_program(program, debug=debug, stack_top=stack_top, use_short_mode=use_short_mode)
