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


def _emit_instruction(lines: list[str], inst: Instruction, debug: bool=False) -> None:
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


def emit_src32_from_program(program: Program, debug: bool=False) -> str:
    if ENTRY_LABEL in program.labels:
        raise SolCompileError(f"label '{ENTRY_LABEL}' is reserved")

    labels_by_pc: dict[int, list[str]] = defaultdict(list)
    for label_name, pc in sorted(program.labels.items(), key=lambda x: (x[1], x[0])):
        labels_by_pc[pc].append(label_name)

    lines: list[str] = []
    lines.append(".ORG 0x00000000")
    lines.append(f"{ENTRY_LABEL}:")
    lines.append("    LDI R28, 0x0000FFFC")

    if not program.instructions:
        lines.append("    HALT")
        return "\n".join(lines)

    for pc, inst in enumerate(program.instructions):
        for label_name in labels_by_pc.get(pc, []):
            lines.append(f"{label_name}:")
        _emit_instruction(lines, inst, debug=debug)

    if program.instructions[-1].op != "halt":
        lines.append("    HALT")
    return "\n".join(lines)


def compile_to_src32_asm(source: str, debug: bool=False) -> str:
    try:
        program = compile_program(source)
    except SolVMError as exc:
        raise SolCompileError(str(exc)) from exc
    return emit_src32_from_program(program, debug=debug)
