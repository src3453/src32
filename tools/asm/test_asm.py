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


def test_register_jump_and_link_opcodes():
    assert assemble_hex("JALR R5") == "84a00000"
    assert assemble_hex("JALRS R5") == "88a00000"


def test_jump_family_accepts_relative_numeric_targets_with_prefix():
    assert assemble_hex("JMP R!0\nafter:\nHALT") == assemble_hex("JMP after\nafter:\nHALT")
    assert assemble_hex("JAL R!0\nafter:\nHALT") == assemble_hex("JAL after\nafter:\nHALT")
    assert assemble_hex("JMPS R!0\nafter:\nS.RET") == assemble_hex("JMPS after\nafter:\nS.RET")
    assert assemble_hex("JALS R!0\nafter:\nS.RET") == assemble_hex("JALS after\nafter:\nS.RET")
    assert assemble_hex("S.JAL R!0\nafter:\nS.RET") == assemble_hex("S.JAL after\nafter:\nS.RET")


def test_ldih_and_ldil_encode_16_bit_immediates():
    assert assemble_hex("LDIH R1, 0x000F\nLDIL R1, 0xFFFC") == "f420000ff020fffc"


def test_org_pads_forward_gap_by_default():
    asm = Assembler()
    binary = asm.assemble(".ORG 0x100\nLDIH R1, 0x0001")
    assert asm.image_base == 0
    assert len(binary) == 0x104
    assert binary[:0x100] == bytes(0x100)
    assert binary[0x100:].hex() == assemble_hex("LDIH R1, 0x0001")


def test_strip_omits_leading_padding_but_keeps_interior_gaps():
    asm = Assembler(strip=True)
    binary = asm.assemble(".ORG 0xFF000000\nLDIH R1, 0x0001\n.ORG 0xFF000010\nLDIL R1, 0x0002")
    assert asm.image_base == 0xFF000000
    assert len(binary) == 0x14
    assert binary[:4].hex() == assemble_hex("LDIH R1, 0x0001")
    assert binary[4:0x10] == bytes(0x10 - 4)
    assert binary[0x10:].hex() == assemble_hex("LDIL R1, 0x0002")


def test_strip_without_org_behaves_like_default():
    asm = Assembler(strip=True)
    binary = asm.assemble("LDIH R1, 0x0001")
    assert asm.image_base == 0
    assert binary.hex() == assemble_hex("LDIH R1, 0x0001")


def test_backward_org_is_rejected_with_and_without_strip():
    for strip in (False, True):
        asm = Assembler(strip=strip)
        try:
            asm.assemble("LDIH R1, 0x0001\n.ORG 0x0002\nLDIL R1, 0x0002")
        except ValueError as exc:
            assert "overlaps" in str(exc)
        else:
            raise AssertionError("backward .ORG must fail")
