"""Test vectors for the software floating point (test_toolchain.py).

`program()` returns a freestanding C program that checks every floating-point
instruction the backend lowers (arithmetic, fmod, sqrt, comparisons,
conversions in both directions, fneg/fabs/copysign, fma) against bit patterns
computed here with the host's IEEE-754 arithmetic. main() returns 42 when all
agree, else the (1-based) number of the first failing table; the failing row
is left in `failed_row`.
"""
from __future__ import annotations

import ctypes
import math
import random
import struct

SPECIAL32 = [0x00000000, 0x80000000, 0x3F800000, 0xBF800000, 0x3F000000, 0x40000000, 0x40490FDB,
             0x7F7FFFFF, 0xFF7FFFFF, 0x7F800000, 0xFF800000, 0x7FC00000, 0x00000001, 0x80000001,
             0x007FFFFF, 0x00800000, 0x33800000, 0x4B000000, 0x4B7FFFFF, 0x3EAAAAAB, 0x41200000,
             0xC2F6E979, 0x5F000000, 0x4EFFFFFF]
SPECIAL64 = [0x0000000000000000, 0x8000000000000000, 0x3FF0000000000000, 0xBFF0000000000000,
             0x3FE0000000000000, 0x4000000000000000, 0x400921FB54442D18, 0x7FEFFFFFFFFFFFFF,
             0xFFEFFFFFFFFFFFFF, 0x7FF0000000000000, 0xFFF0000000000000, 0x7FF8000000000000,
             0x0000000000000001, 0x8000000000000001, 0x000FFFFFFFFFFFFF, 0x0010000000000000,
             0x3CA0000000000000, 0x4330000000000000, 0x433FFFFFFFFFFFFF, 0x3FD5555555555555,
             0x4024000000000000, 0xC05EDD2F1A9FBE77, 0x43E0000000000000, 0x41DFFFFFFFC00000]


def to_f32(x):
    return ctypes.c_float(x).value


def f32_value(bits):
    return struct.unpack(">f", bits.to_bytes(4, "big"))[0]


def f32_bits(x):
    return struct.unpack(">I", struct.pack(">f", x))[0]


def f64_value(bits):
    return struct.unpack(">d", bits.to_bytes(8, "big"))[0]


def f64_bits(x):
    return struct.unpack(">Q", struct.pack(">d", x))[0]


def int_to_f32(n):
    """Round the integer to nearest even binary32 (no double rounding)."""
    if n == 0:
        return 0
    sign, n = (0x80000000, -n) if n < 0 else (0, n)
    exponent = n.bit_length() - 1
    if exponent <= 23:
        mantissa = n << (23 - exponent)
    else:
        shift = exponent - 23
        mantissa, rest = divmod(n, 1 << shift)
        half = 1 << (shift - 1)
        if rest > half or (rest == half and mantissa & 1):
            mantissa += 1
            if mantissa == 1 << 24:
                mantissa >>= 1
                exponent += 1
    return sign | (127 + exponent) << 23 | (mantissa & 0x7FFFFF)


def randoms(rng, count, bits):
    """Random patterns with all exponents represented (and some shared)."""
    result = []
    for _ in range(count):
        if bits == 32:
            result.append(rng.getrandbits(1) << 31 | rng.randrange(0x60, 0xA0) << 23 | rng.getrandbits(23))
        else:
            result.append(rng.getrandbits(1) << 63 | rng.randrange(0x380, 0x480) << 52 | rng.getrandbits(52))
    return result


def divide(x, y):
    try:
        return x / y
    except ZeroDivisionError:
        if x == 0 or x != x:
            return math.nan
        return math.copysign(math.inf, x) * math.copysign(1.0, y)


def fmod(x, y):
    try:
        return math.fmod(x, y)
    except ValueError:
        return math.nan


def square_root(x):
    try:
        return math.sqrt(x)
    except ValueError:
        return math.nan


def pairs(rng, special, bits, count):
    pool = special + randoms(rng, 24, bits)
    result = [(a, b) for a in special for b in special]
    result += [(rng.choice(pool), rng.choice(pool)) for _ in range(count)]
    return result


def compare_mask(a, b):
    ordered = a == a and b == b
    flags = [a == b, a != b, a < b, a <= b, a > b, a >= b,
             not (a < b), not (a <= b), not (a > b), not (a >= b),
             (a < b) or (a > b), not ((a < b) or (a > b)), ordered, not ordered]
    return sum(1 << i for i, flag in enumerate(flags) if flag)


def column_type(rows):
    return "uint64_t" if any(v >> 32 for row in rows for v in row) else "uint32_t"


class Program:
    def __init__(self):
        self.tables, self.checks = [], []

    def add(self, name, rows, expression, kind):
        """kind: 'f' / 'd' result patterns, or an integer mask (compare the
        low bits of an integer result)."""
        ctype = column_type(rows)
        digits = 16 if ctype == "uint64_t" else 8
        suffix = "ull" if ctype == "uint64_t" else "u"
        body = ",\n".join("{" + ",".join(f"0x{v:0{digits}X}{suffix}" for v in row) + "}" for row in rows)
        self.tables.append(f"static const {ctype} {name}[][{len(rows[0])}] = {{\n{body}}};\n")
        self.checks.append((name, len(rows[0]), expression, kind))

    def source(self):
        lines = ["#include <stdint.h>",
                 "typedef union { float f; uint32_t u; } F; typedef union { double d; uint64_t u; } D;",
                 "static float Uf(uint64_t v) { F x; x.u = (uint32_t)v; return x.f; }",
                 "static double Ud(uint64_t v) { D x; x.u = v; return x.d; }",
                 "static uint32_t Bf(float v) { F x; x.f = v; return x.u; }",
                 "static uint64_t Bd(double v) { D x; x.d = v; return x.u; }",
                 "float fmodf(float, float); double fmod(double, double);",
                 "float sqrtf(float); double sqrt(double); double fma(double, double, double);",
                 "volatile uint32_t failed_row;",
                 "static int same32(uint64_t g, uint64_t w) { return g == w || ((g & 0x7FFFFFFFu) > 0x7F800000u && (w & 0x7FFFFFFFu) > 0x7F800000u); }",
                 "static int same64(uint64_t g, uint64_t w) { return g == w || ((g & 0x7FFFFFFFFFFFFFFFull) > 0x7FF0000000000000ull && (w & 0x7FFFFFFFFFFFFFFFull) > 0x7FF0000000000000ull); }",
                 "#define CMP(a, b) ((uint64_t)((a == b) | (a != b) << 1 | (a < b) << 2 | (a <= b) << 3 | (a > b) << 4 | (a >= b) << 5 | \\",
                 "  !(a < b) << 6 | !(a <= b) << 7 | !(a > b) << 8 | !(a >= b) << 9 | (((a < b) || (a > b)) << 10) | \\",
                 "  !((a < b) || (a > b)) << 11 | ((a == a && b == b) << 12) | !(a == a && b == b) << 13))"]
        lines += self.tables
        lines.append("int main(void) {")
        for number, (name, columns, expression, kind) in enumerate(self.checks, 1):
            lines.append(f"  for (unsigned i = 0; i < sizeof {name} / sizeof {name}[0]; ++i) {{")
            lines.append(f"    uint64_t a = {name}[i][0], b = {name}[i][1], want = {name}[i][2]; (void)a; (void)b;")
            if columns > 3:
                lines.append(f"    uint64_t c = {name}[i][3]; (void)c;")
            if kind == "f":
                lines.append(f"    int ok = same32(Bf({expression}), want);")
            elif kind == "d":
                lines.append(f"    int ok = same64(Bd({expression}), want);")
            else:
                lines.append(f"    int ok = (((uint64_t)({expression})) & {kind:#x}ull) == want;")
            lines.append(f"    if (!ok) {{ failed_row = i; return {number}; }}")
            lines.append("  }")
        lines.append("  return 42;\n}")
        return "\n".join(lines)


def program():
    rng = random.Random(0x5233)
    p = Program()
    for width, special, val, bit, rnd, sfx, kind in (
            (32, SPECIAL32, f32_value, f32_bits, to_f32, "f", "f"),
            (64, SPECIAL64, f64_value, f64_bits, lambda x: x, "d", "d")):
        operands = pairs(rng, special, width, 220)
        U = "Uf" if width == 32 else "Ud"
        libm = "f" if width == 32 else ""
        binary = {"+": lambda x, y: x + y, "-": lambda x, y: x - y, "*": lambda x, y: x * y, "/": divide}
        for index, (op, fn) in enumerate(binary.items()):
            p.add(f"arith_{sfx}{index}", [(a, b, bit(rnd(fn(val(a), val(b))))) for a, b in operands],
                  f"{U}(a) {op} {U}(b)", kind)
        p.add(f"fmod_{sfx}", [(a, b, bit(rnd(fmod(val(a), val(b))))) for a, b in operands],
              f"fmod{libm}({U}(a), {U}(b))", kind)
        p.add(f"sqrt_{sfx}", [(a, 0, bit(rnd(square_root(val(a))))) for a, _ in operands],
              f"sqrt{libm}({U}(a))", kind)
        p.add(f"copysign_{sfx}", [(a, b, bit(math.copysign(val(a), val(b)))) for a, b in operands],
              f"__builtin_copysign{libm}({U}(a), {U}(b))", kind)
        p.add(f"fabs_{sfx}", [(a, 0, a & ~(1 << (width - 1))) for a, _ in operands],
              f"__builtin_fabs{libm}({U}(a))", kind)
        p.add(f"neg_{sfx}", [(a, 0, a ^ (1 << (width - 1))) for a, _ in operands], f"-{U}(a)", kind)
        p.add(f"cmp_{sfx}", [(a, b, compare_mask(val(a), val(b))) for a, b in operands],
              f"CMP({U}(a), {U}(b))", 0x3FFF)
    from fractions import Fraction
    rows = []
    while len(rows) < 300:
        a, b, c = (rng.choice(SPECIAL64[2:8] + randoms(rng, 6, 64)) for _ in range(3))
        x, y, z = f64_value(a), f64_value(b), f64_value(c)
        try:
            result = float(Fraction(x) * Fraction(y) + Fraction(z))
        except OverflowError:
            continue
        rows.append((a, b, f64_bits(result), c))
    p.add("fma_d", rows, "fma(Ud(a), Ud(b), Ud(c))", "d")
    ints32 = [0, 1, -1, 2, 127, 128, -128, 255, 256, 32767, -32768, 65535, 0x7FFFFF, 0x1000000, 0x1000001,
              0x7FFFFFFF, -0x80000000, 0x40000001, 0x3FFFFFFF] + [rng.randrange(-2**31, 2**31) for _ in range(60)]
    ints64 = ints32 + [0x7FFFFFFFFFFFFFFF, -0x8000000000000000, 0x20000000000001, 0x1000000000000000,
                       0x0000000100000001] + [rng.randrange(-2**63, 2**63) for _ in range(60)]
    m32, m64 = 0xFFFFFFFF, 0xFFFFFFFFFFFFFFFF
    p.add("i32_f", [(v & m32, 0, int_to_f32(v)) for v in ints32], "(float)(int32_t)a", "f")
    p.add("u32_f", [(v & m32, 0, int_to_f32(v & m32)) for v in ints32], "(float)(uint32_t)a", "f")
    p.add("i64_f", [(v & m64, 0, int_to_f32(v)) for v in ints64], "(float)(int64_t)a", "f")
    p.add("u64_f", [(v & m64, 0, int_to_f32(v & m64)) for v in ints64], "(float)(uint64_t)a", "f")
    p.add("i32_d", [(v & m32, 0, f64_bits(float(v))) for v in ints32], "(double)(int32_t)a", "d")
    p.add("u32_d", [(v & m32, 0, f64_bits(float(v & m32))) for v in ints32], "(double)(uint32_t)a", "d")
    p.add("i64_d", [(v & m64, 0, f64_bits(float(v))) for v in ints64], "(double)(int64_t)a", "d")
    p.add("u64_d", [(v & m64, 0, f64_bits(float(v & m64))) for v in ints64], "(double)(uint64_t)a", "d")
    p.add("i8_f", [(v & 0xFF, 0, int_to_f32(v)) for v in (-128, -1, 0, 1, 127)], "(float)(int8_t)a", "f")
    p.add("u16_d", [(v, 0, f64_bits(float(v))) for v in (0, 1, 255, 256, 65535)], "(double)(uint16_t)a", "d")
    p.add("f_d", [(a, 0, f64_bits(f32_value(a))) for a in SPECIAL32 + randoms(rng, 40, 32)], "(double)Uf(a)", "d")
    p.add("d_f", [(a, 0, f32_bits(to_f32(f64_value(a)))) for a in SPECIAL64 + randoms(rng, 40, 64)
                  + [f64_bits(2.0 ** e) for e in (-150, -149, -127, -126, 127, 128, 129)]], "(float)Ud(a)", "f")
    reals = [0.0, -0.0, 0.5, -0.5, 0.99999994, 1.0, -1.0, 1.5, -1.5, 2.5, 100.75, -100.75, 255.9, 65535.5,
             2147483520.0, -2147483648.0, 16777216.0, 4294967040.0]
    wide = reals + [1e10, -1e10, 4294967296.0, 9.2e18, -9.2e18, 1.8e19, 123456789012.0, 0.1]

    def to_int(name, values, width, expression, low, high, mask, source):
        bit = f32_bits if source == 32 else f64_bits
        rows = [(bit(v), 0, int(v) & mask) for v in values if low <= int(v) <= high]
        p.add(name, rows, expression, mask)

    r32 = [to_f32(v) for v in reals]
    w32 = [to_f32(v) for v in wide]
    to_int("f_i32", r32, 32, "(int32_t)Uf(a)", -2**31, 2**31 - 1, m32, 32)
    to_int("f_u32", [v for v in r32 if v > -1], 32, "(uint32_t)Uf(a)", 0, 2**32 - 1, m32, 32)
    to_int("d_i32", reals, 32, "(int32_t)Ud(a)", -2**31, 2**31 - 1, m32, 64)
    to_int("d_i64", wide, 64, "(int64_t)Ud(a)", -2**63, 2**63 - 1, m64, 64)
    to_int("d_u64", [v for v in wide if v > -1], 64, "(uint64_t)Ud(a)", 0, 2**64 - 1, m64, 64)
    to_int("f_i64", w32, 64, "(int64_t)Uf(a)", -2**63, 2**63 - 1, m64, 32)
    to_int("f_u64", [v for v in w32 if v > -1], 64, "(uint64_t)Uf(a)", 0, 2**64 - 1, m64, 32)
    to_int("d_i8", [-128.9, -1.5, 0.5, 127.9], 8, "(int8_t)Ud(a)", -128, 127, 0xFF, 64)
    return p.source()
