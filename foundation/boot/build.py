"""Builds the Foundation boot images into OUT.

    python foundation/boot/build.py OUT [--clang PATH] [-O0|-O1|-O2] [--stop-ticks N]
                                        [--root DIR] [--disk-blocks N]

Outputs:
  shell.wax       the shell, a PROTECTED WAX v2 for slot 4 (doc/spec_Foundation_shell.md §1)
  foundation.wax  kernel + module-only FNDBOOT linked in (fndboot_image.h); the host
                  loads it directly (doc/spec_Foundation.md §11 without stage1)
  kernel.bin      kernel raw image for 0x00010000, entered by stage1
  fndboot.bin     FNDBOOT: kernel.bin and the module table (§11)
  stage1.bin      boot sector (foundation/boot/stage1.a)
  foundation.img  bootable FS2 volume (doc/spec_FS2.md): stage1 in block 0, FNDBOOT
                  in the reserved boot area, /bin/shell.wax, plus the files under --root

--stop-ticks makes the kernel stop with R1 = 42 at that tick (tests).
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

HERE = Path(__file__).resolve().parent
FOUNDATION = HERE.parent
ROOT = FOUNDATION.parent
DRIVER = ROOT / "tools/llvm-src32/driver.py"
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "tools/fs2"))
import fndboot  # noqa: E402
import fs2  # noqa: E402

# Slot 4 (doc/spec_Foundation.md §5.2) and the shell's stack.
SHELL_ORIGIN = 0x00600000
SHELL_STACK_TOP = 0x00800000
SHELL_STACK = 0x4000
FND_PRIO_APP = 2
KERNEL_LOAD = 0x00010000
KERNEL_STACK_TOP = 0x00100000
# Disk defaults: 8 MiB, with the reserved boot area sized for the largest
# FNDBOOT stage1 accepts (2 MiB), so a bigger kernel never needs a re-format.
DISK_BLOCKS = 16384
BOOT_BLOCKS = fndboot.MAX_SIZE // fs2.BLOCK
DISK_INODES = 256
MODE_EXEC = 0x003D  # owner RWX, other RX (doc/spec_FS2.md §8)

INCLUDES = ["-I", str(ROOT / "sdk/cpt32/include"), "-I", str(FOUNDATION / "include"),
            "-I", str(FOUNDATION / "lib/fnd")]
LIBFND = [FOUNDATION / "lib/fnd" / name for name in ("start.c", "con_pcg.c", "strerror.c")]
KERNEL = [FOUNDATION / "kernel" / name for name in ("sched.c", "proc.c", "trap.c", "wax.c", "fndboot.c")]
ASSEMBLER = ROOT / "tools/asm/asm.py"


def driver(sources, output, args, *extra):
    command = [sys.executable, str(DRIVER), *map(str, sources), *INCLUDES, args.optimization,
               *extra, "-o", str(output)]
    if args.clang:
        command[2:2] = ["--clang", args.clang]
    subprocess.run(command, check=True)
    return output


def embed(name, data):
    """Big-endian words: the same bytes in SRC32 memory, 4-byte aligned."""
    padded = data + bytes(-len(data) % 4)
    words = ",".join("0x%Xu" % int.from_bytes(padded[i:i + 4], "big") for i in range(0, len(padded), 4))
    return (f"#define img_{name}_size {len(data)}u\n"
            f"static const uint32_t img_{name}[{len(padded) // 4}] = {{{words}}};\n")


def console_grants():
    """The shell's provisional MMIO grants: the windows in con_pcg.h (VDP, PCG text, PeC input)."""
    text = (FOUNDATION / "lib/fnd/con_pcg.h").read_text(encoding="utf-8")
    value = {name: int(number, 16) for name, number in re.findall(r"#define (CON_PCG_\w+) (0x[0-9A-Fa-f]+)u", text)}
    return [(value[f"CON_PCG_{window}_START"], value[f"CON_PCG_{window}_END"], 3)
            for window in ("VDP", "TEXT", "INPUT")]


def disk_image(stage1, boot, files, blocks):
    """An FS2 volume: stage1 in block 0, FNDBOOT in the reserved area, files = {path: (bytes, mode)}."""
    img = fs2.format_image(blocks, BOOT_BLOCKS, DISK_INODES)
    volume = fs2.Volume(img)
    volume.install_boot(boot, stage1)
    for path, (data, mode) in sorted(files.items()):
        volume.mkdir(path.rsplit("/", 1)[0] or "/", parents=True)
        volume.put(path, data, mode=mode)
    problems = fs2.check_image(img)
    if problems:
        raise RuntimeError(f"generated image does not mount: {problems[0]}")
    return img


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", type=Path)
    parser.add_argument("--clang")
    parser.add_argument("--stop-ticks", type=int)
    parser.add_argument("--root", type=Path, help="host directory copied into the disk root (e.g. root/bin/*.wax)")
    parser.add_argument("--disk-blocks", type=int, default=DISK_BLOCKS, help=f"disk size in blocks (default {DISK_BLOCKS})")
    level = parser.add_mutually_exclusive_group()
    for n in range(3):
        level.add_argument(f"-O{n}", dest="optimization", action="store_const", const=f"-O{n}")
    parser.set_defaults(optimization="-O1")
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)

    shell = driver([FOUNDATION / "shell/shell.c", *LIBFND], args.out / "shell.wax", args, "--protected",
                   "--origin", hex(SHELL_ORIGIN), "--stack-top", hex(SHELL_STACK_TOP),
                   "--user-stack-size", hex(SHELL_STACK)).read_bytes()
    modules = [dict(name="shell", image=shell, priority=FND_PRIO_APP, grants=console_grants())]
    kernel_args = ["--origin", hex(KERNEL_LOAD), "--stack-top", hex(KERNEL_STACK_TOP)]
    if args.stop_ticks is not None:
        kernel_args += ["-D", f"FOUNDATION_BOOT_STOP_TICKS={args.stop_ticks}u"]

    header = ["/* Generated by foundation/boot/build.py. */", "#include <stdint.h>",
              embed("fndboot", fndboot.pack(b"", modules))]
    (args.out / "fndboot_image.h").write_text("\n".join(header), encoding="utf-8")
    image = driver([HERE / "main.c", *KERNEL], args.out / "foundation.wax", args, "-I", str(args.out),
                   "-D", "FOUNDATION_BOOT_EMBEDDED", *kernel_args)
    print(f"Wrote {image}")

    kernel = driver([HERE / "main.c", *KERNEL], args.out / "kernel.bin", args, *kernel_args).read_bytes()
    boot = fndboot.pack(kernel, modules)
    (args.out / "fndboot.bin").write_bytes(boot)
    stage1 = args.out / "stage1.bin"
    subprocess.run([sys.executable, str(ASSEMBLER), "-o", str(stage1), str(HERE / "stage1.a")], check=True)
    files = {"/bin/shell.wax": (shell, MODE_EXEC)}
    if args.root:
        for host in sorted(p for p in args.root.rglob("*") if p.is_file()):
            path = "/" + host.relative_to(args.root).as_posix()
            files[path] = (host.read_bytes(), MODE_EXEC if path.startswith("/bin/") else None)
    disk = args.out / "foundation.img"
    temporary = disk.with_name(disk.name + ".tmp")
    temporary.write_bytes(disk_image(stage1.read_bytes(), boot, files, args.disk_blocks))
    os.replace(temporary, disk)
    print(f"Wrote {disk}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
