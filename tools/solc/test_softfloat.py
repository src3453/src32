from pathlib import Path

from sol_vm import SolVM


def test_softfloat_binary32_arithmetic_and_special_values():
    library = Path(__file__).resolve().parents[2] / "sol" / "solstd" / "softfloat.sol"
    source = f'''!include "{library.as_posix()}"
1.0f 2.0f softfloat.sf32_add
-1.5f 1.5f softfloat.sf32_add
-0.0f -0.0f softfloat.sf32_add
5.5f 2.25f softfloat.sf32_sub
1.5f -2.0f softfloat.sf32_mul
1.0f 3.0f softfloat.sf32_div
0x00000001 0x00000001 softfloat.sf32_add
0x00000001 1.0f softfloat.sf32_mul
0x00800000 0.5f softfloat.sf32_mul
0x7F7FFFFF 2.0f softfloat.sf32_mul
0x7F800000 0xFF800000 softfloat.sf32_add
1.0f 0.0f softfloat.sf32_div
0.0f -0.0f softfloat.sf32_eq
0x7FC00000 1.0f softfloat.sf32_eq
-1.0f 1.0f softfloat.sf32_lt
1.0f -1.0f softfloat.sf32_lt
0x7FC00000 1.0f softfloat.sf32_lt
1.0f softfloat.sf32_neg
-1.0f softfloat.sf32_abs
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


def test_softfloat_integer_conversions():
    # sf32_toint truncates toward zero (NaN -> 0, out of range -> saturating
    # bounds); sf32_tofloat rounds the integer to nearest binary32 with ties
    # to even. The full contract is documented at the top of softfloat.sol.
    library = Path(__file__).resolve().parents[2] / "sol" / "solstd" / "softfloat.sol"
    source = f'''!include "{library.as_posix()}"
# sf32_toint: truncation toward zero, including |x| < 1
1.0f softfloat.sf32_toint
-1.0f softfloat.sf32_toint
1.9f softfloat.sf32_toint
-1.9f softfloat.sf32_toint
0x3FC00000 softfloat.sf32_toint
0xBFC00000 softfloat.sf32_toint
0.5f softfloat.sf32_toint
-0.5f softfloat.sf32_toint
0.0f softfloat.sf32_toint
-0.0f softfloat.sf32_toint
# sf32_toint: saturation bounds and non-numeric inputs
0x4F000000 softfloat.sf32_toint
0xCF000000 softfloat.sf32_toint
0x4F7FFFFF softfloat.sf32_toint
0xCF7FFFFF softfloat.sf32_toint
0x4EFFFFFF softfloat.sf32_toint
0x7F800000 softfloat.sf32_toint
0xFF800000 softfloat.sf32_toint
0x7FC00000 softfloat.sf32_toint
0x00000001 softfloat.sf32_toint
# sf32_tofloat: exact values, ties to even, and the int32 boundaries
0 softfloat.sf32_tofloat
1 softfloat.sf32_tofloat
-1 softfloat.sf32_tofloat
-3 softfloat.sf32_tofloat
0x01000000 softfloat.sf32_tofloat
0x01000001 softfloat.sf32_tofloat
0x01000003 softfloat.sf32_tofloat
0x00FFFFFF softfloat.sf32_tofloat
0x7FFFFFFF softfloat.sf32_tofloat
0x7FFFFF80 softfloat.sf32_tofloat
0x80000000 softfloat.sf32_tofloat
123456789 softfloat.sf32_tofloat
# round trips through both conversions
3.5f softfloat.sf32_toint softfloat.sf32_tofloat
-3.5f softfloat.sf32_toint softfloat.sf32_tofloat
123456789.0f softfloat.sf32_toint softfloat.sf32_tofloat
'''

    assert SolVM().run_source(source, source_path=str(Path(__file__).resolve())) == [
        1,
        -1,
        1,
        -1,
        1,
        -1,
        0,
        0,
        0,
        0,
        0x7FFFFFFF,
        -0x80000000,
        0x7FFFFFFF,
        -0x80000000,
        2147483520,
        0x7FFFFFFF,
        -0x80000000,
        0,
        0,
        0,
        0x3F800000,
        -0x40800000,
        -0x3FC00000,
        0x4B800000,
        0x4B800000,
        0x4B800002,
        0x4B7FFFFF,
        0x4F000000,
        0x4EFFFFFF,
        -0x31000000,
        0x4CEB79A3,
        0x40400000,
        -0x3FC00000,
        0x4CEB79A3,
    ]
