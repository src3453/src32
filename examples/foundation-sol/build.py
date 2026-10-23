"""Build both sol guests and a runnable Foundation boot image.

    python examples/foundation-sol/build.py target/foundation-sol [--clang PATH]
"""
import argparse
import json
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
               *[str(lib / name) for name in ("start.c", "mem.c", "proc.c", "rename.c", "strerror.c")],
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               "-O1", "--wal", "--image-id", "fnd", "--abi-version", "1.1", "--exports", str(lib / "fnd.exports"),
               "-o", str(wal)]
    if args.clang:
        command += ["--clang", args.clang]
    subprocess.run(command, check=True)

    fixtures = out / "sample-root/data/sol-fixtures"
    fixtures.mkdir(parents=True, exist_ok=True)
    for i in range(64):
        (fixtures / f"entry{i:02d}.txt").write_bytes(f"fixture {i}\n".encode("utf-8"))
    (fixtures / "日本語.txt").write_bytes(b"UTF-8 fixture\n")
    small = fixtures.parent / "sol-small"
    small.mkdir(exist_ok=True)
    for name in ("a.txt", "b.txt"):
        (small / name).write_text(name, encoding="utf-8")
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
    runtime = out / "run"
    runtime.mkdir(parents=True, exist_ok=True)
    disk = runtime / "disk.img"
    disk.write_bytes((out / "foundation.img").read_bytes())
    (runtime / "config.yaml").write_text(
        'memory:\n  ram_mib: 16\n'
        f'pcg:\n  font: {json.dumps(str(ROOT / "assets/UNSCII.F08"))}\n'
        f'disk:\n  - slot: 0\n    type: "hdd"\n    image: {json.dumps(str(disk))}\n'
        '    read_only: false\n', encoding="utf-8")


if __name__ == "__main__":
    main()
