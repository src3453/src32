import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "asm"))

from asm import Assembler
from sol_compiler import SolCompileError, compile_to_src32_asm


def test_compile_arithmetic_sequence():
    asm = compile_to_src32_asm("1 2 add")
    assert ".ORG 0x00000000" in asm
    assert "__solc_entry:" in asm
    assert "__solc_short_entry_" not in asm
    assert "JMPS R!0" in asm
    assert "S.LDI R13, 0x01" in asm
    assert "S.LDI R13, 0x02" in asm
    assert "S.RET" in asm
    assert "ADD R13, R13, R14" in asm


def test_compile_arithmetic_sequence_can_disable_short_mode():
    asm = compile_to_src32_asm("1 2 add", use_short_mode=False)
    assert "LDIH R13, 0x0000" in asm
    assert "LDIL R13, 0x0001" in asm
    assert "LDIH R13, 0x0000" in asm
    assert "LDIL R13, 0x0002" in asm


def test_compile_load_immediate_splits_high_and_low_words():
    asm = compile_to_src32_asm("0x80040000", use_short_mode=False)
    assert "LDIH R13, 0x8004" in asm
    assert "LDIL R13, 0x0000" in asm
    assert "LDIH R13, 0x80040000" not in asm
    assert "LDIL R13, 0x80040000" not in asm


def test_compile_mod_and_neg():
    asm = compile_to_src32_asm("7 4 mod 7 neg")
    assert "MOD R13, R13, R14" in asm
    assert "SUB R13, R0, R13" in asm


def test_compile_shift_ops():
    asm = compile_to_src32_asm("8 1 shl 2 shr")
    assert "SLL R13, R13, R14" in asm
    assert "SRA R13, R13, R14" in asm


def test_compile_bitwise_ops():
    asm = compile_to_src32_asm("6 3 and 5 or 7 xor not")
    assert "AND R13, R13, R14" in asm
    assert "OR R13, R13, R14" in asm
    assert "XOR R13, R13, R14" in asm
    assert "LDIH R14, 0xFFFF" in asm
    assert "LDIL R14, 0xFFFF" in asm


def test_compile_structured_if_else_and_while():
    asm = compile_to_src32_asm("1 2 lt if 3 else 4 end 0 while 1 end")
    assert "__solc_if_false_" in asm
    assert "__solc_if_end_" in asm
    assert "__solc_while_start_" in asm
    assert "BNE R13, R0" in asm
    assert "BEQ R13, R0" in asm
    assert "@loop" not in asm


def test_compile_stack_ops():
    asm = compile_to_src32_asm("1 2 swap dup drop")
    assert "S.MOV R13, R2" in asm
    assert "S.MOV R2, R1" in asm
    assert "S.MOV R1, R13" in asm


def test_compile_extended_stack_ops():
    over_asm = compile_to_src32_asm("1 2 over")
    assert "S.MOV R13, R1" in over_asm

    rot_asm = compile_to_src32_asm("1 2 3 rot")
    assert "ADDI R15, R3, 0" in rot_asm
    assert "S.MOV R3, R13" in rot_asm

    nip_asm = compile_to_src32_asm("1 2 nip")
    assert "S.MOV R13, R2" in nip_asm

    tuck_asm = compile_to_src32_asm("1 2 tuck")
    assert "S.MOV R14, R2" in tuck_asm
    assert "S.MOV R3, R14" in tuck_asm


def test_compile_coalesces_consecutive_short_mov_trampolines():
    asm = compile_to_src32_asm("1 2 swap")
    assert "S.MOV R13, R2" in asm
    assert "S.MOV R2, R1" in asm
    assert "S.MOV R1, R13" in asm
    assert asm.count("S.RET") < asm.count("S.MOV")


def test_compile_coalesces_short_mov_trampolines_with_debug_comments_between():
    asm = compile_to_src32_asm("1 2 mod", debug=True)
    assert "S.MOV R14, R2\n    S.MOV R13, R1\n    S.RET" in asm


def test_compile_coalesces_short_trampolines_with_debug_comments_between():
    asm = compile_to_src32_asm("1 2 add", debug=True)
    assert "S.LDI R13, 0x01" in asm
    assert "S.LDI R13, 0x02" in asm
    assert asm.count("S.RET") == 2


def test_compile_sgn_and_stacksize():
    asm = compile_to_src32_asm("-1 sgn stacksize")
    assert "SRA R13, R13, R14" in asm
    assert "AND R13, R13, R14" in asm
    assert "LDIH R13, 0x0010" in asm
    assert "LDIL R13, 0x0000" in asm


def test_compile_load_store():
    asm = compile_to_src32_asm("1 0 st 0 ld")
    assert "ST R13, [R14 + 0]" in asm
    assert "LD R13, [R13 + 0]" in asm


def test_compile_byte_and_halfword_load_store():
    asm = compile_to_src32_asm("0x1234 0 sth 0 ldh 0xFF 0 stb 0 ldb")
    assert "STH [R14 + 0], R13" in asm
    assert "LDH R13, [R13 + 0]" in asm
    assert "STB [R14 + 0], R13" in asm
    assert "LDB R13, [R13 + 0]" in asm


def test_compile_negative_literal_uses_two_complement_hex():
    asm = compile_to_src32_asm("-1 0 ld")
    assert "LDIH R13, 0xFFFF" in asm
    assert "LDIL R13, 0xFFFF" in asm


def test_compile_string_literal_emits_read_only_data():
    asm = compile_to_src32_asm('"hi"')
    assert "LDIH R13, 0x0002" in asm
    assert "LDIL R13, 0x0000" in asm
    assert ".ORG 0x20000" in asm
    assert ".DB 0x68, 0x69, 0x00" in asm


def test_compile_string_literal_allows_custom_read_only_data_base():
    asm = compile_to_src32_asm('"hi"', read_only_data_base=0x30000)
    assert "LDIH R13, 0x0003" in asm
    assert "LDIL R13, 0x0000" in asm
    assert ".ORG 0x30000" in asm


def test_assembler_accepts_signed_ldi():
    assembler = Assembler()
    binary = assembler.assemble("LDIH R1, -1\nLDIL R1, -1")
    assert binary[:4] == assembler.assemble("LDIH R1, -1")
    assert binary[4:] == assembler.assemble("LDIL R1, -1")


def test_compile_unknown_word_error():
    with pytest.raises(SolCompileError, match="unknown word"):
        compile_to_src32_asm("1 foo")


def test_compile_rejects_stack_underflow():
    with pytest.raises(SolCompileError, match="stack underflow"):
        compile_to_src32_asm("drop")


def test_compile_rejects_inconsistent_branch_stack_depth():
    with pytest.raises(SolCompileError, match="inconsistent stack depth"):
        compile_to_src32_asm("0 if 1 else 2 3 end add")


def test_compile_rejects_unbounded_loop_stack_growth():
    with pytest.raises(SolCompileError, match="stack overflow"):
        compile_to_src32_asm("while 1 2 end")


def test_compile_checks_function_call_arguments():
    source = "fn add1 (x) : x 1 add ret ; add1"
    with pytest.raises(SolCompileError, match="stack underflow"):
        compile_to_src32_asm(source)


def test_compile_missing_branch_operand_error():
    with pytest.raises(SolCompileError, match="hidden"):
        compile_to_src32_asm("jz")


def test_compile_does_not_append_redundant_halt():
    asm = compile_to_src32_asm("halt")
    assert asm.count("HALT") == 1


def test_compile_comparison_eq_true():
    asm = compile_to_src32_asm("5 5 eq")
    assert "XOR R13, R13, R14" in asm
    assert "SLTU R13, R13, R15" in asm
    assert "BEQ R13, R14" not in asm
    assert "BNE R13, R14" not in asm


def test_compile_comparison_eq_false():
    asm = compile_to_src32_asm("5 3 eq")
    assert "XOR R13, R13, R14" in asm
    assert "SLTU R13, R13, R15" in asm


def test_compile_comparison_neq():
    asm = compile_to_src32_asm("5 3 neq")
    assert "XOR R13, R13, R14" in asm
    assert "SLTU R13, R0, R13" in asm
    assert "BEQ R13, R14" not in asm
    assert "BNE R13, R14" not in asm


def test_compile_comparison_lt():
    asm = compile_to_src32_asm("3 5 lt")
    assert "SLT R13, R13, R14" in asm


def test_compile_comparison_gt():
    asm = compile_to_src32_asm("5 3 gt")
    assert "SLT R13, R14, R13" in asm


def test_compile_comparison_le():
    asm = compile_to_src32_asm("5 3 le")
    assert "SLT R13, R14, R13" in asm
    assert "__le_false_" in asm
    assert "__le_end_" in asm


def test_compile_comparison_ge():
    asm = compile_to_src32_asm("3 5 ge")
    assert "SLT R13, R13, R14" in asm
    assert "__ge_false_" in asm
    assert "__ge_end_" in asm


def test_compile_simple_function():
    src = """
fn add_two (a b) :
    a b add
    ret
;

1 2 add_two
"""
    asm = compile_to_src32_asm(src)
    assert "JAL add_two" in asm
    assert "JR R31" in asm
    assert "ADDI R26, R28, 0" in asm
    assert "LD R13, [R26 + 8]" in asm


def test_compile_local_variable_argument_offsets():
    src = """
fn use_local (a) :
    local t 0
    a >t
    t 2 mul
    ret
;

1 2 use_local
"""
    asm = compile_to_src32_asm(src)
    assert "ADDI R26, R28, 0" in asm
    assert "ADDI R13, R26, 8" in asm
    assert "LD R13, [R26 + 12]" in asm


def test_compile_function_can_use_constant():
    src = """
!const UART_ADDR 0x80040000

fn putc (char) :
    char UART_ADDR stb
;

0x30 putc
"""
    asm = compile_to_src32_asm(src)
    assert "LDIH R13, 0x8004" in asm
    assert "LDIL R13, 0x0000" in asm
    assert "STB [R14 + 0], R13" in asm


def test_compile_global_variable_initializes_storage_and_loads_on_reference():
    asm = compile_to_src32_asm("!var value 7 value", use_short_mode=False)
    initialization = "LDIH R13, 0x0000\n    LDIL R13, 0x0007"
    variable_address = "LDIH R13, 0x0010\n    LDIL R13, 0x0000"

    assert initialization in asm
    assert variable_address in asm
    assert "ST R13, [R14 + 0]" in asm
    assert "LD R13, [R13 + 0]" in asm


def test_compile_constant_embeds_value_without_variable_storage_access():
    asm = compile_to_src32_asm("!const value 0x10000000 value", use_short_mode=False)

    assert "LDIH R13, 0x1000" in asm
    assert "LDIL R13, 0x0000" in asm
    assert "ST R13, [R14 + 0]" not in asm
    assert "LD R13, [R13 + 0]" not in asm


def test_compile_retn_restores_past_arguments():
    src = """
fn putc (char) :
    char
;

0x30 putc
"""
    asm = compile_to_src32_asm(src)
    assert "ADDI R28, R28, 16" in asm


def test_compile_stack_cache_spills_only_after_cache_is_full():
    src = " ".join(str(i) for i in range(1, 14))
    asm = compile_to_src32_asm(src)
    first_spill = asm.find("ADDI R28, R28, -4")
    push_13 = asm.find("S.LDI R13, 0x0D")
    assert first_spill > push_13


def test_compile_stack_cache_fills_from_memory_when_cache_is_empty():
    src = " ".join(str(i) for i in range(1, 14))
    src += " " + " ".join(["drop"] * 12) + " dup"
    asm = compile_to_src32_asm(src)
    assert "LD R1, [R28 + 0]" in asm


def test_compile_call_flushes_cache_before_jal():
    src = """
fn id (a) :
    a
    ret
;

7 id
"""
    asm = compile_to_src32_asm(src)
    jal_pos = asm.find("JAL id")
    assert jal_pos > 0
    prologue = asm[:jal_pos]
    assert "ST R1, [R28 + 0]" in prologue
