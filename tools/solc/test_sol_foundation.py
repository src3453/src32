"""`!abi_compat "Foundation"` (doc/spec_Foundation.md section 13).

The run tests need a native src32_testbench (SRC32_TESTBENCH or target/{debug,release}).
There is no Foundation kernel yet, so they start the generated PROTECTED image from a
tiny S-mode harness that maps its blocks and stack into Extension P regions and
answers SYSCALL with stub results. They prove the U-mode code generation and the
proc_exit convention, not kernel behavior.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE.parent / "asm"))
sys.path.insert(0, str(HERE.parent / "wax"))

from asm import Assembler
from sol_compiler import SolCompileError, compile_to_src32_asm
from sol_vm import SolVM, SolVMError, compile_program
from wax import pack as pack_wax, parse as parse_wax

FND_SOL = ROOT / "sol" / "foundation" / "fnd.sol"


def assemble_protected(asm_text):
    assembler = Assembler(strip=True, sparse=True)
    assembler.assemble(asm_text)
    return assembler


def foundation_wax(source, **kwargs):
    assembler = assemble_protected(compile_to_src32_asm(source, **kwargs))
    data = pack_wax(assembler.image_base, 0, assembler.protected_blocks(), protection=assembler.protection)
    return parse_wax(data)


def test_foundation_emits_protected_layout_after_code():
    image = foundation_wax('!abi_compat "Foundation"\n!var a 5\n!var b\n"hi" drop a >b\n')
    assert image["protection"] == (0x00C00000, 0x00040000)
    assert image["entry"] == 0x00800000
    blocks = image["blocks"]
    assert [b["perm"] for b in blocks] == [5, 1, 3]
    code, rodata, variables = blocks
    assert code["load_address"] == 0x00800000
    assert rodata["load_address"] == (code["load_address"] + len(code["data"]) + 3) & ~3
    assert rodata["data"] == b"hi\x00"
    assert variables["load_address"] == (rodata["load_address"] + 3 + 3) & ~3
    assert variables["data"] == b"\x00" * 8


def test_foundation_entry_keeps_kernel_stack_and_exits_with_proc_exit():
    asm = compile_to_src32_asm('!abi_compat "Foundation"\n1 drop\n')
    assert asm.startswith(".PROTECTED 0xc00000, 0x40000\n.PERM rx\n.ORG 0x800000\n__solc_entry:\n")
    assert "R28, 0x" not in asm.split("__solc_entry:", 1)[1].splitlines()[1]
    assert "LDIH R28" not in asm
    assert "HALT" not in asm
    assert asm.rstrip().endswith("ADDI R1, R0, 0\n    SYSCALL 0x11\n__solc_code_end:")


def test_bare_output_is_unchanged_by_the_directive():
    plain = compile_to_src32_asm('"x" drop 1 2 add')
    assert compile_to_src32_asm('!abi_compat "bare"\n"x" drop 1 2 add') == plain
    assert compile_to_src32_asm('!abi_compat "bare" 1\n"x" drop 1 2 add') == plain
    assert ".PROTECTED" not in plain and "HALT" in plain


def test_foundation_explicit_layout_overrides_defaults_and_is_checked():
    image = foundation_wax(
        '!abi_compat "Foundation"\n!stack_top 0x00700000\n!force_stack_size 0x1000\n!var x 0\n1 >x\n', org=0x00600000)
    assert image["protection"] == (0x00700000, 0x1000)
    assert image["blocks"][0]["load_address"] == 0x00600000
    image = foundation_wax('!abi_compat "Foundation"\n!rodata_base 0x00900000\n!var x\n"s" drop\n')
    assert [b["load_address"] for b in image["blocks"]][1:] == [0x00900000, 0x00900004]
    for source, kwargs, message in (
        ('!abi_compat "Foundation"\n1 drop\n', {"org": 0}, "not inside a Foundation user slot"),
        ('!abi_compat "Foundation"\n!stack_top 0x00D00000\n1 drop\n', {}, "does not fit the user slot"),
        ('!abi_compat "Foundation"\n!force_stack_size 0x00500000\n1 drop\n', {}, "exceeds"),
        ('!abi_compat "Foundation"\n!var_base 0x00800000\n!var x 0\n1 >x\n', {}, "overlaps"),
        ('!abi_compat "Foundation"\n!stack_top 0x00800010\n!force_stack_size 16\n1 drop\n', {}, "overlaps"),
    ):
        with pytest.raises(SolCompileError, match=message):
            compile_to_src32_asm(source, **kwargs)


@pytest.mark.parametrize(
    ("source", "message"),
    [
        ('!abi_compat "foundation"\n', 'unknown ABI for !abi_compat: "foundation"'),
        ('!abi_compat "Foundation" 2\n', "unsupported major version"),
        ("!abi_compat Foundation\n", "must be a string literal"),
        ("!abi_compat\n", "requires an ABI name"),
        ('1 drop\n!abi_compat "Foundation"\n', "must appear at the top level"),
        ('fn f () : retn ;\n!abi_compat "Foundation"\n', "must appear at the top level"),
        ('!namespace n -\n!end\n!abi_compat "Foundation"\n', "must appear at the top level"),
        ('!data\n1\n!end\n!abi_compat "Foundation"\n', "must appear at the top level"),
        ('!asm\nNOP\n!end\n!abi_compat "Foundation"\n', "must appear at the top level"),
        ('!abi_compat "Foundation"\n!abi_compat "bare"\n', "!abi_compat mismatch"),
        ('!abi_compat "Foundation"\nfn irq5 () : retn ;\n', "IRQ handler functions are not allowed"),
        ('!abi_compat "Foundation"\n!asm\n  NOP\nloop: halt ; stop\n!end\n', "privileged instruction HALT"),
        ('!abi_compat "Foundation"\nfn f () :\n!asm\n    IRET\n!end\n;\n', "privileged instruction IRET"),
        ('!abi_compat "Foundation"\n!asm\n    MFSR R1, STATUS\n!end\n', "privileged instruction MFSR"),
        ('!abi_compat "Foundation"\n!asm\n    MTSR EPC, R1\n!end\n', "privileged instruction MTSR"),
    ],
)
def test_abi_compat_errors(source, message):
    with pytest.raises(SolVMError, match=message):
        compile_program(source)


def test_abi_compat_placement_allows_preceding_declarations():
    program = compile_program(
        '!const A 1\n!var v\n!var w 3\n!define M 2\n!undef M\n!force_stack_size 4096\n'
        '!abi_compat "Foundation"\n# comment\nA drop\n')
    assert program.abi == "Foundation"
    # Privileged names inside comments or as operands are not instructions.
    compile_program('!abi_compat "Foundation"\n!asm\n    ; HALT here\n    SYSCALL 0x11\n!end\n')


def test_major_on_the_next_line_is_code():
    assert SolVM().run_source('!abi_compat "bare"\n1\n') == [1]


def test_include_rules_name_the_requesting_file(tmp_path):
    lib = tmp_path / "lib.sol"
    lib.write_text('!abi_compat "Foundation"\nfn f () : 1 ret ;\n', encoding="utf-8")
    plain = tmp_path / "plain.sol"
    plain.write_text("fn g () : 2 ret ;\n", encoding="utf-8")
    main = tmp_path / "main.sol"
    for header in ("", '!abi_compat "bare"\n'):
        source = f'{header}!include "lib.sol"\nf drop\n'
        with pytest.raises(SolVMError, match=f"{re.escape(str(lib))}.*requires \"Foundation\" 1") as caught:
            compile_program(source, source_path=str(main))
        assert str(main) in str(caught.value)
    # The main program may declare the ABI after its includes; undeclared libraries fit either ABI.
    program = compile_program('!include "lib.sol"\n!include "plain.sol"\n!abi_compat "Foundation"\nf g drop drop\n',
                              source_path=str(main))
    assert program.abi == "Foundation"
    assert compile_program('!include "plain.sol"\ng drop\n', source_path=str(main)).abi == "bare"


def test_sol_vm_rejects_foundation_programs():
    with pytest.raises(SolVMError, match="cannot run"):
        SolVM().run_source('!abi_compat "Foundation"\n1\n')


def test_fnd_library_declares_foundation_and_wraps_every_syscall():
    text = FND_SOL.read_text(encoding="utf-8")
    assert '!abi_compat "Foundation" 1' in text.splitlines()[:12]
    program = compile_program(text + "\n", source_path=str(FND_SOL), remove_unused_functions=False)
    assert program.abi == "Foundation"
    numbers = {0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
               0x10, 0x11, 0x12, 0x13, 0x18, 0x20, 0x21, 0x28, 0x30, 0x31, 0x32, 0x3F}
    assert {int(line.split()[1], 16) for line in text.splitlines()
            if line.strip().startswith("SYSCALL")} == numbers
    with pytest.raises(SolVMError, match="main program"):
        compile_program(f'!include "{FND_SOL.as_posix()}"\n0 fnd_proc_exit\n')


def _testbench():
    suffix = ".exe" if os.name == "nt" else ""
    candidates = [os.environ.get("SRC32_TESTBENCH")] + [
        str(ROOT / "target" / profile / f"src32_testbench{suffix}") for profile in ("debug", "release")]
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return candidate
    pytest.skip("src32_testbench is not built; run cargo build --bin src32_testbench or set SRC32_TESTBENCH")


def _harness_asm(image):
    """S-mode loader: map the stack and blocks, enter U mode, emulate a few syscalls."""
    stack_top, stack_size = image["protection"]
    regions = [(stack_top - stack_size, stack_top, 3)] + [
        (b["load_address"], b["load_address"] + len(b["data"]), b["perm"]) for b in image["blocks"]]
    lines = [".ORG 0", "    LDIH R28, 0x000F", "    LDIL R28, 0x0000",
             "    LDIH R13, 0xFFFF", "    LDIL R13, 0x0000"]
    for offset, label in ((4, "fault"), (8, "fault"), (12, "syscall")):
        lines += [f"    LDIH R14, {label}", f"    LDIL R14, {label}", f"    ST R14, [R13 + {offset}]"]
    for index, (start, end, perm) in enumerate(regions):
        for name, value in (("START", start), ("END", end), ("FLAGS", perm)):
            lines += [f"    LDIH R1, 0x{value >> 16:04X}", f"    LDIL R1, 0x{value & 0xFFFF:04X}",
                      f"    MTSR RGN{index}_{name}, R1"]
    entry = image["entry"]
    lines += [f"    LDIH R1, 0x{stack_top >> 16:04X}", f"    LDIL R1, 0x{stack_top & 0xFFFF:04X}", "    MTSR USP, R1",
              f"    LDIH R1, 0x{entry >> 16:04X}", f"    LDIL R1, 0x{entry & 0xFFFF:04X}", "    MTSR EPC, R1",
              "    ADDI R1, R0, 8", "    MTSR STATUS, R1",  # PU=1, PIE=0: user code runs with IRQs masked
              "    ADDI R1, R0, 0",
              "    IRET",
              "fault:", "    MFSR R1, CAUSE", "    HALT",
              # Preserve every register except R1, as the Foundation ABI requires.
              "syscall:", "    ADDI R28, R28, -8", "    ST R13, [R28 + 0]", "    ST R14, [R28 + 4]",
              "    MFSR R13, CAUSE", "    ADDI R14, R0, 0xFF", "    AND R13, R13, R14",
              "    ADDI R14, R0, 0x11", "    BEQ R13, R14, sys_exit",
              "    ADDI R14, R0, 0x0C", "    BEQ R13, R14, sys_zero",
              "    ADDI R14, R0, 0x3F", "    BEQ R13, R14, sys_len",
              "    ADDI R1, R0, -1", "    JMP sys_return",
              "sys_zero:", "    ADDI R1, R0, 0", "    JMP sys_return",
              "sys_len:", "    ADDI R1, R2, 0",
              "sys_return:", "    LD R14, [R28 + 4]", "    LD R13, [R28 + 0]", "    ADDI R28, R28, 8", "    IRET",
              "sys_exit:", "    HALT"]
    return "\n".join(lines) + "\n"


def _run_in_user_mode(tmp_path, source, expected, cpu):
    harness = _testbench()
    image = foundation_wax(source)
    loader = Assembler(strip=True, sparse=True)
    loader.assemble(_harness_asm(image))
    blocks = list(loader.blocks) + [(b["load_address"], b["data"]) for b in image["blocks"]]
    program = tmp_path / "foundation.wax"
    program.write_bytes(pack_wax(0, 0, blocks))
    result = subprocess.run([harness, str(program), str(expected), "--cpu", cpu], cwd=tmp_path,
                            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    assert result.returncode == 0, result.stdout + result.stderr


@pytest.mark.parametrize("cpu", ["scalar", "pipeline"])
def test_fnd_wrappers_run_in_user_mode(tmp_path, cpu):
    source = (
        '!abi_compat "Foundation"\n'
        f'!include "{FND_SOL.as_posix()}"\n'
        '!var total 39\n'
        'fn bump (n) : total n add >total retn ;\n'
        # sleep succeeds (0 = sol true), debug_write returns its length, an unknown
        # syscall result is negative, and globals/strings live in RW/R blocks.
        '10 fnd_sleep if 1 bump end\n'
        '"ab" 2 fnd_debug_write 2 eq if 1 bump end\n'
        'fnd_port_create 0 lt if 1 bump end\n'
        'total fnd_proc_exit\n'
        '99 >total\n'
    )
    _run_in_user_mode(tmp_path, source, 42, cpu)


def test_program_end_is_proc_exit_zero(tmp_path):
    _run_in_user_mode(tmp_path, '!abi_compat "Foundation"\nfn f (x) : x 1 add ret ;\n5 f drop\n', 0, "scalar")
