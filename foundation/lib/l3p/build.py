"""Build libl3p as the Foundation WAL "l3p" 1.0 (/lib/l3p.1.wal).

libl3p links libcnet's code (lib/cnet/cnet.c) in and imports only fnd: a
process has 8 Extension P regions, and a program importing fnd, cnet and l3p
would need more (2 per image plus the stack)."""
import argparse
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def build(out, fnd, clang=None, optimization="-O2", *, run=None):
    """fnd is the built fnd WAL (lib/fnd.1.wal) that libl3p imports."""
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), str(HERE / "l3p.c"), str(HERE.parent / "cnet/cnet.c"),
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               optimization, "--wal", "--page-align", "--image-id", "l3p", "--abi-version", "1.0",
               "--exports", str(HERE / "l3p.exports"), "--import-wal", str(fnd),
               "-o", str(out / "libl3p.wal")]
    if clang:
        command += ["--clang", clang]
    output = out / "libl3p.wal"
    if run is None:
        subprocess.run(command, check=True)
    else:
        run(command, output)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--fnd", type=Path, required=True, help="the built fnd.1.wal")
    parser.add_argument("--clang")
    parser.add_argument("--optimization", choices=("-O0", "-O1", "-O2"), default="-O2")
    args = parser.parse_args()
    print(build(args.out, args.fnd, args.clang, args.optimization))


if __name__ == "__main__":
    main()
