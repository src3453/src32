"""Builds the Foundation boot images into OUT.

    python foundation/boot/build.py OUT [--clang PATH] [-O0|-O1|-O2] [--stop-ticks N]
                                        [--root DIR] [--disk-blocks N]

The programs, by slot (doc/spec_Foundation.md §5.2):

    slot 0  init       starts /bin/shell.wax (services/init)
    slot 1  console    the console service (services/console)
    slot 2  disk       the disk service, blk_io for fs (services/disk)
    slot 3  fs         the fs service (fs/)
    slot 4  shell      /bin/shell.wax (doc/spec_Foundation_shell.md)
    slot 5  /bin/*     the standard commands (bin/, §10.3 of the shell spec)

libfnd is the WAL "fnd" 1.1 (/lib/fnd.1.wal, exports in lib/fnd/fnd.exports),
and the shell and the commands are WAX v3 that import it; fnd_proc_exec
reads it from /lib when it starts them (§9.5). init and the services are
FNDBOOT modules (§11), PROTECTED WAX v2 with libfnd linked in, since the
kernel starts them before there is an fs.

The four modules (slots 0 to 3) go into FNDBOOT (§11); /bin and /lib are
files on the boot disk, which fs serves as "/".

Environment: FND_MM=slot builds the kernel for fixed slots only, which is
what a CPU without Extension V runs; otherwise it uses virtual memory when
CPUID reports V (doc/foundation_vm_plan.md).

Outputs:
  root/           /bin/*.wax (the shell and the commands) and /lib/fnd.1.wal
  foundation.wax  kernel + module-only FNDBOOT as a block at 0x200000; the host
                  loads it directly (§11 without stage1) and fs mounts IDC slot 0 as "/"
  kernel.bin      kernel raw image for 0x00010000, entered by stage1
  fndboot.bin     FNDBOOT: kernel.bin and the module table (§11)
  stage1.bin      boot sector (foundation/boot/stage1.a)
  foundation.img  bootable FS2 volume (doc/spec_FS2.md): stage1 in block 0, FNDBOOT
                  in the reserved boot area, root/, plus the files under --root

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
sys.path.insert(0, str(ROOT / "tools/wax"))
import fndboot  # noqa: E402
import fs2  # noqa: E402
import wax  # noqa: E402

# (origin, stack top) of the user slots used here.
SLOT0 = (0x00400000, 0x00480000)
SLOT1 = (0x00480000, 0x00500000)
SLOT2 = (0x00500000, 0x00580000)
SLOT3 = (0x00580000, 0x00600000)
SLOT4 = (0x00600000, 0x00800000)
SLOT5 = (0x00800000, 0x00C00000)
STACK = 0x4000
FS_STACK = 0x8000
FND_PRIO_SERVICE = 1
FND_CAP_DISK = 0x4  # include/foundation/ipc.h
FND_CAP_SVC = 0x8
KERNEL_LOAD = 0x00010000
KERNEL_STACK_TOP = 0x00100000
FNDBOOT_STAGING = 0x00200000  # where stage1 stages FNDBOOT (§11)
# Disk defaults: 8 MiB, with the reserved boot area sized for the largest
# FNDBOOT stage1 accepts (2 MiB), so a bigger kernel never needs a re-format.
DISK_BLOCKS = 16384
BOOT_BLOCKS = fndboot.MAX_SIZE // fs2.BLOCK
DISK_INODES = 256
MODE_EXEC = 0x003D  # owner RWX, other RX (doc/spec_FS2.md §8): /bin and /lib

COMMANDS = ["clear", "echo", "free", "ls", "ps", "pwd", "top", "uname"]

INCLUDES = ["-I", str(ROOT / "sdk/cpt32/include"), "-I", str(FOUNDATION / "include"),
            "-I", str(FOUNDATION / "lib/fnd")]
# The toolchain links every function it is given, so the static programs
# that start no others leave out proc.c.
LIBFND = [FOUNDATION / "lib/fnd" / name for name in ("start.c", "mem.c", "strerror.c")]
LIBFND_PROC = FOUNDATION / "lib/fnd/proc.c"
KERNEL = [FOUNDATION / "kernel" / name for name in ("sched.c", "proc.c", "mm.c", "trap.c", "ipc.c", "blk.c", "wax.c",
                                                     "fndboot.c")]
FS = [FOUNDATION / "fs" / name for name in ("main.c", "fs2.c", "vfs.c", "server.c", "dsfs.c")]
# KAN-38 temporary boot mitigation. Set to 1 to restore the full §10 scan.
FS2_FULL_MOUNT_CHECK = 0
ASSEMBLER = ROOT / "tools/asm/asm.py"


def driver(sources, output, args, *extra):
    command = [sys.executable, str(DRIVER), *map(str, sources), *INCLUDES, args.optimization,
               *extra, "-o", str(output)]
    if args.clang:
        command[2:2] = ["--clang", args.clang]
    subprocess.run(command, check=True)
    return output


def layout(slot, stack=STACK):
    origin, stack_top = slot
    return ["--origin", hex(origin), "--stack-top", hex(stack_top), "--user-stack-size", hex(stack)]


def static(sources, output, args, slot, *extra, stack=STACK):
    """A PROTECTED WAX v2 with libfnd linked in."""
    return driver([*sources, *LIBFND], output, args, "--protected", *layout(slot, stack), *extra)


def program(sources, output, args, slot, image_id, fnd):
    """A WAX v3 that imports the WAL fnd."""
    return driver(sources, output, args, "--wax3", "--image-id", image_id, "--import-wal", fnd, *layout(slot))


def embed(name, data):
    """Big-endian words: the same bytes in SRC32 memory, 4-byte aligned."""
    padded = data + bytes(-len(data) % 4)
    words = ",".join("0x%Xu" % int.from_bytes(padded[i:i + 4], "big") for i in range(0, len(padded), 4))
    return (f"#define img_{name}_size {len(data)}u\n"
            f"static const uint32_t img_{name}[{len(padded) // 4}] = {{{words}}};\n")


def console_grants():
    """The console service's MMIO grants: the windows in con_pcg.h (VDP, PCG text, PeC input)."""
    text = (FOUNDATION / "services/console/con_pcg.h").read_text(encoding="utf-8")
    value = {name: int(number, 16) for name, number in re.findall(r"#define (CON_PCG_\w+) (0x[0-9A-Fa-f]+)u", text)}
    return [(value[f"CON_PCG_{window}_START"], value[f"CON_PCG_{window}_END"], 3)
            for window in ("VDP", "TEXT", "INPUT")]


def header(path, parts):
    path.write_text("\n".join(["/* Generated by foundation/boot/build.py. */", "#include <stdint.h>",
                               *parts]), encoding="utf-8")


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
    out = args.out
    root = out / "root"
    (root / "bin").mkdir(parents=True, exist_ok=True)
    (root / "lib").mkdir(parents=True, exist_ok=True)
    services = FOUNDATION / "services"

    fnd = driver([*LIBFND, LIBFND_PROC], root / "lib/fnd.1.wal", args, "--wal", "--image-id", "fnd",
                 "--abi-version", "1.1", "--exports", FOUNDATION / "lib/fnd/fnd.exports")
    program([FOUNDATION / "shell/shell.c"], root / "bin/shell.wax", args, SLOT4, "foundation.shell", fnd)
    for name in COMMANDS:
        program([FOUNDATION / "bin" / f"{name}.c", FOUNDATION / "bin/cmd.c"], root / "bin" / f"{name}.wax",
                args, SLOT5, f"foundation.{name}", fnd)

    # §11: spawned in this order, init last.
    modules = [
        dict(name="console", priority=FND_PRIO_SERVICE, caps=FND_CAP_SVC, grants=console_grants(),
             image=static([services / "console/main.c", services / "console/con_pcg.c"], out / "console.wax",
                        args, SLOT1, "-I", str(services / "console"))),
        dict(name="disk", priority=FND_PRIO_SERVICE, caps=FND_CAP_DISK | FND_CAP_SVC,
             image=static([services / "disk/main.c"], out / "disk.wax", args, SLOT2)),
        dict(name="fs", priority=FND_PRIO_SERVICE, caps=FND_CAP_SVC,
             image=static([*FS, FOUNDATION / "lib/fnd/name.c"], out / "fs.wax", args, SLOT3,
                        f"-DFS2_FULL_MOUNT_CHECK={FS2_FULL_MOUNT_CHECK}",
                        "-I", str(FOUNDATION / "fs"), stack=FS_STACK)),
        dict(name="init", priority=FND_PRIO_SERVICE, caps=0,
             image=static([services / "init/main.c", LIBFND_PROC], out / "init.wax", args, SLOT0)),
    ]
    for module in modules:
        module["image"] = module["image"].read_bytes()

    kernel_args = ["--origin", hex(KERNEL_LOAD), "--stack-top", hex(KERNEL_STACK_TOP)]
    if args.stop_ticks is not None:
        kernel_args += ["-D", f"FOUNDATION_BOOT_STOP_TICKS={args.stop_ticks}u"]
    if os.environ.get("FND_MM") == "slot":
        # Ignore Extension V and run on fixed slots even on a CPU that has it
        # (doc/foundation_vm_plan.md); the tests use this to check both backends.
        kernel_args += ["-D", "FND_MM_SLOT_ONLY"]
    image = driver([HERE / "main.c", *KERNEL], out / "foundation.wax", args,
                   "-D", "FOUNDATION_BOOT_EMBEDDED", *kernel_args)
    # The module-only FNDBOOT goes where stage1 stages it (§11).
    parsed = wax.parse(image.read_bytes())
    blocks = [(b["load_address"], b["data"]) for b in parsed["blocks"]]
    blocks.append((FNDBOOT_STAGING, fndboot.pack(b"", modules)))
    image.write_bytes(wax.pack(parsed["base"], parsed["entry_offset"], blocks))
    print(f"Wrote {image}")

    kernel = driver([HERE / "main.c", *KERNEL], out / "kernel.bin", args, *kernel_args).read_bytes()
    boot = fndboot.pack(kernel, modules)
    (out / "fndboot.bin").write_bytes(boot)
    stage1 = out / "stage1.bin"
    subprocess.run([sys.executable, str(ASSEMBLER), "-o", str(stage1), str(HERE / "stage1.a")], check=True)
    files = {}
    for tree in (root, args.root):
        for host in sorted(p for p in tree.rglob("*") if p.is_file()) if tree else ():
            path = "/" + host.relative_to(tree).as_posix()
            files[path] = (host.read_bytes(), MODE_EXEC if path.startswith(("/bin/", "/lib/")) else None)
    disk = out / "foundation.img"
    temporary = disk.with_name(disk.name + ".tmp")
    temporary.write_bytes(disk_image(stage1.read_bytes(), boot, files, args.disk_blocks))
    os.replace(temporary, disk)
    print(f"Wrote {disk}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
