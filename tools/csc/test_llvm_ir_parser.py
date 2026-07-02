"""
test_llvm_ir_parser.py - Unit tests for the LLVM IR parser frontend.
"""
import pytest
from llvm_ir_parser import (
    tokenize_ll,
    parse_llvm_ir,
    compile_llvm_ir,
    LLVMModule,
    LLVMFunction,
)
from csc_gen import CodeEmitter


# ---------------------------------------------------------------------------
# Tokenizer tests
# ---------------------------------------------------------------------------

class TestTokenizer:
    def test_empty(self):
        assert tokenize_ll("") == []

    def test_comment_only(self):
        assert tokenize_ll("; this is a comment\n") == []

    def test_simple_ret(self):
        tokens = tokenize_ll("ret i32 42")
        assert tokens == ["ret", "i32", "42"]

    def test_string_constant(self):
        tokens = tokenize_ll('@g = constant [3 x i8] c"abc"')
        assert "@g" in tokens
        assert "constant" in tokens

    def test_negative_integer(self):
        tokens = tokenize_ll("ret i32 -5")
        assert tokens == ["ret", "i32", "-5"]

    def test_identifier_with_percent(self):
        tokens = tokenize_ll("%x = add i32 1, 2")
        assert "%x" in tokens

    def test_identifier_with_at(self):
        tokens = tokenize_ll("define i32 @main() {")
        assert "@main" in tokens


# ---------------------------------------------------------------------------
# Parser tests
# ---------------------------------------------------------------------------

class TestParser:
    def test_simple_function(self):
        src = """
define i32 @main() {
  ret i32 42
}
"""
        module = parse_llvm_ir(src)
        assert "main" in module.functions
        func = module.functions["main"]
        assert len(func.params) == 0
        assert "entry" in func.blocks
        instrs = func.blocks["entry"]
        assert len(instrs) == 1
        assert instrs[0]["op"] == "ret"
        assert instrs[0]["value"] == 42

    def test_function_with_params(self):
        src = """
define i32 @add(i32 %a, i32 %b) {
  %result = add i32 %a, %b
  ret i32 %result
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["add"]
        assert len(func.params) == 2
        assert func.params[0] == ("i32", "%a")
        assert func.params[1] == ("i32", "%b")

    def test_add_instruction(self):
        src = """
define i32 @foo() {
  %x = add i32 1, 2
  ret i32 %x
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "add"
        assert entry[0]["a"] == 1
        assert entry[0]["b"] == 2

    def test_sub_instruction(self):
        src = """
define i32 @foo() {
  %x = sub i32 10, 3
  ret i32 %x
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "sub"
        assert entry[0]["a"] == 10
        assert entry[0]["b"] == 3

    def test_mul_instruction(self):
        src = """
define i32 @foo() {
  %x = mul i32 3, 4
  ret i32 %x
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "mul"

    def test_sdiv_instruction(self):
        src = """
define i32 @foo() {
  %x = sdiv i32 10, 2
  ret i32 %x
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "sdiv"

    def test_icmp_eq(self):
        src = """
define i32 @foo() {
  %c = icmp eq i32 1, 2
  ret i32 %c
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "icmp"
        assert entry[0]["cond"] == "eq"

    def test_icmp_slt(self):
        src = """
define i32 @foo() {
  %c = icmp slt i32 1, 2
  ret i32 %c
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["cond"] == "slt"

    def test_alloca(self):
        src = """
define i32 @foo() {
  %x = alloca i32
  ret i32 0
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "alloca"

    def test_store_and_load(self):
        src = """
define i32 @foo() {
  %x = alloca i32
  store i32 42, i32* %x
  %v = load i32, i32* %x
  ret i32 %v
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[0]["op"] == "alloca"
        assert entry[1]["op"] == "store"
        assert entry[1]["value"] == 42
        assert entry[2]["op"] == "load"
        assert entry[3]["op"] == "ret"

    def test_branch_unconditional(self):
        src = """
define i32 @foo() {
entry:
  br label %exit
exit:
  ret i32 1
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        assert "entry" in func.blocks
        assert "exit" in func.blocks
        assert func.blocks["entry"][0]["op"] == "br"
        assert func.blocks["entry"][0]["dest"] == "exit"

    def test_branch_conditional(self):
        src = """
define i32 @foo() {
entry:
  %c = icmp eq i32 1, 1
  br i1 %c, label %then, label %else
then:
  ret i32 1
else:
  ret i32 0
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        entry = func.blocks["entry"]
        assert entry[1]["op"] == "br_cond"
        assert entry[1]["true"] == "then"
        assert entry[1]["false"] == "else"

    def test_call_instruction(self):
        src = """
define i32 @bar() {
  ret i32 10
}

define i32 @foo() {
  %r = call i32 @bar()
  ret i32 %r
}
"""
        module = parse_llvm_ir(src)
        foo = module.functions["foo"]
        entry = foo.blocks["entry"]
        assert entry[0]["op"] == "call"
        assert entry[0]["name"] == "bar"

    def test_phi_node(self):
        src = """
define i32 @foo() {
entry:
  br label %loop
loop:
  %i = phi i32 [ 0, %entry ], [ %next, %loop ]
  %next = add i32 %i, 1
  ret i32 %i
}
"""
        module = parse_llvm_ir(src)
        func = module.functions["foo"]
        loop_block = func.blocks["loop"]
        assert loop_block[0]["op"] == "phi"
        assert len(loop_block[0]["entries"]) == 2

    def test_multiple_functions(self):
        src = """
define i32 @foo() {
  ret i32 1
}

define i32 @bar() {
  ret i32 2
}
"""
        module = parse_llvm_ir(src)
        assert "foo" in module.functions
        assert "bar" in module.functions

    def test_skip_declare(self):
        src = """
declare void @puts(i8*)

define i32 @main() {
  ret i32 0
}
"""
        module = parse_llvm_ir(src)
        assert "main" in module.functions
        assert "puts" not in module.functions


# ---------------------------------------------------------------------------
# Code generation tests (LLVM IR -> CodeEmitter)
# ---------------------------------------------------------------------------

class TestCodegen:
    def test_simple_return(self):
        src = """
define i32 @main() {
  ret i32 42
}
"""
        emitter = compile_llvm_ir(src)
        assert isinstance(emitter, CodeEmitter)
        # Should contain PUSH_CONST 42 and RETURN
        ops = [instr[0] for instr in emitter.code]
        assert "PUSH_CONST" in ops
        assert "RETURN" in ops

    def test_add_codegen(self):
        src = """
define i32 @main() {
  %x = add i32 10, 20
  ret i32 %x
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "PUSH_CONST" in ops
        assert "ADD" in ops
        assert "RETURN" in ops

    def test_sub_codegen(self):
        src = """
define i32 @main() {
  %x = sub i32 100, 30
  ret i32 %x
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "SUB" in ops

    def test_mul_codegen(self):
        src = """
define i32 @main() {
  %x = mul i32 6, 7
  ret i32 %x
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "MUL" in ops

    def test_div_codegen(self):
        src = """
define i32 @main() {
  %x = sdiv i32 20, 4
  ret i32 %x
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "DIV" in ops

    def test_function_call_codegen(self):
        src = """
define i32 @helper() {
  ret i32 10
}

define i32 @main() {
  %r = call i32 @helper()
  ret i32 %r
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "CALL" in ops
        assert "helper" in [instr[1] for instr in emitter.code if instr[0] == "CALL"]

    def test_param_binding(self):
        src = """
define i32 @add(i32 %a, i32 %b) {
  %r = add i32 %a, %b
  ret i32 %r
}
"""
        emitter = compile_llvm_ir(src)
        # Should have STORE_VAR for params
        ops = [instr[0] for instr in emitter.code]
        assert "STORE_VAR" in ops
        assert "LOAD_VAR" in ops

    def test_alloca_load_store(self):
        src = """
define i32 @main() {
  %x = alloca i32
  store i32 42, i32* %x
  %v = load i32, i32* %x
  ret i32 %v
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "STORE_VAR" in ops
        assert "LOAD_VAR" in ops

    def test_conditional_branch(self):
        src = """
define i32 @main() {
entry:
  %c = icmp eq i32 1, 1
  br i1 %c, label %then, label %else
then:
  ret i32 1
else:
  ret i32 0
}
"""
        emitter = compile_llvm_ir(src)
        ops = [instr[0] for instr in emitter.code]
        assert "CMP" in ops
        assert "JUMP_IF_FALSE" in ops
        assert "JUMP" in ops

    def test_func_labels_set(self):
        src = """
define i32 @main() {
  ret i32 0
}
"""
        emitter = compile_llvm_ir(src)
        assert hasattr(emitter, "func_labels")
        assert "main" in emitter.func_labels


# ---------------------------------------------------------------------------
# End-to-end tests (LLVM IR -> SRC32 assembly)
# ---------------------------------------------------------------------------

class TestEndToEnd:
    def test_return_42(self):
        from backend_src32 import emit_src32
        src = """
define i32 @main() {
  ret i32 42
}
"""
        emitter = compile_llvm_ir(src)
        asm = emit_src32(emitter)
        assert "LDI R1" in asm
        assert "HALT" in asm

    def test_add_e2e(self):
        from backend_src32 import emit_src32
        src = """
define i32 @main() {
  %x = add i32 10, 20
  ret i32 %x
}
"""
        emitter = compile_llvm_ir(src)
        asm = emit_src32(emitter)
        assert "ADD R1, R1, R2" in asm

    def test_call_e2e(self):
        from backend_src32 import emit_src32
        src = """
define i32 @helper() {
  ret i32 10
}

define i32 @main() {
  %r = call i32 @helper()
  ret i32 %r
}
"""
        emitter = compile_llvm_ir(src)
        asm = emit_src32(emitter)
        assert "JAL" in asm
        assert "helper" in asm


# ---------------------------------------------------------------------------
# CLI integration tests
# ---------------------------------------------------------------------------

class TestCLI:
    def test_csc_from_llvm_flag(self, tmp_path):
        """Test that csc.py accepts --from-llvm flag."""
        import subprocess
        import sys

        ll_file = tmp_path / "test.ll"
        ll_file.write_text(
            "define i32 @main() {\n  ret i32 42\n}\n",
            encoding="utf-8",
        )
        out_file = tmp_path / "test.s"

        result = subprocess.run(
            [
                sys.executable,
                "tools/csc/csc.py",
                str(ll_file),
                "--from-llvm",
                "-o",
                str(out_file),
            ],
            capture_output=True,
            text=True,
            cwd=".",
        )
        assert result.returncode == 0, f"stderr: {result.stderr}"
        assert out_file.exists()
        content = out_file.read_text(encoding="utf-8")
        assert "HALT" in content

    def test_csc_dump_bc(self, tmp_path):
        """Test --dump-bc with LLVM IR input."""
        import subprocess
        import sys

        ll_file = tmp_path / "test.ll"
        ll_file.write_text(
            "define i32 @main() {\n  ret i32 7\n}\n",
            encoding="utf-8",
        )

        result = subprocess.run(
            [
                sys.executable,
                "tools/csc/csc.py",
                str(ll_file),
                "--from-llvm",
                "--dump-bc",
            ],
            capture_output=True,
            text=True,
            cwd=".",
        )
        assert result.returncode == 0, f"stderr: {result.stderr}"
        assert "PUSH_CONST" in result.stdout
