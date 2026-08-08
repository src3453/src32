import pytest

from sol_vm import STRING_POOL_BASE, SolVM, SolVMError


def test_arithmetic_basic():
    vm = SolVM()
    stack = vm.run_source("1 2 add 4 mul 2 div")
    assert stack == [6]


def test_mod_and_neg_ops():
    vm = SolVM()
    assert vm.run_source("7 4 mod") == [3]

    vm = SolVM()
    assert vm.run_source("7 neg") == [-7]


def test_shift_ops():
    vm = SolVM()
    stack = vm.run_source("8 1 shl")
    assert stack == [16]

    vm = SolVM()
    stack = vm.run_source("16 2 shr")
    assert stack == [4]

    vm = SolVM()
    stack = vm.run_source("-8 1 shr")
    assert stack == [-4]


def test_trace_records_step_by_step_execution():
    vm = SolVM()
    vm.set_trace(True)

    stack = vm.run_source("1 2 add")

    assert stack == [3]
    assert vm.trace
    assert any("push" in entry for entry in vm.trace)
    assert any("add" in entry for entry in vm.trace)


def test_stack_ops():
    vm = SolVM()
    stack = vm.run_source("1 2 swap dup add")
    assert stack == [2, 2]


def test_bitwise_ops():
    vm = SolVM()
    assert vm.run_source("6 3 and") == [2]

    vm = SolVM()
    assert vm.run_source("6 3 or") == [7]

    vm = SolVM()
    assert vm.run_source("6 3 xor") == [5]

    vm = SolVM()
    assert vm.run_source("0 not") == [-1]


def test_extended_stack_ops():
    vm = SolVM()
    assert vm.run_source("1 2 over") == [1, 2, 1]

    vm = SolVM()
    assert vm.run_source("1 2 3 rot") == [2, 3, 1]

    vm = SolVM()
    assert vm.run_source("1 2 nip") == [2]

    vm = SolVM()
    assert vm.run_source("1 2 tuck") == [2, 1, 2]


def test_structured_if_else():
    vm = SolVM()
    assert vm.run_source("0 if 1 else 2 end") == [1]

    vm = SolVM()
    assert vm.run_source("1 if 1 else 2 end") == [2]


def test_structured_while_consumes_condition():
    vm = SolVM()
    assert vm.run_source("0 while 1 end") == [0]


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


def test_string_literal_allows_custom_read_only_data_base():
    vm = SolVM()
    stack = vm.run_source('"hi"', read_only_data_base=0x30000)
    assert stack == [0x30000]
    assert vm.memory[0x30000] == ord("h")
    assert vm.memory[0x30001] == ord("i")
    assert vm.memory[0x30002] == 0


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


def test_hidden_label_error():
    vm = SolVM()
    with pytest.raises(SolVMError, match="hidden"):
        vm.run_source("@missing")


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
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("5 3 eq")
    assert stack == [1]


def test_comparison_neq():
    vm = SolVM()
    stack = vm.run_source("5 3 neq")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("5 5 neq")
    assert stack == [1]


def test_comparison_lt():
    vm = SolVM()
    stack = vm.run_source("3 5 lt")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("5 3 lt")
    assert stack == [1]


def test_comparison_gt():
    vm = SolVM()
    stack = vm.run_source("5 3 gt")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("3 5 gt")
    assert stack == [1]


def test_comparison_le():
    vm = SolVM()
    stack = vm.run_source("3 5 le")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("5 5 le")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("5 3 le")
    assert stack == [1]


def test_comparison_ge():
    vm = SolVM()
    stack = vm.run_source("5 3 ge")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("5 5 ge")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("3 5 ge")
    assert stack == [1]


def test_comparison_with_negative_numbers():
    vm = SolVM()
    stack = vm.run_source("-5 -3 lt")
    assert stack == [0]

    vm = SolVM()
    stack = vm.run_source("-3 -5 lt")
    assert stack == [1]


def test_sgn():
    vm = SolVM()
    assert vm.run_source("-5 sgn") == [1]

    vm = SolVM()
    assert vm.run_source("0 sgn") == [0]

    vm = SolVM()
    assert vm.run_source("7 sgn") == [0]


def test_stacksize():
    vm = SolVM()
    assert vm.run_source("stacksize") == [0x00100000]


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


def test_nested_function_call_preserves_caller_stack_below_args():
    vm = SolVM()
    src = """
fn inc (x) :
    x 1 add
    ret
;

fn outer (x) :
    1 2 x inc
    add
    ret
;

3 outer
"""
    stack = vm.run_source(src)
    assert stack == [6]


def test_recursive_factorial():
    vm = SolVM()
    src = """
fn fact (n) :
    n 1 le if
        1
    else
        n 1 sub
        fact
        n mul
    end
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
    # call preserves preexisting stack values below the consumed argument
    assert stack == [1, 4]


def test_local_shadows_global_for_store():
    vm = SolVM()
    src = """
!var x 1

fn update (v) :
    local x 0
    v >x
    x
    ret
;

2 update
x
"""
    stack = vm.run_source(src)
    assert stack == [2, 1]
