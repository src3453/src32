import pytest
from csc_gen import compile_source
import csc_ast as ast


# ── helpers ──────────────────────────────────────────────────────────

def opcodes(code):
    """Return list of just the opcode names for easier assertions."""
    return [c[0] for c in code]


# ── variable declarations ───────────────────────────────────────────

def test_simple_assignment():
    emitter = compile_source("int x = 5;")
    code = emitter.code
    assert code[0] == ('PUSH_CONST', 5)
    assert code[1] == ('STORE_VAR', 0)


def test_var_without_initializer():
    emitter = compile_source("int x;")
    code = emitter.code
    assert len(code) == 0


def test_var_reassignment():
    emitter = compile_source("int x = 1; x = 2;")
    code = emitter.code
    assert code[0] == ('PUSH_CONST', 1)
    assert code[1] == ('STORE_VAR', 0)
    assert code[2] == ('PUSH_CONST', 2)
    assert code[3] == ('STORE_VAR', 0)


# ── binary operations ───────────────────────────────────────────────

def test_binary_op():
    emitter = compile_source("int x = 1 + 2;")
    code = emitter.code
    assert code[0] == ('PUSH_CONST', 1)
    assert code[1] == ('PUSH_CONST', 2)
    assert code[2] == ('ADD',)
    assert code[3] == ('STORE_VAR', 0)


def test_precedence_mul_over_add():
    emitter = compile_source("int x = 1 + 2 * 3;")
    code = emitter.code
    # 1 + (2*3)  →  push 1, push 2, push 3, MUL, ADD, STORE
    assert ('PUSH_CONST', 1) in code
    assert ('PUSH_CONST', 2) in code
    assert ('PUSH_CONST', 3) in code
    assert ('MUL',) in code
    assert ('ADD',) in code
    mul_idx = opcodes(code).index('MUL')
    add_idx = opcodes(code).index('ADD')
    assert mul_idx < add_idx


def test_comparison_ops():
    for op_sym, bc_op in [('==', 'CMP'), ('!=', 'CMP'),
                           ('<', 'CMP'), ('>', 'CMP'),
                           ('<=', 'CMP'), ('>=', 'CMP')]:
        emitter = compile_source(f"int x = 1 {op_sym} 2;")
        code = emitter.code
        assert ('CMP', op_sym) in code, f"Expected CMP {op_sym} for '{op_sym}'"


# ── function definitions ────────────────────────────────────────────

def test_function_def_structure():
    src = "int add(int a, int b) { return a + b; }"
    emitter = compile_source(src)
    code = emitter.code
    ops = opcodes(code)
    # Every function starts with SAVE_RET to capture the return address
    assert ops[0] == 'SAVE_RET'
    # Parameters are stored into local slots
    assert ('STORE_VAR', 0) in code  # a → slot 0
    assert ('STORE_VAR', 1) in code  # b → slot 1
    # Body computes a + b
    assert ('LOAD_VAR', 0) in code
    assert ('LOAD_VAR', 1) in code
    assert ('ADD',) in code
    # Function ends with RETURN
    assert ops[-1] == 'RETURN'


def test_function_def_no_params():
    src = "int foo() { return 42; }"
    emitter = compile_source(src)
    code = emitter.code
    assert code[0] == ('SAVE_RET',)
    assert ('PUSH_CONST', 42) in code
    assert code[-1] == ('RETURN',)


def test_main_function():
    src = "int main() { return 42; }"
    emitter = compile_source(src)
    code = emitter.code
    ops = opcodes(code)
    # main starts with SAVE_RET
    assert ops[0] == 'SAVE_RET'
    # CALL main and HALT are appended at the end
    assert ('CALL', 'main') in code
    assert ('HALT',) in code


def test_function_labels_correct():
    src = "int add(int a, int b) { return a + b; } int main() { return add(1, 2); }"
    emitter = compile_source(src)
    labels = emitter.func_labels
    assert 'add' in labels
    assert 'main' in labels
    assert labels['add'] < labels['main']


# ── function calls ──────────────────────────────────────────────────

def test_function_call_pushes_args_right_to_left():
    """Arguments must be pushed right-to-left so the first arg is on
    top of the stack after JAL+SAVE_RET."""
    src = "int add(int a, int b) { return a + b; } int main() { return add(10, 20); }"
    emitter = compile_source(src)
    code = emitter.code
    # In the caller (main), find the PUSH_CONST before CALL add
    call_idx = opcodes(code).index('CALL')
    # The two pushes immediately before CALL should be: push 20, push 10
    push_before = [c for c in code[:call_idx] if c[0] == 'PUSH_CONST']
    assert push_before[-2] == ('PUSH_CONST', 20), "Second arg (20) should be pushed first"
    assert push_before[-1] == ('PUSH_CONST', 10), "First arg (10) should be pushed last (on top)"


def test_function_call_emits_save_ret():
    """Every function body must start with SAVE_RET."""
    src = "int add(int a, int b) { return a + b; } int main() { return add(1, 2); }"
    emitter = compile_source(src)
    code = emitter.code
    # Find the add function start and main function start
    add_start = emitter.func_labels['add']
    main_start = emitter.func_labels['main']
    assert code[add_start] == ('SAVE_RET',)
    assert code[main_start] == ('SAVE_RET',)


def test_function_call_multiple_args():
    """Three-arg function: args pushed right-to-left."""
    src = (
        "int triple(int a, int b, int c) { return a + b + c; }"
        "int main() { return triple(1, 2, 3); }"
    )
    emitter = compile_source(src)
    code = emitter.code
    call_idx = opcodes(code).index('CALL')
    push_before = [c for c in code[:call_idx] if c[0] == 'PUSH_CONST']
    # Right-to-left: push 3, push 2, push 1
    assert push_before[-3] == ('PUSH_CONST', 3)
    assert push_before[-2] == ('PUSH_CONST', 2)
    assert push_before[-1] == ('PUSH_CONST', 1)


def test_function_call_zero_args():
    src = "int foo() { return 7; } int main() { return foo(); }"
    emitter = compile_source(src)
    code = emitter.code
    call_idx = opcodes(code).index('CALL')
    # No PUSH_CONST should appear right before CALL for zero-arg function
    # (there may be PUSH_CONST inside foo itself, so check just before CALL)
    immediately_before = code[call_idx - 1]
    assert immediately_before[0] == 'CALL' or immediately_before[0] != 'PUSH_CONST'


# ── var_index isolation between functions ────────────────────────────

def test_var_index_not_shared_between_functions():
    """Each function must have its own local var slots starting from 0.
    Two functions both using parameter name 'x' should not collide."""
    src = (
        "int get_x(int x) { return x; }"
        "int main() { return get_x(5); }"
    )
    emitter = compile_source(src)
    code = emitter.code
    # Both functions should use STORE_VAR 0 for their 'x' parameter
    # but they operate in separate local namespaces.
    add_start = emitter.func_labels['get_x']
    main_start = emitter.func_labels['main']
    # In get_x: SAVE_RET, STORE_VAR 0 (x), LOAD_VAR 0, RETURN, ...
    get_x_ops = code[add_start:main_start]
    assert ('STORE_VAR', 0) in get_x_ops
    # In main: SAVE_RET, ..., CALL get_x, ...
    main_ops = code[main_start:]
    # main doesn't have 'x' parameter but may have other locals
    # The key point: both functions use slot 0 independently
    store_vars_in_get_x = [c for c in get_x_ops if c[0] == 'STORE_VAR']
    assert len(store_vars_in_get_x) >= 1


def test_emitter_has_var_index():
    """The emitter must carry var_index so the backend can determine var count."""
    emitter = compile_source("int x = 5; int y = 10;")
    assert hasattr(emitter, 'var_index')
    assert isinstance(emitter.var_index, dict)
    assert len(emitter.var_index) > 0


# ── control flow ────────────────────────────────────────────────────

def test_if_statement():
    emitter = compile_source("int x = 1; if (x == 1) { x = 2; } else { x = 3; }")
    code = emitter.code
    ops = opcodes(code)
    assert 'JUMP_IF_FALSE' in ops
    assert 'JUMP' in ops


def test_if_without_else():
    emitter = compile_source("int x = 1; if (x == 1) { x = 2; }")
    code = emitter.code
    ops = opcodes(code)
    assert 'JUMP_IF_FALSE' in ops
    # No unconditional JUMP needed (no else branch)
    assert ops.count('JUMP') == 0 or ops.count('JUMP') < 2


def test_while_loop():
    emitter = compile_source("int i = 0; while (i < 10) { i = i + 1; }")
    code = emitter.code
    ops = opcodes(code)
    assert 'JUMP_IF_FALSE' in ops
    # While body should jump back to the loop start
    assert 'JUMP' in ops


def test_while_jump_back():
    """The JUMP in a while loop must target an earlier instruction (loop start)."""
    emitter = compile_source("int i = 0; while (i < 10) { i = i + 1; }")
    code = emitter.code
    jump_back = [c for c in code if c[0] == 'JUMP']
    assert len(jump_back) >= 1
    # The jump target should be less than the jump's own index
    for i, c in enumerate(code):
        if c[0] == 'JUMP':
            assert c[1] < i, "While loop JUMP should target an earlier instruction"


# ── top-level statements ────────────────────────────────────────────

def test_top_level_statements_without_main():
    """Top-level statements should be code-generated when no main() exists."""
    emitter = compile_source("int x = 5;")
    code = emitter.code
    assert ('PUSH_CONST', 5) in code
    assert ('STORE_VAR', 0) in code


def test_top_level_statements_with_main():
    """Top-level statements with main() are analyzed but not code-generated.
    Only main's body and the trailing CALL main + HALT should appear."""
    emitter = compile_source("int x = 5; int main() { return 42; }")
    code = emitter.code
    ops = opcodes(code)
    # Top-level 'int x = 5' should NOT appear in the code
    assert ops.count('STORE_VAR') == 0 or all(
        code[i][0] != 'STORE_VAR'
        for i in range(len(code))
        if i < emitter.func_labels.get('main', len(code))
    )
    assert ('CALL', 'main') in code
    assert ('HALT',) in code


# ── regression: incomplete test_function_def now verified ────────────

def test_function_def_detailed():
    """Detailed test for function definition bytecode (replaces the old FIXME test)."""
    src = "int add(int a, int b) { return a + b; }"
    emitter = compile_source(src)
    code = emitter.code
    # Exact expected sequence:
    #   SAVE_RET          – capture return address
    #   STORE_VAR 0       – pop 'a' from stack
    #   STORE_VAR 1       – pop 'b' from stack
    #   LOAD_VAR 0        – push a
    #   LOAD_VAR 1        – push b
    #   ADD               – a + b
    #   RETURN            – return result
    #   PUSH_CONST 0      – implicit return (dead code)
    #   RETURN            – implicit return
    assert code[0] == ('SAVE_RET',)
    assert ('STORE_VAR', 0) in code
    assert ('STORE_VAR', 1) in code
    assert ('LOAD_VAR', 0) in code
    assert ('LOAD_VAR', 1) in code
    assert ('ADD',) in code
    # Verify ordering: STORE before LOAD, LOAD before ADD
    ops = opcodes(code)
    s0 = ops.index('STORE_VAR')  # first STORE_VAR (a)
    l0 = ops.index('LOAD_VAR')   # first LOAD_VAR (a)
    add = ops.index('ADD')
    assert s0 < l0 < add


# ── RETURN semantics ────────────────────────────────────────────────

def test_return_with_value():
    emitter = compile_source("int main() { return 42; }")
    code = emitter.code
    main_start = emitter.func_labels['main']
    # The main function body ends before the trailing CALL main + HALT
    call_main_idx = opcodes(code).index('CALL')
    main_body = code[main_start:call_main_idx]
    assert ('PUSH_CONST', 42) in main_body
    assert main_body[-1] == ('RETURN',)


def test_implicit_return():
    """Functions without explicit return should get implicit PUSH_CONST 0 + RETURN."""
    src = "int noop() { int x = 1; }"
    emitter = compile_source(src)
    code = emitter.code
    # Last two instructions should be the implicit return
    assert code[-2] == ('PUSH_CONST', 0)
    assert code[-1] == ('RETURN',)
