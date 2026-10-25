"""Build and launch the Foundation shell with its own emulator configuration."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def physical_cores():
    """Physical core count of this machine (SMT siblings excluded); falls back to the logical count."""
    count = None
    try:
        if sys.platform.startswith("linux"):
            cores, package = set(), None
            for line in Path("/proc/cpuinfo").read_text().splitlines():
                key, _, value = line.partition(":")
                key, value = key.strip(), value.strip()
                if key == "physical id":
                    package = value
                elif key == "core id":
                    cores.add((package, value))
            count = len(cores)
        elif sys.platform == "darwin":
            count = int(subprocess.check_output(["sysctl", "-n", "hw.physicalcpu"], text=True))
        elif os.name == "nt":
            output = subprocess.check_output(
                ["powershell", "-NoProfile", "-Command",
                 "(Get-CimInstance Win32_Processor | Measure-Object -Property NumberOfCores -Sum).Sum"], text=True)
            count = int(output.strip())
    except (OSError, ValueError, subprocess.SubprocessError):
        pass
    return count if count and count > 0 else (os.cpu_count() or 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", action="store_true", help="build without opening the emulator")
    parser.add_argument("--clang", help="Clang executable passed to the SRC32 compiler")
    parser.add_argument("-j", "--jobs", type=int, metavar="N",
                        help="parallel compiler processes (default: physical core count)")
    parser.add_argument("--root", type=Path, help="host directory copied into the boot disk root")
    parser.add_argument("--bin-dir", type=Path, action="append", default=[], metavar="DIR",
                        help="add direct *.wax children to /bin; repeatable, later directories win")
    parser.add_argument("--ram-mib", type=int, default=16, help="Main RAM in MiB (default: 16)")
    parser.add_argument("--disk-blocks", type=int, help="boot disk size in 512-byte blocks (default: 16384)")
    parser.add_argument("--disk", action="store_true",
                        help="boot foundation.img through the BIOS and stage1 instead of loading the WAX")
    parser.add_argument("--arg", nargs=argparse.REMAINDER, default=[], metavar="ARG",
                        help="pass all remaining arguments to the emulator (place this option last)")
    args = parser.parse_args()
    if not 1 <= args.ram_mib <= 256:
        parser.error("--ram-mib must be between 1 and 256")

    output = ROOT / "target/foundation-boot"
    runtime = output / "run"
    temporary = output / "tmp"
    runtime.mkdir(parents=True, exist_ok=True)
    temporary.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, TMP=str(temporary), TEMP=str(temporary), TMPDIR=str(temporary))
    command = [sys.executable, str(ROOT / "foundation/boot/build.py"), str(output)]
    if args.clang:
        command += ["--clang", args.clang]
    command += ["-j", str(args.jobs if args.jobs and args.jobs > 0 else physical_cores())]
    if args.root:
        command += ["--root", str(args.root.resolve())]
    for directory in args.bin_dir:
        command += ["--bin-dir", str(directory.resolve())]
    if args.disk_blocks is not None:
        command += ["--disk-blocks", str(args.disk_blocks)]
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    subprocess.run(["cargo", "build", "--release", "--bin", "cpt32"],
                   cwd=ROOT, env=environment, check=True)

    # Config is relative to the emulator's cwd. Keep Foundation's font and
    # requested RAM, without the user's BIOS overriding the WAX entry point.
    # foundation.img is always the HDD in dd0: fs mounts it as "/" and init
    # starts /bin/shell.wax from it, and the fs service writes to it. --disk
    # also configures the BIOS and passes a zero word as the program, so the
    # BIOS skips the RAM boot and boots the disk through stage1.
    font = ROOT / "assets/UNSCII.F08"
    if not font.is_file():
        raise FileNotFoundError(font)
    config = ("cpu:\n  microarchitecture: cached-relaxed\n"
              f"memory:\n  ram_mib: {args.ram_mib}\n"
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
