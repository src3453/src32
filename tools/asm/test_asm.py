import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from asm import Assembler


def assemble_hex(source: str) -> str:
    return Assembler().assemble(source).hex()


def test_jump_family_accepts_numeric_absolute_targets():
    assert assemble_hex("JMP 8\nNOP\ntarget:\nHALT") == assemble_hex("JMP target\nNOP\ntarget:\nHALT")
    assert assemble_hex("JAL 8\nNOP\ntarget:\nHALT") == assemble_hex("JAL target\nNOP\ntarget:\nHALT")
    assert assemble_hex("JMPS 8\nNOP\ntarget:\nHALT") == assemble_hex("JMPS target\nNOP\ntarget:\nHALT")
    assert assemble_hex("JALS 8\nNOP\ntarget:\nHALT") == assemble_hex("JALS target\nNOP\ntarget:\nHALT")
    assert assemble_hex("S.JAL 4\nS.RET\ntarget:\nS.RET") == assemble_hex("S.JAL target\nS.RET\ntarget:\nS.RET")


def test_jump_family_accepts_relative_numeric_targets_with_prefix():
    assert assemble_hex("JMP R!0\nafter:\nHALT") == assemble_hex("JMP after\nafter:\nHALT")
    assert assemble_hex("JAL R!0\nafter:\nHALT") == assemble_hex("JAL after\nafter:\nHALT")
    assert assemble_hex("JMPS R!0\nafter:\nS.RET") == assemble_hex("JMPS after\nafter:\nS.RET")
    assert assemble_hex("JALS R!0\nafter:\nS.RET") == assemble_hex("JALS after\nafter:\nS.RET")
    assert assemble_hex("S.JAL R!0\nafter:\nS.RET") == assemble_hex("S.JAL after\nafter:\nS.RET")
