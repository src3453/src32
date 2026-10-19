# TinyBASIC in C (LLVM/SRC32)

`main.c` is a freestanding C port of the complete root [`tinybasic.sol`](../../tinybasic.sol) interpreter, using the independent `tools/llvm-src32` Clang/LLVM IR-to-SRC32 toolchain. It does not use `csc` or a host C runtime. It retains the lexer, precedence-climbing parser, fixed-address AST and string pools, program-line directory, REPL, expression evaluator, statements and control stacks, keyboard mapping, PCG console, PeC RNG, and the original IDC/DMA disk transfer path.

`SAVE` keeps the original requirement for both LBA and sector count. `FOR` executes its body once before `NEXT`, and `LOAD` and `NEW` stop a running program. `BYE`/`QUIT` and an idle input reader use the SDK's HALT intrinsic. The reader drains PeC events queued before it sleeps and after it wakes; IRQ3 dispatch acknowledges the source so a FIFO level left high by an earlier key cannot strand the REPL.

## Requirements

- LLVM/Clang 20 and Python 3.11 or later with the toolchain dependencies. For the tested Windows LLVM version and dependency setup, see [`3d-cube-c/README.md`](../3d-cube-c/README.md#必要なツール).
- CMake 3.20 or later and Ninja.
- `src32_testbench` (built from the repository root) or CPT32 for running the WAX guest image.

## Build

Run from the repository root; use the actual Clang and Python executable paths for your environment.

```sh
cmake -S examples/tinybasic-c -B target/tinybasic-c -G Ninja \
  -DSRC32_PYTHON_EXECUTABLE=/path/to/python \
  -DSRC32_CLANG="C:/Program Files/LLVM/bin/clang.exe" \
  -DCMAKE_MAKE_PROGRAM=/path/to/ninja
cmake --build target/tinybasic-c --target tinybasic
```

The output is `target/tinybasic-c/tinybasic.wax`. `OPTIMIZATION 0` is intentional. The linker uses origin 0 and stack top `0x00100000`; the program's fixed input, line, variable, array, disk, AST, and string-pool regions match the addresses in `tinybasic.sol`.

## Run

The emulator must have a slot-0 disk configured for `LOAD` and `SAVE`.

```sh
/absolute/path/src32_testbench /absolute/path/tinybasic.wax --allow-running --frames 120
```

Or launch the guest in the emulator:

```sh
cargo run --release --bin cpt32 -- target/tinybasic-c/tinybasic.wax
```

The program initializes PCG text mode, PeC RNG, and IRQ3 before displaying the help banner and entering the `OK` / `> ` REPL.
