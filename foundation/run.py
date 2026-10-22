"""Build and launch the Foundation shell with its own emulator configuration."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", action="store_true", help="build without opening the emulator")
    parser.add_argument("--clang", help="Clang executable passed to the SRC32 compiler")
    args = parser.parse_args()

    output = ROOT / "target/foundation-boot"
    runtime = output / "run"
    temporary = output / "tmp"
    runtime.mkdir(parents=True, exist_ok=True)
    temporary.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, TMP=str(temporary), TEMP=str(temporary), TMPDIR=str(temporary))
    command = [sys.executable, str(ROOT / "foundation/boot/build.py"), str(output)]
    if args.clang:
        command += ["--clang", args.clang]
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    subprocess.run(["cargo", "build", "--release", "--bin", "cpt32"],
                   cwd=ROOT, env=environment, check=True)

    # Config is relative to the emulator's cwd. Keep Foundation's font and
    # 16 MiB RAM, without the user's BIOS overriding the WAX entry point.
    font = ROOT / "assets/UNSCII.F08"
    if not font.is_file():
        raise FileNotFoundError(font)
    (runtime / "config.yaml").write_text(
        "cpu:\n  microarchitecture: cached-relaxed\n"
        "memory:\n  ram_mib: 16\n"
        f"pcg:\n  font: {json.dumps(str(font))}\n", encoding="utf-8")
    image = output / "foundation.wax"
    print(f"Foundation image: {image}", flush=True)
    if not args.build_only:
        executable = "cpt32.exe" if os.name == "nt" else "cpt32"
        return subprocess.run([str(ROOT / "target/release" / executable), str(image)],
                              cwd=runtime, env=environment).returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
