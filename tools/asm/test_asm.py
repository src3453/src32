import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "wax"))

from asm import Assembler, main as asm_main
from wax import parse as parse_wax

ASM_SCRIPT = Path(__file__).with_name("asm.py").resolve()


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



def test_sparse_assembly_preserves_addresses_without_gap_bytes():
    asm = Assembler(strip=True, sparse=True)
    compact = asm.assemble(".ORG 0x100\n.BYTE 1\n.ORG 0x10000\n.BYTE 2")
    assert asm.image_base == 0x100
    assert compact == b"\x01\x02"
    assert asm.blocks == [(0x100, b"\x01"), (0x10000, b"\x02")]
def test_cli_selects_sparse_wax_by_output_extension_and_entry_label(tmp_path, monkeypatch):
    source = tmp_path / "program.asm"
    source.write_text(
        ".ORG 0x100\nstart: NOP\nentry: HALT\n.ORG 0x10000\nHALT\n",
        encoding="utf-8",
    )
    output = tmp_path / "program.wax"
    monkeypatch.setattr(
        sys,
        "argv",
        [str(ASM_SCRIPT), str(source), "--entry", "entry", "-o", str(output)],
    )
    asm_main()
    image = parse_wax(output.read_bytes())
    assert image["base"] == 0x100
    assert image["entry_offset"] == 4
    assert image["entry"] == 0x104
    assert image["blocks"] == [
        {"load_address": 0x100, "data": bytes.fromhex("00000000fc000000")},
        {"load_address": 0x10000, "data": bytes.fromhex("fc000000")},
    ]
    assert len(output.read_bytes()) < len(Assembler().assemble(source.read_text()))
def test_wax_memory_image_matches_raw_binary_for_same_address_org(tmp_path, monkeypatch):
    source = tmp_path / "same_address_org.asm"
    source.write_text(
        ".ORG 0x100\n.BYTE 0x11\n.ORG 0x110\n.BYTE 0x22\n"
        ".ORG 0x111\n.BYTE 0x33\n",
        encoding="utf-8",
    )
    binary_path = tmp_path / "program.bin"
    monkeypatch.setattr(
        sys, "argv", [str(ASM_SCRIPT), str(source), "-o", str(binary_path)]
    )
    asm_main()
    binary = binary_path.read_bytes()

    wax_path = tmp_path / "program.wax"
    monkeypatch.setattr(
        sys, "argv", [str(ASM_SCRIPT), str(source), "-o", str(wax_path)]
    )
    asm_main()
    image = parse_wax(wax_path.read_bytes())
    expanded = bytearray(len(binary))
    for block in image["blocks"]:
        start = block["load_address"]
        expanded[start:start + len(block["data"])] = block["data"]

    assert bytes(expanded) == binary
    assert image["blocks"] == [
        {"load_address": 0x100, "data": b"\x11"},
        {"load_address": 0x110, "data": b"\x22\x33"},
    ]


def test_cli_raw_binary_output_remains_the_default(tmp_path, monkeypatch):
    source = tmp_path / "raw.asm"
    source.write_text(".ORG 8\nHALT\n", encoding="utf-8")
    monkeypatch.setattr(sys, "argv", [str(ASM_SCRIPT), str(source)])
    asm_main()
    output = tmp_path / "raw.asm.bin"
    assert output.read_bytes() == bytes(8) + bytes.fromhex("fc000000")
