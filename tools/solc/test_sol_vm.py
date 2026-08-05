import pytest

from sol_vm import STRING_POOL_BASE, SolVM, SolVMError


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


def test_string_literal_pushes_pointer_and_loads_read_only_bytes():
    vm = SolVM()
    stack = vm.run_source('"hi"')
    assert stack == [STRING_POOL_BASE]
    assert vm.memory[STRING_POOL_BASE] == ord("h")
    assert vm.memory[STRING_POOL_BASE + 1] == ord("i")
    assert vm.memory[STRING_POOL_BASE + 2] == 0


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


def test_comparison_eq():
    vm = SolVM()
    stack = vm.run_source("5 5 eq")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("5 3 eq")
    assert stack == [0]


def test_comparison_neq():
    vm = SolVM()
    stack = vm.run_source("5 3 neq")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("5 5 neq")
    assert stack == [0]


def test_comparison_lt():
    vm = SolVM()
    stack = vm.run_source("3 5 lt")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("5 3 lt")
    assert stack == [0]


def test_comparison_gt():
    vm = SolVM()
    stack = vm.run_source("5 3 gt")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("3 5 gt")
    assert stack == [0]


def test_comparison_le():
    vm = SolVM()
    stack = vm.run_source("3 5 le")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("5 5 le")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("5 3 le")
    assert stack == [0]


def test_comparison_ge():
    vm = SolVM()
    stack = vm.run_source("5 3 ge")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("5 5 ge")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("3 5 ge")
    assert stack == [0]


def test_comparison_with_negative_numbers():
    vm = SolVM()
    stack = vm.run_source("-5 -3 lt")
    assert stack == [1]

    vm = SolVM()
    stack = vm.run_source("-3 -5 lt")
    assert stack == [0]


def test_simple_function_call():
    vm = SolVM()
    src = """
fn add_two (a b) :
    a b add
    ret
;

1 2 add_two
"""
    stack = vm.run_source(src)
    assert stack == [3]


def test_nested_function_calls():
    vm = SolVM()
    src = """
fn inc (x) :
    x 1 add
    ret
;

fn add_and_inc (a b) :
    a b add
    inc
    ret
;

1 2 add_and_inc
"""
    stack = vm.run_source(src)
    assert stack == [4]


def test_recursive_factorial():
    vm = SolVM()
    src = """
fn fact (n) :
    n 1 le        # if n <= 1
    jz @recurse
    1             # base case: return 1
    ret
@recurse
    n 1 sub
    fact
    n mul
    ret
;

5 fact
"""
    stack = vm.run_source(src)
    assert stack == [120]


def test_local_lifetime_non_reentrant():
    vm = SolVM()
    src = """
fn use_local (a) :
    local t 0
    a >t
    t 2 mul
    ret
;

1 2 use_local  # call use_local with 2
"""
    stack = vm.run_source(src)
    # use_local returns 4
    assert stack == [4]
