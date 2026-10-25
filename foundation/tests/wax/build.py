"""Builds the images of the Foundation WAX loader test into OUT/images.h.

There is no v3 compiler yet, so the libraries and the v3 executables are
assembled with tools/asm and laid out with tools/wax/wax3.py. The v2 program
is C, built with tools/llvm-src32 --protected. main.c includes images.h.

    python foundation/tests/wax/build.py OUT [--clang PATH]
"""
import argparse
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / "tools/asm"))
sys.path.insert(0, str(ROOT / "tools/wax"))
from asm import Assembler  # noqa: E402
from wax3 import FUNCTION, OBJECT, PERM_RW, PERM_RX, Block, Dependency, Export, Import, build  # noqa: E402

# User slots (doc/spec_Foundation.md §5.2) and stack size of each executable.
SLOT_END = {0x00480000: 0x00500000, 0x00500000: 0x00580000, 0x00600000: 0x00800000,
            0x00800000: 0x00C00000, 0x00C00000: 0x01000000}
STACK = 0x1000

# test.app's descriptors, at these offsets in its BSS block:
# (constant, dependency index, symbol, kind, signature or object size).
APP_IMPORTS = [
    ("TWICE", 0, "twice", FUNCTION, "fn(u32)->u32"),
    ("APPLY", 0, "apply", FUNCTION, "fn(fnptr,u32)->u32"),
    ("CPTR", 0, "counter", FUNCTION, "fn()->ptr"),
    ("ORIGIN", 0, "origin", FUNCTION, "fn()->u32"),
    ("ADD64", 1, "add64", FUNCTION, "fn(u64,u64)->u64"),
    ("ADD_A", 1, "add", FUNCTION, "fn(u32,u32)->u32"),
    ("ADD_B", 1, "add", FUNCTION, "fn(u32,u32)->u32"),
    ("COUNTER", 1, "counter", OBJECT, 4),
]


def assemble(name, consts=None):
    text = "".join(f".CONST {key} 0x{value:X}\n" for key, value in (consts or {}).items())
    text += (HERE / name).read_text(encoding="utf-8")
    assembler = Assembler(strip=True, sparse=True)
    assembler.assemble(text, source=str(HERE / name), base_dir=str(HERE))
    return assembler.blocks, assembler.symbols


def library(name, image_id, minor, exports, deps=(), imports=()):
    """Code at offset 0 (RX), a 16-byte BSS block at 0x100 (RW)."""
    (code,), symbols = assemble(name)
    table = [Export(n, kind, symbols[n] if kind == FUNCTION else offset,
                    symbols[end] - symbols[n] if kind == FUNCTION else size, sig)
             for n, kind, end, offset, size, sig in exports]
    return build("wal", image_id=image_id, abi_minor=minor, deps=deps, imports=imports, exports=table,
                 blocks=[Block(0, code[1], perm=PERM_RX),
                         Block(0x100, b"", memory_size=0x10, perm=PERM_RW)]).data


def app(base, data, protected=True):
    consts = {"APP_BASE": base}
    imports = []
    for i, (name, dep, symbol, kind, extra) in enumerate(APP_IMPORTS):
        consts["S_" + name], consts["C_" + name] = data + 8 * i, data + 8 * i + 4
        imports.append(Import(dep, symbol, kind, data + 8 * i, extra if kind == OBJECT else 0,
                              extra if kind == FUNCTION else None))
    (code,), symbols = assemble("app.asm", consts)
    return build("wax", image_id="test.app", entry_offset=symbols["start"] - base,
                 stack_top=SLOT_END[base], stack_size=STACK, protected=protected,
                 deps=[Dependency("test.math", 1, 2), Dependency("test.base", 1, 0)], imports=imports,
                 blocks=[Block(base, code[1], perm=PERM_RX),
                         Block(base + data, b"", memory_size=8 * len(APP_IMPORTS), perm=PERM_RW)]).data


def poke(base, entry, data=0x100):
    blocks, symbols = assemble("poke.asm", {"APP_BASE": base, "DATA_ADDR": base + data})
    return build("wax", image_id="test.poke", entry_offset=symbols[entry] - base,
                 stack_top=SLOT_END[base], stack_size=STACK,
                 blocks=[Block(blocks[0][0], blocks[0][1], perm=PERM_RX),
                         Block(blocks[1][0], blocks[1][1], perm=PERM_RW)]).data


def v2_program(out, clang):
    image = out / "v2app.wax"
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), str(HERE / "v2app.c"),
               "-I", str(ROOT / "sdk/cpt32/include"), "-I", str(ROOT / "foundation/include"),
               "--protected", "--origin", "0x580000", "--stack-top", "0x600000",
               "--user-stack-size", "0x1000", "-O1", "-o", str(image)]
    if clang:
        command[3:3] = ["--clang", clang]
    subprocess.run(command, check=True)
    return image.read_bytes()


def pie_program(out, clang):
    """A relocatable executable in C (driver --pie)."""
    image = out / "pie.wax"
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), str(HERE / "pie.c"),
               "-I", str(ROOT / "sdk/cpt32/include"), "-I", str(ROOT / "foundation/include"),
               "--wax3", "--pie", "--page-align", "--image-id", "test.pie", "--user-stack-size", "0x1000",
               "-O1", "-o", str(image)]
    if clang:
        command[3:3] = ["--clang", clang]
    subprocess.run(command, check=True)
    return image.read_bytes()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", type=Path)
    parser.add_argument("--clang")
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)

    base_exports = [("add", FUNCTION, "add64", 0, 0, "fn(u32,u32)->u32"),
                    ("add64", FUNCTION, "base_end", 0, 0, "fn(u64,u64)->u64"),
                    ("counter", OBJECT, None, 0x100, 4, None)]
    math_exports = [("apply", FUNCTION, "counter", 0, 0, "fn(fnptr,u32)->u32"),
                    ("counter", FUNCTION, "origin", 0, 0, "fn()->ptr"),
                    ("origin", FUNCTION, "math_end", 0, 0, "fn()->u32"),
                    ("twice", FUNCTION, "apply", 0, 0, "fn(u32)->u32")]
    math_deps = [Dependency("test.base")]
    math_imports = [Import(0, "add", FUNCTION, 0x100, 0, "fn(u32,u32)->u32"),
                    Import(0, "counter", OBJECT, 0x108, 4)]
    images = {
        "base": library("base.asm", "test.base", 0, base_exports),
        "math": library("math.asm", "test.math", 2, math_exports, math_deps, math_imports),
        "math_old": library("math.asm", "test.math", 1, math_exports, math_deps, math_imports),
        # The same libraries at different bases: test.app's reservation
        # differs between slots 5 and 6.
        "app": app(0x00800000, 0x1000),
        "app2": app(0x00C00000, 0x3000),
        "app_unprotected": app(0x00800000, 0x1000, protected=False),
        "poke_write": poke(0x00600000, "write_code"),
        "poke_exec": poke(0x00500000, "exec_data"),
        "spin": poke(0x00480000, "spin"),
        # With V a page has one set of permissions, so these keep code and data
        # on different pages (foundation/tests/vm).
        "poke_write_vm": poke(0x00600000, "write_code", 0x1000),
        "poke_exec_vm": poke(0x00500000, "exec_data", 0x1000),
        "v2app": v2_program(args.out, args.clang),
        "pie": pie_program(args.out, args.clang),
    }
    # Big-endian words: the same bytes in SRC32 memory, 4-byte aligned.
    lines = ["/* Generated by foundation/tests/wax/build.py. */", "#include <stdint.h>", ""]
    for name, data in images.items():
        padded = data + bytes(-len(data) % 4)
        words = ",".join("0x%Xu" % int.from_bytes(padded[i:i + 4], "big") for i in range(0, len(padded), 4))
        lines.append(f"#define img_{name}_size {len(data)}u")
        lines.append(f"static const uint32_t img_{name}[{len(padded) // 4}] = {{{words}}};")
    (args.out / "images.h").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Wrote {args.out / 'images.h'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
