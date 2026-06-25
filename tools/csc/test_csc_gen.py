import pytest
from csc_gen import compile_source
import csc_ast as ast


def test_simple_assignment():
    src = "int x = 5;"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: PUSH_CONST 5, STORE_VAR 0
    assert len(code) == 2
    assert code[0] == ('PUSH_CONST', 5)
    assert code[1] == ('STORE_VAR', 0)


def test_binary_op():
    src = "int x = 1 + 2;"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: PUSH_CONST 1, PUSH_CONST 2, ADD, STORE_VAR 0
    assert len(code) == 4
    assert code[0] == ('PUSH_CONST', 1)
    assert code[1] == ('PUSH_CONST', 2)
    assert code[2] == ('ADD',)
    assert code[3] == ('STORE_VAR', 0)


#FIXME: 不完全なテストコード
def test_function_def():
    src = "int add(int a, int b) { return a + b; }"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: function label, param stores, body, return
    # The exact structure depends on implementation
    # Just ensure no crash
    assert len(code) > 0


def test_main_function():
    src = "int main() { return 42; }"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: function label, body, return, CALL main, HALT
    assert len(code) > 0


def test_if_statement():
    src = "int x = 1; if (x == 1) { x = 2; } else { x = 3; }"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: PUSH_CONST 1, STORE_VAR 0, PUSH_CONST 1, CMP ==, JUMP_IF_FALSE placeholder, ...
    assert len(code) > 0


def test_while_loop():
    src = "int i = 0; while (i < 10) { i = i + 1; }"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: PUSH_CONST 0, STORE_VAR 0, ... loop body
    assert len(code) > 0


def test_function_call():
    src = "int foo() { return 1; } int main() { int x = foo(); }"
    emitter = compile_source(src)
    code = emitter.code
    # Expect: function label, body, return, CALL foo, ...
    assert len(code) > 0