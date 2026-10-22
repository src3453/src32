"""Link a sol Foundation guest with the real C libfnd into a PROTECTED WAX.

    python tools/solc/build_foundation.py main.sol -o main.wax [--clang PATH]

No compiler/VM changes or external-symbol syntax are needed. The private C
dispatcher address is supplied as FND_LIB_ENTRY before sol compilation.
"""
import argparse
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/llvm-src32"))
sys.path.insert(0, str(ROOT / "tools/asm"))
sys.path.insert(0, str(ROOT / "tools/wax"))
sys.path.insert(0, str(ROOT / "tools/solc"))

from asm import Assembler
from driver import protected_blocks
from sol_compiler import compile_to_src32_asm, FOUNDATION_SLOTS
from wax import pack
import wax3


def build(source, output, *, clang=None, org=0x00800000, import_wal=None):
    source, output = Path(source).resolve(), Path(output).resolve()
    slot = next(((lo, hi) for lo, hi in FOUNDATION_SLOTS if lo == org and hi - lo >= 0x400000), None)
    if slot is None:
        raise ValueError("libfnd guests need slot 5 (0x800000) or slot 6 (0xc00000)")
    from llvmlite import binding as llvm
    from backend import compile_module
    from data import Layout
    import v3
    stack_top = slot[1]
    bridge_origin = org + 0x100000
    output.parent.mkdir(parents=True, exist_ok=True)
    ir_path = output.with_suffix(".libfnd.ll")
    lib = ROOT / "foundation/lib/fnd"
    sources = [ROOT / "sol/foundation/libfnd_bridge.c"]
    if import_wal is None:
        sources += [lib / name for name in ("start.c", "mem.c", "proc.c", "strerror.c")]
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), *map(str, sources),
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               "-O1", "--emit-llvm", "-o", str(ir_path)]
    if clang:
        command += ["--clang", str(clang)]
    subprocess.run(command, check=True)
    module = llvm.parse_assembly(ir_path.read_text(encoding="utf-8"))
    module.verify()
    imports = None
    if import_wal is not None:
        library = wax3.read_library(Path(import_wal).read_bytes())
        if (library.call_abi, library.os_id, library.os_major) != (1, "foundation", 1):
            raise ValueError("libfnd needs call ABI 1 and Foundation OS ABI 1")
        imports = v3.resolve_imports(module, Layout(module), [library], wax3)
    info = {}
    c_asm = compile_module(module, origin=bridge_origin, stack_top=stack_top,
                           entry=None, protected=True, info=info,
                           imports=imports.kinds if imports else None)
    entry = info["extents"]["sol_fnd_dispatch"][0]
    text = f"!const FND_LIB_ENTRY {entry}\n" + source.read_text(encoding="utf-8")
    sol_asm = compile_to_src32_asm(text, source_path=str(source), org=org, stack_top=stack_top)
    sol = Assembler(strip=True, sparse=True)
    sol.assemble(sol_asm)
    if sol.protection is None:
        raise ValueError('guest must declare !abi_compat "Foundation"')
    if sol.protection[0] != stack_top:
        raise ValueError("stack top must be the end of the selected user slot")
    sol_blocks = sol.protected_blocks()
    if max(b[0] + len(b[1]) for b in sol_blocks) > bridge_origin:
        raise ValueError("sol image overlaps libfnd (maximum sol image size: 1 MiB)")
    native = Assembler(strip=True, sparse=True)
    native.assemble(c_asm)
    blocks = sol_blocks + protected_blocks(native.blocks, info["data_start"])
    stack_bottom = stack_top - sol.protection[1]
    if max(b[0] + len(b[1]) for b in blocks) > stack_bottom:
        raise ValueError("libfnd overlaps the user stack")
    if imports is None:
        data = pack(org, 0, blocks, protection=sol.protection)
    else:
        data = v3.pack(wax3, kind="wax", image_id=v3.check_id(output.stem.replace("-", "_"), "image ID"), abi=(1, 0),
                       blocks=blocks, info=info, imports=imports, origin=org,
                       stack_top=stack_top, stack_size=sol.protection[1])
    output.with_suffix(".sol.a").write_text(sol_asm, encoding="utf-8")
    output.with_suffix(".libfnd.a").write_text(c_asm, encoding="utf-8")
    output.write_bytes(data)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("-o", "--output", type=Path, required=True)
    parser.add_argument("--clang")
    parser.add_argument("--import-wal", type=Path, help="use the fnd WAL instead of static libfnd")
    parser.add_argument("--org", type=lambda s: int(s, 0), default=0x00800000)
    args = parser.parse_args()
    print(build(args.source, args.output, clang=args.clang, org=args.org, import_wal=args.import_wal))


if __name__ == "__main__":
    main()
