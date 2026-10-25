"""Software floating point for the SRC32 backend.

There is no FPU. `float` and `double` values are carried as their IEEE-754
bit patterns, exactly like `i32` and `i64` (the same frame slots, argument
words and results; a double is big-endian, high word first). Every arithmetic
instruction becomes a call of a compiler-rt compatible routine (`__addsf3`,
`__ltdf2`, ...) that runtime/softfloat/rt.c implements with Berkeley SoftFloat.
This module is the single table of which routines an instruction needs, used
by the driver (which links the runtime) and by the backend (which emits calls
and prunes the runtime functions nothing reaches).
"""
from __future__ import annotations

import re
import struct

from llvmlite import binding as llvm

from data import CompileError

# Comparison routines return, like compiler-rt: __eq/__ne 0 iff equal/not
# equal... see PREDICATES for how each fcmp predicate tests them.
_ORDER = ("eq", "ne", "lt", "le", "gt", "ge")
# fcmp predicate -> [(routine, test)], clauses joined by `join`.
# A test is (operator, 0) on the routine's signed result; unordered operands
# make the compiler-rt routines return a value that fails the ordered test.
PREDICATES = {
    "oeq": ("and", [("eq", "==")]),
    "ogt": ("and", [("gt", ">")]),
    "oge": ("and", [("ge", ">=")]),
    "olt": ("and", [("lt", "<")]),
    "ole": ("and", [("le", "<=")]),
    "one": ("and", [("unord", "=="), ("ne", "!=")]),
    "ord": ("and", [("unord", "==")]),
    "uno": ("and", [("unord", "!=")]),
    "ueq": ("or", [("unord", "!="), ("eq", "==")]),
    "ugt": ("and", [("le", ">")]),
    "uge": ("and", [("lt", ">=")]),
    "ult": ("and", [("ge", "<")]),
    "ule": ("and", [("gt", "<=")]),
    "une": ("and", [("ne", "!=")]),
    "true": ("and", []),
    "false": ("and", []),
}
_FLAGS = {"fast", "nnan", "ninf", "nsz", "arcp", "contract", "afn", "reassoc"}
_ARITHMETIC = {"fadd": "add", "fsub": "sub", "fmul": "mul", "fdiv": "div"}
# libm functions the runtime provides (and llvm intrinsics lower to).
LIBM = ("fmodf", "fmod", "sqrtf", "sqrt", "fmaf", "fma")


def entry_points():
    """Every routine runtime/softfloat/rt.c can define."""
    names = set(LIBM)
    for k in ("sf", "df"):
        names.update(f"__{op}{k}3" for op in ("add", "sub", "mul", "div"))
        names.update(f"__{op}{k}2" for op in (*_ORDER, "unord"))
    names.update(("__extendsfdf2", "__truncdfsf2"))
    for k in ("sf", "df"):
        for width in ("si", "di"):
            names.update((f"__fix{k}{width}", f"__fixuns{k}{width}"))
            names.update((f"__float{width}{k}", f"__floatun{width}{k}"))
    return names


def kind(ty):
    """'sf' for float, 'df' for double, else None. Other FP types are errors."""
    k = ty.type_kind
    if k == llvm.TypeKind.float:
        return "sf"
    if k == llvm.TypeKind.double:
        return "df"
    if k in (llvm.TypeKind.half, llvm.TypeKind.bfloat, llvm.TypeKind.fp128,
             llvm.TypeKind.x86_fp80, llvm.TypeKind.ppc_fp128):
        raise CompileError(f"unsupported floating-point type {ty}; only float and double are supported")
    if k in (llvm.TypeKind.vector, llvm.TypeKind.scalable_vector):
        raise CompileError("vector types are not supported")
    return None


def is_fp(ty):
    return kind(ty) is not None


def comparison_predicate(inst):
    from backend import _instruction_tail
    words = _instruction_tail(inst).split()
    for word in words:
        if word not in _FLAGS:
            if word not in PREDICATES:
                raise CompileError(f"unsupported fcmp predicate {word}")
            return word
    raise CompileError("malformed fcmp")


def intrinsic_routines(name, ty):
    """Runtime routines of an fp intrinsic (inline ones need none)."""
    base = name.split(".")[1]
    k = kind(ty)
    if base == "sqrt":
        return ["sqrtf" if k == "sf" else "sqrt"]
    if base == "fma":
        return ["fmaf" if k == "sf" else "fma"]
    if base == "fmuladd":
        return [f"__mul{k}3", f"__add{k}3"]
    return []


def instruction_routines(inst):
    """The runtime routines instruction `inst` calls once lowered."""
    op = inst.opcode
    operands = list(inst.operands)
    if op in _ARITHMETIC:
        return [f"__{_ARITHMETIC[op]}{kind(inst.type)}3"]
    if op == "frem":
        return ["fmodf" if kind(inst.type) == "sf" else "fmod"]
    if op == "fcmp":
        k = kind(operands[0].type)
        join, clauses = PREDICATES[comparison_predicate(inst)]
        return [f"__{name}{k}2" for name, _ in clauses]
    if op == "fpext":
        return ["__extendsfdf2"]
    if op == "fptrunc":
        return ["__truncdfsf2"]
    if op in ("fptosi", "fptoui"):
        bits = 64 if inst.type.type_width > 32 else 32
        return [f"__fix{'uns' if op == 'fptoui' else ''}{kind(operands[0].type)}{'di' if bits == 64 else 'si'}"]
    if op in ("sitofp", "uitofp"):
        wide = operands[0].type.type_width > 32
        return [f"__float{'un' if op == 'uitofp' else ''}{'di' if wide else 'si'}{kind(inst.type)}"]
    if op == "call":
        callee = operands[-1]
        name = callee.name if callee.value_kind == llvm.ValueKind.function else ""
        if name.startswith("llvm.") and name.split(".")[1] in ("sqrt", "fma", "fmuladd"):
            return intrinsic_routines(name, inst.type)
    return []


def module_routines(module):
    """Every routine the instructions of all defined functions call."""
    needed = set()
    for function in module.functions:
        for block in function.blocks:
            for inst in block.instructions:
                needed.update(instruction_routines(inst))
    return needed


def literal_bits(ty, text):
    """Bit pattern of the LLVM constant `text` (without type) of float/double."""
    text = text.strip()
    double = kind(ty) == "df"
    if re.fullmatch(r"0x[0-9a-fA-F]{1,16}", text):
        bits = int(text, 16)  # LLVM prints every float constant as a double
        if double:
            return bits
        sign, exponent, fraction = bits >> 63, (bits >> 52) & 0x7FF, bits & ((1 << 52) - 1)
        if exponent == 0x7FF:  # inf, nan (the payload keeps its top bits)
            return sign << 31 | 0xFF << 23 | fraction >> 29
        value = struct.unpack(">d", bits.to_bytes(8, "big"))[0]
        return int.from_bytes(struct.pack(">f", value), "big")
    if re.fullmatch(r"[-+]?[0-9]+\.[0-9]*(?:[eE][-+]?[0-9]+)?", text):
        value = float(text)
        return int.from_bytes(struct.pack(">d" if double else ">f", value), "big")
    raise CompileError(f"unsupported floating-point constant {text}")
