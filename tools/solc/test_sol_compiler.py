import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "asm"))

from asm import Assembler, enc_ldi
from sol_compiler import SolCompileError, compile_to_src32_asm


def test_compile_arithmetic_sequence():
    asm = compile_to_src32_asm("1 2 add")
    assert ".ORG 0x00000000" in asm
    assert "__solc_entry:" in asm
    assert "LDI R1, 0x1" in asm
    assert "LDI R1, 0x2" in asm
    assert "ADD R1, R1, R2" in asm


def test_compile_labels_and_jumps():
    asm = compile_to_src32_asm("@loop 1 jz @end jmp @loop @end halt")
    assert "loop:" in asm
    assert "end:" in asm
    assert "BEQ R1, R0, end" in asm
    assert "JMP loop" in asm


def test_compile_stack_ops():
    asm = compile_to_src32_asm("1 2 swap dup drop")
    assert "LD R1, [R28 + 0]" in asm
    assert "LD R2, [R28 + 4]" in asm
    assert "ST R1, [R28 + 4]" in asm
    assert "ST R2, [R28 + 0]" in asm


def test_compile_load_store():
    asm = compile_to_src32_asm("1 0 st 0 ld")
    assert "ST R1, [R2 + 0]" in asm
    assert "LD R1, [R1 + 0]" in asm


def test_compile_byte_and_halfword_load_store():
    asm = compile_to_src32_asm("0x1234 0 sth 0 ldh 0xFF 0 stb 0 ldb")
    assert "STH [R2 + 0], R1" in asm
    assert "LDH R1, [R1 + 0]" in asm
    assert "STB [R2 + 0], R1" in asm
    assert "LDB R1, [R1 + 0]" in asm


def test_compile_negative_literal_uses_two_complement_hex():
    asm = compile_to_src32_asm("-1 0 ld")
    assert "LDI R1, 0xFFFFFFFF" in asm


def test_assembler_accepts_signed_ldi():
    assembler = Assembler()
    binary = assembler.assemble("LDI R1, -1")
    assert binary == enc_ldi(1, 0xFFFFFFFF)


def test_compile_unknown_word_error():
    with pytest.raises(SolCompileError, match="unknown word"):
        compile_to_src32_asm("1 foo")


def test_compile_missing_branch_operand_error():
    with pytest.raises(SolCompileError, match="missing label operand"):
        compile_to_src32_asm("jz")


def test_compile_does_not_append_redundant_halt():
    asm = compile_to_src32_asm("halt")
    assert asm.count("HALT") == 1
