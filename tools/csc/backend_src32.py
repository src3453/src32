"""
backend_src32.py
Translate csc bytecode emitter output into SRC32 assembly (asm.py-compatible).

Assumptions:
- Stack grows downward at address 0x0000FFFC. SP is R28.
- Global variable area at label `vars` (word-aligned), accessed via GP (R30).
- Return value is placed into R1.
- `RETURN` returns to the caller via `JR R31`.
- `HALT` stops program execution after `main` returns.
"""
from typing import List


def emit_src32(emitter) -> str:
    code = getattr(emitter, "code", [])
    labels = {i: f"bc_{i}" for i in range(len(code))}

    # Detect number of vars from emitter metadata if present.
    var_count = _get_var_count(emitter)

    asm_lines: List[str] = []

    # Prologue: set SP and GP
    asm_lines.append(".ORG 0x00000000")
    asm_lines.append("start:")
    _emit_load_imm32(asm_lines, "R28", 0x0000FFFC)
    _emit_load_imm32(asm_lines, "R30", "vars")

    # If a main entrypoint exists, call it first.
    main_label = None
    func_labels = getattr(emitter, "func_labels", None)
    if isinstance(func_labels, dict) and "main" in func_labels:
        main_idx = func_labels["main"]
        main_label = labels.get(main_idx)
    if main_label:
        asm_lines.append(f"    JAL {main_label}")
        asm_lines.append("    HALT")

    def emit_push_reg(reg: str) -> None:
        asm_lines.append("    ADDI R28, R28, -4")
        asm_lines.append(f"    ST {reg}, [R28 + 0]")

    def emit_pop_reg(reg: str) -> None:
        asm_lines.append(f"    LD {reg}, [R28 + 0]")
        asm_lines.append("    ADDI R28, R28, 4")

    for i, instr in enumerate(code):
        asm_lines.append(f"{labels[i]}: ; {" ".join([str(x) for x in instr])}")
        op = instr[0]

        if op == "CALL":
            funcname = instr[1]
            if isinstance(func_labels, dict) and funcname in func_labels:
                target_idx = func_labels[funcname]
                target_label = labels[target_idx]
                asm_lines.append(f"    JAL {target_label}")
                emit_push_reg("R1")
            else:
                raise SyntaxError(f"CALL to unknown function {funcname}")

        elif op == "PUSH_CONST":
            val = instr[1]
            _emit_load_imm32(asm_lines, "R1", val)
            emit_push_reg("R1")

        elif op == "POP":
            emit_pop_reg("R1")

        elif op == "SAVE_RET":
            emit_pop_reg("R31")

        elif op == "LOAD_VAR":
            idx = instr[1]
            offset = idx * 4
            asm_lines.append(f"    LD R1, [R30 + {offset}]    ; load var {idx}")
            emit_push_reg("R1")

        elif op == "STORE_VAR":
            idx = instr[1]
            offset = idx * 4
            emit_pop_reg("R1")
            asm_lines.append(f"    ST R1, [R30 + {offset}]    ; store var {idx}")

        elif op in ("ADD", "SUB", "MUL", "DIV", "AND", "OR"):
            emit_pop_reg("R2")
            emit_pop_reg("R1")
            if op == "ADD":
                asm_lines.append("    ADD R1, R1, R2")
            elif op == "SUB":
                asm_lines.append("    SUB R1, R1, R2")
            elif op == "MUL":
                asm_lines.append("    MUL R1, R1, R2")
            elif op == "DIV":
                asm_lines.append("    DIV R1, R1, R2")
            elif op == "AND":
                asm_lines.append("    AND R1, R1, R2")
            elif op == "OR":
                asm_lines.append("    OR R1, R1, R2")
            emit_push_reg("R1")

        elif op == "CMP":
            cmpop = instr[1]
            emit_pop_reg("R2")
            emit_pop_reg("R1")

            if cmpop == "==":
                asm_lines.append(f"    BEQ R1, R2, cmp_true_{i}")
                _emit_load_imm32(asm_lines, "R1", 0)
                asm_lines.append(f"    JMP cmp_end_{i}")
                asm_lines.append(f"cmp_true_{i}:")
                _emit_load_imm32(asm_lines, "R1", 1)
                asm_lines.append(f"cmp_end_{i}:")
                emit_push_reg("R1")

            elif cmpop == "!=":
                asm_lines.append(f"    BNE R1, R2, cmp_true_{i}")
                _emit_load_imm32(asm_lines, "R1", 0)
                asm_lines.append(f"    JMP cmp_end_{i}")
                asm_lines.append(f"cmp_true_{i}:")
                _emit_load_imm32(asm_lines, "R1", 1)
                asm_lines.append(f"cmp_end_{i}:")
                emit_push_reg("R1")

            elif cmpop == "<":
                asm_lines.append("    SLT R1, R1, R2")
                emit_push_reg("R1")

            elif cmpop == ">":
                asm_lines.append("    SLT R1, R2, R1")
                emit_push_reg("R1")

            elif cmpop == "<=":
                asm_lines.append("    SLT R1, R2, R1")
                asm_lines.append(f"    BEQ R1, R0, le_true_{i}")
                _emit_load_imm32(asm_lines, "R1", 0)
                asm_lines.append(f"    JMP le_end_{i}")
                asm_lines.append(f"le_true_{i}:")
                _emit_load_imm32(asm_lines, "R1", 1)
                asm_lines.append(f"le_end_{i}:")
                emit_push_reg("R1")

            elif cmpop == ">=":
                asm_lines.append("    SLT R1, R1, R2")
                asm_lines.append(f"    BEQ R1, R0, ge_true_{i}")
                _emit_load_imm32(asm_lines, "R1", 0)
                asm_lines.append(f"    JMP ge_end_{i}")
                asm_lines.append(f"ge_true_{i}:")
                _emit_load_imm32(asm_lines, "R1", 1)
                asm_lines.append(f"ge_end_{i}:")
                emit_push_reg("R1")

            else:
                raise NotImplementedError(f"Unknown cmp {cmpop}")

        elif op == "JUMP_IF_FALSE":
            target = instr[1]
            emit_pop_reg("R1")
            asm_lines.append(f"    BEQ R1, R0, {labels[target]}")

        elif op == "JUMP":
            target = instr[1]
            asm_lines.append(f"    JMP {labels[target]}")

        elif op == "RETURN":
            emit_pop_reg("R1")
            asm_lines.append("    JR R31")

        elif op == "HALT":
            asm_lines.append("    HALT")

        else:
            raise NotImplementedError(f"Backend: unknown op {op}")

    asm_lines.append("")
    asm_lines.append("; data section")
    asm_lines.append(".ORG 0x00008000")
    asm_lines.append("vars:")
    for _ in range(var_count):
        asm_lines.append("    .DWORD 0")

    return "\n".join(asm_lines)


def _get_var_count(emitter) -> int:
    if hasattr(emitter, "var_count") and isinstance(emitter.var_count, int):
        return max(emitter.var_count, 0)

    var_index = getattr(emitter, "var_index", None)
    if isinstance(var_index, dict) and var_index:
        return max(var_index.values()) + 1
    if isinstance(var_index, int):
        return max(var_index, 0)

    return 0


def format_imm(val: int) -> str:
    if isinstance(val, int):
        # Always use hex for SRC32 assembler compatibility
        if val < 0:
            return str(val)
        return hex(val)
    return str(val)


def _emit_load_imm32(asm_lines: List[str], reg: str, value) -> None:
    # LDIH/LDIL each consume one 16-bit half of the value.
    if isinstance(value, int):
        hi = (value >> 16) & 0xFFFF
        lo = value & 0xFFFF
        asm_lines.append(f"    LDIH {reg}, 0x{hi:04X}")
        asm_lines.append(f"    LDIL {reg}, 0x{lo:04X}")
    else:
        # `vars` is fixed by this backend's data-section layout. A single
        # symbolic LDIL is sufficient while the assembler has no relocations.
        asm_lines.append(f"    LDIH {reg}, 0x0000")
        asm_lines.append(f"    LDIL {reg}, {value}")