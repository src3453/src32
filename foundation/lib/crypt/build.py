"""Build the ChaCha20 and Poly1305 library as a Foundation WAL."""
import argparse
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def build(out, clang=None, optimization="-O2"):
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"),
               str(HERE / "crypt.c"), "-I", str(ROOT / "foundation/include"), optimization,
               "--wal", "--image-id", "crypt", "--abi-version", "1.0",
               "--exports", str(HERE / "crypt.exports"), "-o", str(out / "libcrypt.wal")]
    if clang:
        command += ["--clang", clang]
    subprocess.run(command, check=True)
    return out / "libcrypt.wal"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--clang")
    parser.add_argument("--optimization", choices=("-O0", "-O1", "-O2"), default="-O2")
    args = parser.parse_args()
    print(build(args.out, args.clang, args.optimization))


if __name__ == "__main__":
    main()
