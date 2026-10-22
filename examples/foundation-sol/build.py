"""Build both sol guests and a runnable Foundation boot image.

    python examples/foundation-sol/build.py target/foundation-sol [--clang PATH]
"""
import argparse
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tools/solc"))
from build_foundation import build


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--clang")
    parser.add_argument("--stop-ticks", type=int)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    lib = ROOT / "foundation/lib/fnd"
    wal = out / "sample-fnd.wal"
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"),
               *[str(lib / name) for name in ("start.c", "mem.c", "proc.c", "strerror.c")],
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               "-O1", "--wal", "--image-id", "fnd", "--exports", str(lib / "fnd.exports"),
               "-o", str(wal)]
    if args.clang:
        command += ["--clang", args.clang]
    subprocess.run(command, check=True)
    # Keep intermediate .a/.ll outside --root, which installs every file.
    for name, source, org in (("sol-demo", "main.sol", 0x800000),
                              ("sol-child", "child.sol", 0xc00000)):
        image = build(HERE / source, out / "guests" / f"{name}.wax", clang=args.clang, org=org,
                      import_wal=wal)
        dest = out / "sample-root/bin" / image.name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(image.read_bytes())
    command = [sys.executable, str(ROOT / "foundation/boot/build.py"), str(out),
               "--root", str(out / "sample-root")]
    if args.clang:
        command += ["--clang", args.clang]
    if args.stop_ticks is not None:
        command += ["--stop-ticks", str(args.stop_ticks)]
    subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
