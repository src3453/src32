"""Build liblzma.wal and its Foundation WAX v3 consumer; stage an FS2 overlay."""
import argparse
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def build(out, clang=None, optimization="-O2"):
    out = Path(out).resolve()
    spec = importlib.util.spec_from_file_location("lzma_build", ROOT / "foundation/lib/lzma/build.py")
    library_build = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(library_build)
    library = library_build.build(out, clang, optimization)
    command = [sys.executable, str(ROOT / "tools/llvm-src32/driver.py"), str(HERE / "main.c"),
               "-I", str(ROOT / "foundation/include"), optimization, "--wax3",
               "--image-id", "lzma_demo", "--import-wal", str(library),
               "--origin", "0x800000", "--stack-top", "0xC00000",
               "--user-stack-size", "0x240000", "-o", str(out / "lzma-demo.wax")]
    if clang:
        command += ["--clang", clang]
    subprocess.run(command, check=True)
    for directory in ("bin", "lib"):
        (out / "root" / directory).mkdir(parents=True, exist_ok=True)
    shutil.copyfile(library, out / "root/lib/lzma.1.wal")
    shutil.copyfile(out / "lzma-demo.wax", out / "root/bin/lzma-demo.wax")
    return library


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--clang")
    parser.add_argument("--optimization", choices=("-O0", "-O1", "-O2"), default="-O2")
    args = parser.parse_args()
    build(args.out, args.clang, args.optimization)
