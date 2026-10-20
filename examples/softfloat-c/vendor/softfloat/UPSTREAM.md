# Vendored Berkeley SoftFloat Release 3e subset

- Requested repository: https://github.com/InfiniBrains/softfloat
- Parent revision: `5f544d63f2b9859c7105c49935c800e5c0e717b4`
- Source: its `berkeley-sw` submodule, https://github.com/ucb-bar/berkeley-softfloat-3
- Submodule revision: `b64af41c3276f97f0e181920400ee056b9c88037`
- Release: 3e (`README.txt`, 2018-01-20)
- License: BSD 3-clause, retained in `COPYING.txt` and each source file.

All files copied from that submodule are unmodified. This directory contains
the original `README.txt`, `COPYING.txt`, `source/include/*.h`, and the binary32
arithmetic/int32 conversion source subset explicitly listed in the demo's
`CMakeLists.txt`, including its `8086` specialization and portable primitives.
It is not the full SoftFloat distribution; declarations for other formats in
the upstream headers do not imply that those APIs are linked here.

The SRC32-specific `platform.h` lives outside this directory, in `../../port/`.
The parent repository's C++ wrapper, float8 extensions and host build files are
not part of this demo. Updating requires checking the submodule version,
dependencies, license, endian configuration, and the guest regression at all
three optimization levels.
