"""Standard-library integration tests for the LLVM -> SRC32 pipeline.

This runner intentionally uses unittest so it works on a clean checkout without
requiring pytest.  It validates parsing, lowering, assembly, and (when the Rust
harness is built) CPU execution.
"""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "csc"))
sys.path.insert(0, str(ROOT / "tools" / "asm"))
from llvm_ir_parser import compile_llvm_ir
from backend_src32 import emit_src32
from asm import Assembler


class PipelineTests(unittest.TestCase):
    def compile(self, body):
        emitter = compile_llvm_ir(body)
        assembly = emit_src32(emitter)
        binary = Assembler().assemble(assembly, base_dir=str(ROOT))
        return assembly, binary

    def test_arithmetic_assembles(self):
        assembly, binary = self.compile("""
        define i32 @main() {
          %x = add i32 19, 23
          ret i32 %x
        }
        """)
        self.assertIn("ADD R1, R1, R2", assembly)
        self.assertGreater(len(binary), 0)

    def test_conditional_assembles(self):
        _, binary = self.compile("""
        define i32 @main() {
        entry:
          %c = icmp eq i32 1, 1
          br i1 %c, label %yes, label %no
        yes:
          ret i32 7
        no:
          ret i32 0
        }
        """)
        self.assertGreater(len(binary), 0)

    def test_cli_and_cpu_harness(self):
        harness = ROOT / "target" / "debug" / "src32_testbench"
        if not harness.exists():
            self.skipTest("cargo test/build has not built src32_testbench")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            ll = td / "main.ll"
            asm = td / "main.a"
            binary = td / "main.bin"
            ll.write_text("define i32 @main() { ret i32 42 }\n")
            result = subprocess.run(
                [sys.executable, str(ROOT / "tools/csc/csc.py"), str(ll), "--from-llvm", "-o", str(asm)],
                cwd=ROOT, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            result = subprocess.run(
                [sys.executable, str(ROOT / "tools/csc/csc.py"), str(ll), "--from-llvm", "--assemble", "-o", str(binary)],
                cwd=ROOT, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            result = subprocess.run([str(harness), str(binary), "42"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
