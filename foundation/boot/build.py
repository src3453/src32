"""Builds the Foundation boot images into OUT.

    python foundation/boot/build.py OUT [--clang PATH] [-O0|-O1|-O2] [--stop-ticks N]
                                        [--root DIR] [--bin-dir DIR ...] [--disk-blocks N]

The programs (doc/spec_Foundation.md §5.2). Every one is a relocatable WAX v3
(driver --pie, doc/spec_WAX_v3.md §3.1), which the kernel loads into any free
RAM, so none of them has an address of its own:

    init       starts /bin/shell.wax (services/init)
    console    the console service (services/console)
    disk       the disk service, blk_io for fs (services/disk)
    fs         the fs service (fs/)
    pipe       the pipe service (services/pipe)
    shell      /bin/shell.wax (doc/spec_Foundation_shell.md)
    /bin/*     the standard commands (bin/, §10.3 of the shell spec)

libfnd is the WAL "fnd" 1.3 (/lib/fnd.1.wal, exports in lib/fnd/fnd.exports),
and the shell and the commands are WAX v3 that import it; fnd_proc_exec
reads it from /lib when it starts them (§9.5). init and the services are
FNDBOOT modules (§11) with libfnd linked in, since the kernel starts them
before there is an fs.

The four modules (init, console, disk, fs) go into FNDBOOT (§11); /bin and /lib are
files on the boot disk, which fs serves as "/".

Environment: FND_MM=nommu builds the kernel for physical memory only, which
is what a CPU without Extension V runs; otherwise it uses virtual memory when
CPUID reports V (doc/foundation_vm_plan.md).

Outputs:
  root/           /bin/*.wax (the shell and the commands), /lib/fnd.1.wal, /lib/lzma.1.wal, /lib/rle.1.wal, /lib/code.1.wal and /lib/sgc.1.wal
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
import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess
from concurrent.futures import ThreadPoolExecutor
import sys

HERE = Path(__file__).resolve().parent
FOUNDATION = HERE.parent
ROOT = FOUNDATION.parent
DRIVER = ROOT / "tools/llvm-src32/driver.py"
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "tools/fs2"))
sys.path.insert(0, str(ROOT / "tools/wax"))
import fndboot  # noqa: E402
import wide_font  # noqa: E402
import fs2  # noqa: E402
import wax  # noqa: E402

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

COMMANDS = ["cat", "clear", "cp", "echo", "free", "hex", "ls", "mkdir", "mode", "mv", "ps", "pwd", "rm", "rmdir", "top",
            "uname", "tinybasic", "chip8", "bmpview", "sgubplay", "wavplay"]

INCLUDES = ["-I", str(ROOT / "sdk/cpt32/include"), "-I", str(FOUNDATION / "include"),
            "-I", str(FOUNDATION / "lib/fnd")]
# The toolchain links every function it is given, so the static programs
# that start no others leave out proc.c.
LIBFND = [FOUNDATION / "lib/fnd" / name for name in ("start.c", "mem.c", "strerror.c")]
LIBFND_PROC = FOUNDATION / "lib/fnd/proc.c"
# Only the WAL exports it; static users (fs, whose slot is nearly full) leave it out.
LIBFND_RENAME = FOUNDATION / "lib/fnd/rename.c"
KERNEL = [FOUNDATION / "kernel" / name for name in ("sched.c", "proc.c", "mm.c", "trap.c", "ipc.c", "fb.c", "sgu.c", "sgc.c", "pec.c", "blk.c", "wax.c",
                                                     "fndboot.c")]
LZMA_DIR = FOUNDATION / "lib/lzma"
SGC_DIR = FOUNDATION / "lib/sgc"
# Commands that import /lib/sgc.1.wal (libsgc) as well as fnd.
SGC_COMMANDS = {"chip8"}
RLE_DIR = FOUNDATION / "lib/rle"
CODE_DIR = FOUNDATION / "lib/code"
FS = [FOUNDATION / "fs" / name for name in ("main.c", "fs2.c", "vfs.c", "server.c", "dsfs.c")]
# KAN-38 temporary boot mitigation. Set to 1 to restore the full §10 scan.
FS2_FULL_MOUNT_CHECK = 0
ASSEMBLER = ROOT / "tools/asm/asm.py"


def driver(sources, output, args, *extra):
    # Parallelism is across driver processes (main), so each one compiles serially.
    command = [sys.executable, str(DRIVER), *map(str, sources), *INCLUDES, args.optimization,
               "-j", "1", *extra, "-o", str(output)]
    if args.clang:
        command[2:2] = ["--clang", args.clang]
    subprocess.run(command, check=True)
    return output


def layout(stack=STACK, heap=0):
    return ["--pie", "--user-stack-size", hex(stack), *(["--heap-hint", hex(heap)] if heap else [])]


def static(sources, output, args, *extra, stack=STACK, heap=0):
    """A WAX v3 with libfnd linked in (no imports)."""
    return driver([*sources, *LIBFND], output, args, "--wax3", "--page-align", *layout(stack, heap), *extra)


def program(sources, output, args, image_id, fnd, *libs, heap=0):
    """A WAX v3 that imports the WAL fnd, and any other WALs in libs."""
    imports = [arg for lib in (fnd, *libs) for arg in ("--import-wal", lib)]
    return driver(sources, output, args, "--wax3", "--page-align", "--image-id", image_id, *imports,
                  *layout(heap=heap))


def embed(name, data):
    """Big-endian words: the same bytes in SRC32 memory, 4-byte aligned."""
    padded = data + bytes(-len(data) % 4)
    words = ",".join("0x%Xu" % int.from_bytes(padded[i:i + 4], "big") for i in range(0, len(padded), 4))
    return (f"#define img_{name}_size {len(data)}u\n"
            f"static const uint32_t img_{name}[{len(padded) // 4}] = {{{words}}};\n")


def console_grants():
    """MMIO windows in con_pcg.h: VDP, PCG text, wide font, PeC input, UART."""
    text = (FOUNDATION / "services/console/con_pcg.h").read_text(encoding="utf-8")
    value = {name: int(number, 16) for name, number in re.findall(r"#define (CON_PCG_\w+) (0x[0-9A-Fa-f]+)u", text)}
    return [(value[f"CON_PCG_{window}_START"], value[f"CON_PCG_{window}_END"], 3)
            for window in ("VDP", "TEXT", "FONT", "INPUT", "UART")]


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


def overlay_files(files, additions):
    """Apply precedence using the disk's Unicode case folding, before sorting."""
    for path, payload in additions.items():
        key = fs2.fold(path)
        for old in list(files):
            if fs2.fold(old) == key:
                del files[old]
        files[path] = payload


def external_bins(directories):
    """Snapshot direct .wax children before the build can overwrite outputs."""
    files = {}
    for directory in directories:
        if not directory.is_dir():
            raise ValueError(f"--bin-dir is not a directory: {directory}")
        programs = sorted(p for p in directory.iterdir() if p.is_file() and p.suffix.lower() == ".wax")
        if not programs:
            raise ValueError(f"--bin-dir contains no .wax files: {directory}")
        overlay_files(files, {"/bin/" + program.name: (program.read_bytes(), MODE_EXEC) for program in programs})
    return files


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", type=Path)
    parser.add_argument("--clang")
    parser.add_argument("-j", "--jobs", type=int, default=0, metavar="N",
                        help="parallel driver processes (default: CPU count; the images do not depend on N)")
    parser.add_argument("--stop-ticks", type=int)
    parser.add_argument("--root", type=Path, help="host directory copied into the disk root (e.g. root/bin/*.wax)")
    parser.add_argument("--bin-dir", type=Path, action="append", default=[], metavar="DIR",
                        help="add direct *.wax children to /bin after --root; repeatable, later directories win")
    parser.add_argument("--disk-blocks", type=int, default=DISK_BLOCKS, help=f"disk size in blocks (default {DISK_BLOCKS})")
    level = parser.add_mutually_exclusive_group()
    for n in range(3):
        level.add_argument(f"-O{n}", dest="optimization", action="store_const", const=f"-O{n}")
    parser.set_defaults(optimization="-O1")
    args = parser.parse_args(argv)
    try:
        bins = external_bins(args.bin_dir)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    out = args.out
    root = out / "root"
    (root / "bin").mkdir(parents=True, exist_ok=True)
    (root / "lib").mkdir(parents=True, exist_ok=True)
    services = FOUNDATION / "services"

    jobs = args.jobs if args.jobs > 0 else (os.cpu_count() or 1)
    pool = ThreadPoolExecutor(max_workers=jobs)
    # Every image below is an independent driver run, except that /bin programs import the fnd WAL.
    fnd_job = pool.submit(driver, [*LIBFND, LIBFND_PROC, LIBFND_RENAME], root / "lib/fnd.1.wal", args, "--wal",
                          "--page-align", "--image-id", "fnd",
                          "--abi-version", "1.3", "--exports", FOUNDATION / "lib/fnd/fnd.exports")
    # /lib/lzma.1.wal: the LZMA SDK adapter (lib/lzma, WAL "lzma" 1.0). Self-contained: it needs
    # no libfnd and no heap, so it adds nothing to the fs heap; programs that import it pass an arena.
    lzma_spec = importlib.util.spec_from_file_location("lzma_build", LZMA_DIR / "build.py")
    lzma_build = importlib.util.module_from_spec(lzma_spec)
    lzma_spec.loader.exec_module(lzma_build)
    lzma_job = pool.submit(lzma_build.build, out / "lzma", args.clang, args.optimization)
    # /lib/rle.1.wal: PackBits RLE (lib/rle, WAL "rle" 1.0), also self-contained. Both follow conde.
    rle_spec = importlib.util.spec_from_file_location("rle_build", RLE_DIR / "build.py")
    rle_build = importlib.util.module_from_spec(rle_spec)
    rle_spec.loader.exec_module(rle_build)
    rle_job = pool.submit(rle_build.build, out / "rle", args.clang, args.optimization)
    # /lib/code.1.wal: character code conversion (lib/code, WAL "code" 1.0), also self-contained.
    code_spec = importlib.util.spec_from_file_location("code_build", CODE_DIR / "build.py")
    code_build = importlib.util.module_from_spec(code_spec)
    code_spec.loader.exec_module(code_build)
    code_job = pool.submit(code_build.build, out / "code", args.clang, args.optimization)

    kernel_args = ["--origin", hex(KERNEL_LOAD), "--stack-top", hex(KERNEL_STACK_TOP)]
    if args.stop_ticks is not None:
        kernel_args += ["-D", f"FOUNDATION_BOOT_STOP_TICKS={args.stop_ticks}u"]
    if os.environ.get("FND_MM") in ("nommu", "slot"):
        # Ignore Extension V and run on physical memory even on a CPU that has it
        # (doc/foundation_vm_plan.md); the tests use this to check both backends.
        kernel_args += ["-D", "FND_MM_NOMMU_ONLY"]
    image_job = pool.submit(driver, [HERE / "main.c", *KERNEL], out / "foundation.wax", args,
                            "-D", "FOUNDATION_BOOT_EMBEDDED", *kernel_args)
    kernel_job = pool.submit(driver, [HERE / "main.c", *KERNEL], out / "kernel.bin", args, *kernel_args)

    # §11: spawned in this order, init last.
    specs = [
        dict(name="console", priority=FND_PRIO_SERVICE, caps=FND_CAP_SVC, grants=console_grants(),
             image=pool.submit(static, [services / "console/main.c", services / "console/con_pcg.c",
                                      services / "console/con_wide.c", services / "console/con_uart.c"],
                               out / "console.wax", args, "-I", str(services / "console"),
                               "-I", str(wide_font.write_header(out / "console/con_wide_font.h").parent))),
        dict(name="disk", priority=FND_PRIO_SERVICE, caps=FND_CAP_DISK | FND_CAP_SVC,
             image=pool.submit(static, [services / "disk/main.c", services / "disk/cache.c"],
                               out / "disk.wax", args)),
        dict(name="fs", priority=FND_PRIO_SERVICE, caps=FND_CAP_SVC,
             image=pool.submit(static, [*FS, FOUNDATION / "lib/fnd/name.c"], out / "fs.wax", args,
                               f"-DFS2_FULL_MOUNT_CHECK={FS2_FULL_MOUNT_CHECK}",
                               "-I", str(FOUNDATION / "fs"), stack=FS_STACK)),
        dict(name="pipe", priority=FND_PRIO_SERVICE, caps=FND_CAP_SVC,
             image=pool.submit(static, [services / "pipe/main.c"], out / "pipe.wax", args)),
        dict(name="init", priority=FND_PRIO_SERVICE, caps=0,
             image=pool.submit(static, [services / "init/main.c", LIBFND_PROC], out / "init.wax", args)),
    ]

    fnd = fnd_job.result()
    # /lib/sgc.1.wal: libsgc (lib/sgc, WAL "sgc" 1.0), which imports the fnd WAL built above.
    sgc_spec = importlib.util.spec_from_file_location("sgc_build", SGC_DIR / "build.py")
    sgc_build = importlib.util.module_from_spec(sgc_spec)
    sgc_spec.loader.exec_module(sgc_build)
    sgc_job = pool.submit(sgc_build.build, out / "sgc", fnd, args.clang, args.optimization)
    program_jobs = [pool.submit(program, [FOUNDATION / "shell/shell.c"], root / "bin/shell.wax", args,
                                "foundation.shell", fnd)]
    # The commands that import libsgc go last, so the others start while it builds.
    for name in sorted(COMMANDS, key=lambda name: name in SGC_COMMANDS):
        libs = [sgc_job.result()] if name in SGC_COMMANDS else []
        sources = [FOUNDATION / "bin" / f"{name}.c", FOUNDATION / "bin/cmd.c"]
        if name == "sgubplay":
            sources.append(ROOT / "sdk/cpt32/src/sgub.c")
        program_jobs.append(pool.submit(program, sources,
                                        root / "bin" / f"{name}.wax", args, f"foundation.{name}", fnd, *libs))
    shutil.copyfile(lzma_job.result(), root / "lib/lzma.1.wal")
    shutil.copyfile(sgc_job.result(), root / "lib/sgc.1.wal")
    shutil.copyfile(rle_job.result(), root / "lib/rle.1.wal")
    shutil.copyfile(code_job.result(), root / "lib/code.1.wal")
    for job in program_jobs:
        job.result()
    modules = [dict(spec, image=spec["image"].result().read_bytes()) for spec in specs]
    pool.shutdown()

    image = image_job.result()
    # The module-only FNDBOOT goes where stage1 stages it (§11).
    parsed = wax.parse(image.read_bytes())
    blocks = [(b["load_address"], b["data"]) for b in parsed["blocks"]]
    blocks.append((FNDBOOT_STAGING, fndboot.pack(b"", modules)))
    image.write_bytes(wax.pack(parsed["base"], parsed["entry_offset"], blocks))
    print(f"Wrote {image}")

    kernel = kernel_job.result().read_bytes()
    boot = fndboot.pack(kernel, modules)
    (out / "fndboot.bin").write_bytes(boot)
    stage1 = out / "stage1.bin"
    subprocess.run([sys.executable, str(ASSEMBLER), "-o", str(stage1), str(HERE / "stage1.a")], check=True)
    files = {}
    for tree in (root, args.root):
        for host in sorted(p for p in tree.rglob("*") if p.is_file()) if tree else ():
            path = "/" + host.relative_to(tree).as_posix()
            files[path] = (host.read_bytes(), MODE_EXEC if path.startswith(("/bin/", "/lib/")) else None)
    overlay_files(files, bins)
    disk = out / "foundation.img"
    temporary = disk.with_name(disk.name + ".tmp")
    temporary.write_bytes(disk_image(stage1.read_bytes(), boot, files, args.disk_blocks))
    os.replace(temporary, disk)
    print(f"Wrote {disk}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
