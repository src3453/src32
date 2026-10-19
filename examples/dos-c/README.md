# CPT32 DOS in C (LLVM/SRC32)

`main.c` is a freestanding C port of the root [`dos.sol`](../../dos.sol) guest shell. It uses the independent `tools/llvm-src32` Clang/LLVM IR-to-SRC32 toolchain, not `csc`, and does not link host libc. The port retains the DOS command aliases, 8.3 names and drive-qualified copies, PeC keyboard event processing with IRQ3 wakeup, PCG text display and 40/80-column selection, DSFS file operations, the line and hexadecimal editors, DSFS FORMAT, WAX v2 validation/loading, and the original command/status messages. DSFS, IDC, WAX, IRQ and PCG use the shared [`sdk/cpt32`](../../sdk/cpt32/README.md); the guest directly accesses the PeC FIFO, RTC, VDP enable and text-plane/MMIO controls that the original guest uses directly.

## Requirements

- LLVM/Clang 20 (the bundled toolchain parses LLVM IR with `llvmlite 0.50`; on Windows, `winget install --id LLVM.LLVM --exact --version 20.1.8`).
- Python 3.11+ and dependencies: from repository root, `python -m pip install -r tools/llvm-src32/requirements.txt ninja`.
- CMake 3.20+ and Ninja. If Ninja is not on `PATH`, pass its executable as `CMAKE_MAKE_PROGRAM`.
- For the testbench, build `src32_testbench` with Cargo from the repository root.

## Build

Run from the repository root, supplying the actual Clang 20 executable. Python-installed Ninja can be selected explicitly when it is not on `PATH`.

```sh
cmake -S examples/dos-c -B target/dos-c -G Ninja \
  -DSRC32_PYTHON_EXECUTABLE=/path/to/python \
  -DSRC32_CLANG="C:/Program Files/LLVM/bin/clang.exe" \
  -DCMAKE_MAKE_PROGRAM=/path/to/ninja
cmake --build target/dos-c --target dos
```

The executable is `target/dos-c/dos.wax`. WAX retains the guest origin and entry point; use it rather than converting it to a raw binary. The project links display, IRQ controller, disk/DSFS and WAX SDK components with optimization level 0, matching the original guest's behavior. No host C standard library is used.

## Run

DOS expects an attached DSFS disk in drive slot 0. Create and populate a floppy image with the repository DSFS host utility:

```sh
python tools/dsfs/dsfs.py mkfs disk0.img --sectors 2880
python tools/dsfs/dsfs.py put disk0.img HELLO.TXT hello.txt
```

Start the guest in a directory with a disk configuration for slot 0 and no BIOS section:

```sh
/path/to/src32_testbench /absolute/path/to/target/dos-c/dos.wax --allow-running
```

For GUI use, attach the disk image to slot 0 and launch:

```sh
cargo run --release --bin cpt32 -- target/dos-c/dos.wax
```

The shell starts at drive 0. Type `HELP` for commands; `FORMAT [N]` formats a drive as DSFS. Available commands include `DIR`, `TYPE`, `RUN`, `NEW`, `REN`, `DEL`, `COPY`, `EDIT`, `HEX`, `FORMAT`, `DRV`, `MODE`, `FREE`, `CLS`, `HELP` and `EXIT` (with the original aliases). IRQ3 and the foreground readers drain PeC events into a 512-event guest ring. Readers check and drain an already-nonempty FIFO before HALT, then drain again after waking; this handles keys queued before the prompt begins waiting as well as normal IRQ wakeups. `EXIT`, `QUIT` and `BYE` print `BYE` and enter a permanent `cpt32_halt()` loop.
