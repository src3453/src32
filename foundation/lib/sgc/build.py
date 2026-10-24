"""Build libsgc as the Foundation WAL "sgc" 1.0 (/lib/sgc.1.wal)."""
import argparse
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def build(out, fnd, clang=None, optimization="-O2"):
    """fnd is the built fnd WAL (lib/fnd.1.wal) that libsgc imports."""
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), str(HERE / "sgc.c"),
               "-I", str(ROOT / "foundation/include"), "-I", str(ROOT / "sdk/cpt32/include"),
               optimization, "--wal", "--page-align", "--image-id", "sgc", "--abi-version", "1.0",
               "--exports", str(HERE / "sgc.exports"), "--import-wal", str(fnd),
               "-o", str(out / "libsgc.wal")]
    if clang:
        command += ["--clang", clang]
    subprocess.run(command, check=True)
    return out / "libsgc.wal"


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
