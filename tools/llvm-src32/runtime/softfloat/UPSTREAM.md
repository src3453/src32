# Vendored Berkeley SoftFloat Release 3e subset (floating-point runtime)

- Source: https://github.com/ucb-bar/berkeley-softfloat-3, revision
  `b64af41c3276f97f0e181920400ee056b9c88037` (Release 3e, 2018-01-20).
- License: BSD 3-clause, retained in `COPYING.txt` and every source file.

`source/` holds unmodified files of that revision: the binary32 and binary64
arithmetic, comparison, square root, fused multiply-add, remainder and
conversion routines, the `8086` specialization, and the primitives of the
non-`SOFTFLOAT_FAST_INT64` build (the file list is the one of the upstream
`build/Linux-386-GCC/Makefile`, minus the binary16, extFloat80 and binary128
files). It is not the full distribution.

`port/platform.h` and `rt.c` are SRC32-specific. `platform.h` is the same as
`examples/softfloat-c/port/platform.h` (big endian, out-of-line primitives,
no host intrinsics); `rt.c` implements the compiler-rt compatible routines
the SRC32 backend calls for floating-point instructions (see `fp.py`) and
`fmod`/`sqrt`/`fma`. The `fmod` bit-pattern algorithm follows the well known
musl (MIT) one, written independently.

Why Berkeley SoftFloat rather than a compiler-rt port: it is bit-exact IEEE-754
(all four rounding modes, flags, subnormals, NaNs), BSD-3-Clause like the
copy already shipped with `examples/softfloat-c`, and the build without
`SOFTFLOAT_FAST_INT64` needs only 32- and 64-bit integer arithmetic. compiler-rt's
builtins (Apache-2.0 WITH LLVM-exception) need `__int128` for some routines
and a port per routine.

The driver compiles every file here at `-O2` when a program needs floating
point, and the backend emits only the functions that program reaches. To
update, check the upstream revision, the file list above, and rerun
`python -m unittest discover -s tools/llvm-src32` at all three optimization levels.
