from pathlib import Path

from sol_vm import SolVM


def test_softfloat_binary32_arithmetic_and_special_values():
    library = Path(__file__).resolve().parents[2] / "sol" / "solstd" / "softfloat.sol"
    source = f'''!include "{library.as_posix()}"
1.0f 2.0f sf32_add
-1.5f 1.5f sf32_add
-0.0f -0.0f sf32_add
5.5f 2.25f sf32_sub
1.5f -2.0f sf32_mul
1.0f 3.0f sf32_div
0x00000001 0x00000001 sf32_add
0x00000001 1.0f sf32_mul
0x00800000 0.5f sf32_mul
0x7F7FFFFF 2.0f sf32_mul
0x7F800000 0xFF800000 sf32_add
1.0f 0.0f sf32_div
0.0f -0.0f sf32_eq
0x7FC00000 1.0f sf32_eq
-1.0f 1.0f sf32_lt
1.0f -1.0f sf32_lt
0x7FC00000 1.0f sf32_lt
1.0f sf32_neg
-1.0f sf32_abs
'''

    assert SolVM().run_source(source, source_path=str(Path(__file__).resolve())) == [
        0x40400000,
        0,
        -0x80000000,
        0x40500000,
        -0x3FC00000,
        0x3EAAAAAB,
        2,
        1,
        0x00400000,
        0x7F800000,
        0x7FC00000,
        0x7F800000,
        0,
        1,
        0,
        1,
        1,
        -0x40800000,
        0x3F800000,
    ]
