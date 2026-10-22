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
    parser.add_argument("--root", type=Path, help="host directory copied into the boot disk root")
    parser.add_argument("--disk", action="store_true",
                        help="boot foundation.img through the BIOS and stage1 instead of loading the WAX")
    parser.add_argument("--arg", nargs=argparse.REMAINDER, default=[], metavar="ARG",
                        help="pass all remaining arguments to the emulator (place this option last)")
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
    if args.root:
        command += ["--root", str(args.root.resolve())]
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    subprocess.run(["cargo", "build", "--release", "--bin", "cpt32"],
                   cwd=ROOT, env=environment, check=True)

    # Config is relative to the emulator's cwd. Keep Foundation's font and
    # 16 MiB RAM, without the user's BIOS overriding the WAX entry point.
    # foundation.img is always the HDD in dd0: fs mounts it as "/" and init
    # starts /bin/shell.wax from it, and the fs service writes to it. --disk
    # also configures the BIOS and passes a zero word as the program, so the
    # BIOS skips the RAM boot and boots the disk through stage1.
    font = ROOT / "assets/UNSCII.F08"
    if not font.is_file():
        raise FileNotFoundError(font)
    config = ("cpu:\n  microarchitecture: cached-relaxed\n"
              "memory:\n  ram_mib: 16\n"
              f"pcg:\n  font: {json.dumps(str(font))}\n")
    image = output / "foundation.wax"
    if args.disk:
        config += f"bios:\n  image: {json.dumps(str(ROOT / 'assets/bios.rom'))}\n"
        image = runtime / "no-ram-program.bin"
        image.write_bytes(bytes(4))
    config += (f"disk:\n  - slot: 0\n    type: \"hdd\"\n"
               f"    image: {json.dumps(str(output / 'foundation.img'))}\n    read_only: false\n")
    print(f"Foundation disk: {output / 'foundation.img'}", flush=True)
    (runtime / "config.yaml").write_text(config, encoding="utf-8")
    print(f"Foundation image: {image}", flush=True)
    if not args.build_only:
        executable = "cpt32.exe" if os.name == "nt" else "cpt32"
        return subprocess.run([str(ROOT / "target/release" / executable), str(image), *args.arg],
                              cwd=runtime, env=environment).returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
