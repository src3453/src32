import pytest

from sol_vm import SolVM, SolVMError


def test_arithmetic_basic():
    vm = SolVM()
    stack = vm.run_source("1 2 add 4 mul 2 div")
    assert stack == [6]


def test_stack_ops():
    vm = SolVM()
    stack = vm.run_source("1 2 swap dup add")
    assert stack == [2, 2]


def test_label_and_jumps():
    vm = SolVM()
    stack = vm.run_source("3 @loop dup jz @end 1 sub jmp @loop @end drop")
    assert stack == []


def test_comments_and_semicolon():
    vm = SolVM()
    stack = vm.run_source("1 2 add; # comment\n3 add")
    assert stack == [6]


def test_unsigned_and_hex():
    vm = SolVM()
    stack = vm.run_source("0xFFFFFFFF 1 add")
    assert stack == [0]


def test_load_store():
    vm = SolVM()
    stack = vm.run_source("1 0 st 0 ld")
    assert stack == [1]


def test_byte_and_halfword_load_store():
    vm = SolVM()
    stack = vm.run_source("0x1234 0 sth 0 ldh")
    assert stack == [0x1234]

    vm = SolVM()
    stack = vm.run_source("0xFF 0 stb 0 ldb")
    assert stack == [0xFF]


def test_unknown_word_error():
    vm = SolVM()
    with pytest.raises(SolVMError, match="unknown word"):
        vm.run_source("1 nope")


def test_undefined_label_error():
    vm = SolVM()
    with pytest.raises(SolVMError, match="undefined label"):
        vm.run_source("jmp @missing")


def test_stack_underflow_error():
    vm = SolVM()
    with pytest.raises(SolVMError, match="stack underflow"):
        vm.run_source("add")


def test_division_by_zero_error():
    vm = SolVM()
    with pytest.raises(SolVMError, match="division by zero"):
        vm.run_source("1 0 div")
