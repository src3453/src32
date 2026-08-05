"""sol -> SRC32 assembly compiler (phase 2 start)."""

from __future__ import annotations

from collections import defaultdict

from sol_vm import Instruction, Program, SolVMError, compile_program


ENTRY_LABEL = "__solc_entry"


class SolCompileError(RuntimeError):
    pass


def _format_imm(value: int) -> str:
    masked = value & 0xFFFFFFFF
    if masked >= 0x80000000:
        return f"0x{masked:08X}"
    return hex(masked)


def _emit_push(lines: list[str], reg: str) -> None:
    lines.append("    ADDI R28, R28, -4")
    lines.append(f"    ST {reg}, [R28 + 0]")


def _emit_pop(lines: list[str], reg: str) -> None:
    lines.append(f"    LD {reg}, [R28 + 0]")
    lines.append("    ADDI R28, R28, 4")


def _emit_instruction(lines: list[str], inst: Instruction, pc: int = 0, debug: bool = False) -> None:
    op = inst.op
    if debug:
        lines.append(f"    ; {op} {inst.arg if inst.arg is not None else ''}".rstrip())

    if op == "push":
        assert isinstance(inst.arg, int)
        lines.append(f"    LDI R1, {_format_imm(inst.arg)}")
        _emit_push(lines, "R1")
        return

    if op in {"add", "sub", "mul", "div"}:
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        lines.append(f"    {op.upper()} R1, R1, R2")
        _emit_push(lines, "R1")
        return

    if op == "ld":
        _emit_pop(lines, "R1")
        lines.append("    LD R1, [R1 + 0]")
        _emit_push(lines, "R1")
        return

    if op == "st":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        lines.append("    ST R1, [R2 + 0]")
        return

    if op == "ldb":
        _emit_pop(lines, "R1")
        lines.append("    LDB R1, [R1 + 0]")
        _emit_push(lines, "R1")
        return

    if op == "ldh":
        _emit_pop(lines, "R1")
        lines.append("    LDH R1, [R1 + 0]")
        _emit_push(lines, "R1")
        return

    if op == "stb":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        lines.append("    STB [R2 + 0], R1")
        return

    if op == "sth":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        lines.append("    STH [R2 + 0], R1")
        return

    if op == "eq":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        true_label = f"__eq_true_{pc}"
        end_label = f"__eq_end_{pc}"
        lines.append(f"    BEQ R1, R2, {true_label}")
        lines.append("    LDI R1, 0")
        lines.append(f"    JMP {end_label}")
        lines.append(f"{true_label}:")
        lines.append("    LDI R1, 1")
        lines.append(f"{end_label}:")
        _emit_push(lines, "R1")
        return

    if op == "neq":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        true_label = f"__neq_true_{pc}"
        end_label = f"__neq_end_{pc}"
        lines.append(f"    BNE R1, R2, {true_label}")
        lines.append("    LDI R1, 0")
        lines.append(f"    JMP {end_label}")
        lines.append(f"{true_label}:")
        lines.append("    LDI R1, 1")
        lines.append(f"{end_label}:")
        _emit_push(lines, "R1")
        return

    if op == "lt":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        lines.append("    SLT R1, R1, R2")
        _emit_push(lines, "R1")
        return

    if op == "gt":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        lines.append("    SLT R1, R2, R1")
        _emit_push(lines, "R1")
        return

    if op == "le":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        false_label = f"__le_false_{pc}"
        end_label = f"__le_end_{pc}"
        lines.append("    SLT R1, R2, R1")
        lines.append(f"    BNE R1, R0, {false_label}")
        lines.append("    LDI R1, 1")
        lines.append(f"    JMP {end_label}")
        lines.append(f"{false_label}:")
        lines.append("    LDI R1, 0")
        lines.append(f"{end_label}:")
        _emit_push(lines, "R1")
        return

    if op == "ge":
        _emit_pop(lines, "R2")
        _emit_pop(lines, "R1")
        false_label = f"__ge_false_{pc}"
        end_label = f"__ge_end_{pc}"
        lines.append("    SLT R1, R1, R2")
        lines.append(f"    BNE R1, R0, {false_label}")
        lines.append("    LDI R1, 1")
        lines.append(f"    JMP {end_label}")
        lines.append(f"{false_label}:")
        lines.append("    LDI R1, 0")
        lines.append(f"{end_label}:")
        _emit_push(lines, "R1")
        return

    if op == "dup":
        lines.append("    LD R1, [R28 + 0]")
        _emit_push(lines, "R1")
        return

    if op == "drop":
        lines.append("    ADDI R28, R28, 4")
        return

    if op == "swap":
        lines.append("    LD R1, [R28 + 0]")
        lines.append("    LD R2, [R28 + 4]")
        lines.append("    ST R1, [R28 + 4]")
        lines.append("    ST R2, [R28 + 0]")
        return

    if op == "jmp":
        assert isinstance(inst.arg, str)
        lines.append(f"    JMP {inst.arg}")
        return

    if op == "jz":
        assert isinstance(inst.arg, str)
        _emit_pop(lines, "R1")
        lines.append(f"    BEQ R1, R0, {inst.arg}")
        return

    if op == "jnz":
        assert isinstance(inst.arg, str)
        _emit_pop(lines, "R1")
        lines.append(f"    BNE R1, R0, {inst.arg}")
        return

    if op == "halt":
        lines.append("    HALT")
        return

    raise SolCompileError(f"unsupported opcode for compile: {op}")


def emit_src32_from_program(program: Program, debug: bool=False, stack_top: int = 0x0000FFFC) -> str:
    if ENTRY_LABEL in program.labels:
        raise SolCompileError(f"label '{ENTRY_LABEL}' is reserved")

    labels_by_pc: dict[int, list[str]] = defaultdict(list)
    for label_name, pc in sorted(program.labels.items(), key=lambda x: (x[1], x[0])):
        labels_by_pc[pc].append(label_name)

    lines: list[str] = []
    lines.append(".ORG 0x00000000")
    lines.append(f"{ENTRY_LABEL}:")
    lines.append(f"    LDI R28, {_format_imm(stack_top)}")

    if not program.instructions:
        lines.append("    HALT")
        return "\n".join(lines)

    for pc, inst in enumerate(program.instructions):
        for label_name in labels_by_pc.get(pc, []):
            lines.append(f"{label_name}:")
        _emit_instruction(lines, inst, pc=pc, debug=debug)

    if program.instructions[-1].op != "halt":
        lines.append("    HALT")
    return "\n".join(lines)


def compile_to_src32_asm(source: str, debug: bool=False, var_base: int = 0x00100000, stack_top: int = 0x0000FFFC) -> str:
    try:
        program = compile_program(source, var_base=var_base)
    except SolVMError as exc:
        raise SolCompileError(str(exc)) from exc
    return emit_src32_from_program(program, debug=debug, stack_top=stack_top)
