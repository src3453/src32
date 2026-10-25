#!/usr/bin/env python3
"""Build a private Foundation disk with external DOOMGeneric/Newlib and a local WAD."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/fs2"))
import fs2  # noqa: E402


def install(disk, executable, wad):
    image = executable.read_bytes()
    volume = fs2.Volume(bytearray(disk.read_bytes()))
    library = volume.read(volume.resolve("/lib/fnd.1.wal"))
    if (len(image) + 3) // 4 * 4 + len(library) > 0x300000:
        raise ValueError("doom.wax and fnd.1.wal exceed the 3 MiB spawn staging area")
    with wad.open("rb") as stream:
        if stream.read(4) not in (b"IWAD", b"PWAD"):
            raise ValueError("WAD must start with IWAD or PWAD")
    volume.put("/bin/doom.wax", image, mode=0x3D)
    volume.put("/doom.wad", wad.read_bytes(), mode=0x24)  # owner/other read
    errors = fs2.check_image(volume.img)
    if errors:
        raise ValueError(f"FS2 integrity check failed: {errors}")
    disk.write_bytes(volume.img)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--doomgeneric-source", type=Path, required=True)
    p.add_argument("--newlib-source", type=Path, required=True)
    p.add_argument("--wad", type=Path, required=True)
    p.add_argument("--out", type=Path, default=ROOT / "target/foundation-doom")
    p.add_argument("--clang")
    p.add_argument("-j", "--jobs", type=int, default=4)
    p.add_argument("--build-only", action="store_true")
    args = p.parse_args(argv)
    out = args.out.resolve()
    boot = out / "boot"
    runtime = out / "run"
    temp = out / "tmp"
    runtime.mkdir(parents=True, exist_ok=True)
    temp.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TEMP=str(temp), TMP=str(temp), TMPDIR=str(temp))
    extra = ["--clang", args.clang] if args.clang else []
    subprocess.run([sys.executable, str(ROOT / "foundation/boot/build.py"), str(boot),
                    "--disk-blocks", "131072", "-j", str(args.jobs), *extra], env=env, check=True)
    subprocess.run([sys.executable, str(Path(__file__).with_name("build.py")),
                    "--doomgeneric-source", str(args.doomgeneric_source.resolve()),
                    "--newlib-source", str(args.newlib_source.resolve()),
                    "--fnd-wal", str(boot / "root/lib/fnd.1.wal"), "--out", str(out),
                    "-j", str(args.jobs), *extra], env=env, check=True)
    disk = boot / "foundation.img"
    install(disk, out / "doom.wax", args.wad.resolve())
    config = ("cpu:\n  microarchitecture: cached-relaxed\n"
              "memory:\n  ram_mib: 64\n  vram_mib: 4\n"
              f"pcg:\n  font: {json.dumps(str(ROOT / 'assets/UNSCII.F08'))}\n"
              f"disk:\n  - slot: 0\n    type: hdd\n    image: {json.dumps(str(disk))}\n"
              "    read_only: false\n")
    (runtime / "config.yaml").write_text(config, encoding="utf-8")
    print(f"Disk: {disk}\nShell command: doom -iwad /doom.wad", flush=True)
    if not args.build_only:
        subprocess.run(["cargo", "build", "--release", "--bin", "cpt32"], cwd=ROOT, env=env, check=True)
        exe = ROOT / "target/release" / ("cpt32.exe" if os.name == "nt" else "cpt32")
        return subprocess.run([str(exe), str(boot / "foundation.wax")], cwd=runtime, env=env).returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
