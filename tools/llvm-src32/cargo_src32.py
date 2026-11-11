#!/usr/bin/env python3
"""Builds a no_std Rust crate with Cargo and compiles it for SRC32 with driver.py.

    python tools/llvm-src32/cargo_src32.py [--manifest-path Cargo.toml] [cargo options]
                                           [driver options] -o OUT.wax

rustc emits LLVM IR for the 32-bit big-endian MIPS o32 layout (the same one
Clang uses for C, rust/src32-none.json); it never generates MIPS code. The
crate's library target is built as a staticlib with fat LTO, so the IR that
rustc writes after LTO holds the crate, `core` (and `alloc`) and every
dependency in one module. That module, plus any C sources given here, goes to
driver.py, which links the runtime and makes the WAX / WAX v3 / WAL image.

Options this script takes (the rest go to driver.py unchanged):

  --manifest-path PATH   the crate (default: ./Cargo.toml)
  --profile NAME         Cargo profile (default: release); panic=abort, lto=fat and
                         codegen-units=1 are forced for it
  --features LIST        Cargo features; --no-default-features also passes through
  --build-std LIST       standard crates to build from rust-src (default: core;
                         use core,alloc for `alloc`)
  --target-dir DIR       Cargo target directory (default: Cargo's)
  --cargo PATH           Cargo executable (default: CARGO, then PATH)
  --emit-rust-ir PATH    also copy the post-LTO IR there
  --foundation-wal WAL   a Foundation program: shorthand for the driver options
                         --wax3 --page-align --pie --import-wal WAL (WAL is the
                         built /lib/fnd.1.wal, OUT/root/lib/fnd.1.wal of
                         foundation/boot/build.py)

Requirements: a Rust toolchain with the rust-src component whose LLVM major
version matches llvmlite's (the IR is parsed by llvmlite), and Clang for the
runtime (memcpy, SoftFloat). A stable toolchain is run with RUSTC_BOOTSTRAP=1,
because -Zbuild-std and JSON target specs are still unstable Cargo features.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
TARGET_SPEC = HERE / "rust" / "src32-none.json"
DRIVER = HERE / "driver.py"


class CargoError(Exception):
    pass


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    p.add_argument("--manifest-path", type=Path, default=Path("Cargo.toml"))
    p.add_argument("--profile", default="release")
    p.add_argument("--features", action="append", default=[])
    p.add_argument("--no-default-features", action="store_true")
    p.add_argument("--build-std", default="core")
    p.add_argument("--target-dir", type=Path)
    p.add_argument("--cargo")
    p.add_argument("--emit-rust-ir", type=Path, metavar="PATH")
    p.add_argument("--foundation-wal", type=Path, metavar="WAL")
    p.add_argument("-v", "--verbose", action="store_true", help="show the Cargo and driver commands")
    return p


def find_cargo(explicit: str | None) -> str:
    selected = explicit or os.environ.get("CARGO") or "cargo"
    found = shutil.which(selected)
    if not found:
        raise CargoError(f"Cargo not found: {selected!r}; install Rust (rustup) or pass --cargo PATH")
    return found


def rustc_info(cargo: str, cwd: Path) -> dict[str, str]:
    """`rustc -vV` of the toolchain Cargo runs in cwd (rustup overrides apply)."""
    rustc = os.environ.get("RUSTC")
    if not rustc:
        sibling = Path(cargo).with_name("rustc" + Path(cargo).suffix)
        rustc = str(sibling) if sibling.exists() else "rustc"
    try:
        result = subprocess.run([rustc, "-vV"], cwd=cwd, capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        raise CargoError(f"cannot run {rustc} -vV: {exc}") from exc
    info = {}
    for line in result.stdout.splitlines():
        key, _, value = line.partition(":")
        if value:
            info[key.strip()] = value.strip()
    info["sysroot"] = subprocess.run([rustc, "--print", "sysroot"], cwd=cwd, capture_output=True,
                                     text=True, check=True).stdout.strip()
    return info


def check_toolchain(info: dict[str, str]) -> None:
    try:
        import llvmlite.binding as llvm
    except ImportError:
        raise CargoError("llvmlite is required; python -m pip install -r tools/llvm-src32/requirements.txt") from None
    rust_llvm = info.get("LLVM version", "")
    match = re.match(r"(\d+)", rust_llvm)
    if not match:
        raise CargoError(f"rustc does not report its LLVM version ({info.get('release')})")
    if int(match.group(1)) != llvm.llvm_version_info[0]:
        raise CargoError(
            f"rustc {info.get('release')} uses LLVM {rust_llvm}, llvmlite uses LLVM "
            f"{'.'.join(map(str, llvm.llvm_version_info))}; the IR must come from the same LLVM major "
            "version (choose a Rust toolchain or an llvmlite release that matches)")
    library = Path(info["sysroot"]) / "lib" / "rustlib" / "src" / "rust" / "library" / "core"
    if not library.is_dir():
        raise CargoError("the rust-src component is missing; run: rustup component add rust-src")


def toolchain_problem(cargo: str | None = None, cwd: Path = HERE) -> str | None:
    """Why Rust guests cannot be built here, or None (tests skip on a reason)."""
    try:
        found = find_cargo(cargo)
        check_toolchain(rustc_info(found, cwd))
    except CargoError as exc:
        return str(exc)
    return None


def library_target(cargo: str, manifest: Path, env: dict[str, str]) -> tuple[str, str]:
    """(package name, library crate name) of the manifest's package."""
    result = subprocess.run([cargo, "metadata", "--no-deps", "--format-version", "1",
                             "--manifest-path", str(manifest)], capture_output=True, text=True, env=env)
    if result.returncode:
        raise CargoError(f"cargo metadata failed:\n{result.stderr.strip()}")
    metadata = json.loads(result.stdout)
    manifest_key = os.path.normcase(str(manifest.resolve()))
    for package in metadata["packages"]:
        if os.path.normcase(str(Path(package["manifest_path"]).resolve())) != manifest_key:
            continue
        for target in package["targets"]:
            if "lib" in target["kind"] or set(target["kind"]) & {"staticlib", "rlib", "cdylib"}:
                return package["name"], target["name"].replace("-", "_")
        raise CargoError(f"{manifest}: the package has no [lib] target (src/lib.rs); "
                         "SRC32 programs are libraries whose #[no_mangle] extern \"C\" fn main is the entry")
    raise CargoError(f"{manifest}: not a package manifest (a workspace root needs the member's manifest)")


def remove_dead_globals(source: Path, output: Path) -> None:
    """LLVM's globaldce on the post-LTO IR. Without optimisation (opt-level 0)
    rustc's LTO keeps every internal function of core, including ones on types
    the backend rejects (f16, i128), although nothing calls them."""
    import llvmlite.binding as llvm
    llvm.initialize_all_targets()
    llvm.initialize_all_asmprinters()
    try:
        module = llvm.parse_assembly(source.read_text(encoding="utf-8"))
        module.verify()
    except RuntimeError as exc:
        raise CargoError(f"{source}: LLVM parse/verification error:\n{exc}") from exc
    machine = llvm.Target.from_triple(module.triple).create_target_machine()
    builder = llvm.create_pass_builder(machine, llvm.create_pipeline_tuning_options(speed_level=0))
    passes = llvm.create_new_module_pass_manager()
    passes.add_global_dead_code_eliminate_pass()
    passes.run(module, builder)
    output.write_text(str(module), encoding="utf-8")


def cargo_build(args, cargo: str, manifest: Path, env: dict[str, str], ir: Path) -> None:
    profile_key = "profile." + args.profile
    command = [cargo, "rustc", "--manifest-path", str(manifest), "--profile", args.profile, "--lib",
               "--crate-type", "staticlib", "--target", str(TARGET_SPEC),
               "-Zjson-target-spec", f"-Zbuild-std={args.build_std}",
               "--config", f'{profile_key}.panic="abort"',
               "--config", f'{profile_key}.lto="fat"',
               "--config", f"{profile_key}.codegen-units=1",
               "--config", f"{profile_key}.debug=false"]
    for features in args.features:
        command += ["--features", features]
    if args.no_default_features:
        command.append("--no-default-features")
    if args.target_dir:
        command += ["--target-dir", str(args.target_dir)]
    # With fat LTO, rustc writes the IR of the whole program after LTO; the
    # dependencies' own IR is not needed (and is not written).
    command += ["--", f"--emit=llvm-ir={ir}"]
    if args.verbose:
        print(" ".join(command), file=sys.stderr)
    if subprocess.run(command, env=env).returncode:
        raise CargoError("cargo build failed")


def run(args, driver_args: list[str]) -> int:
    manifest = args.manifest_path.resolve()
    if not manifest.is_file():
        raise CargoError(f"manifest not found: {args.manifest_path}")
    cargo = find_cargo(args.cargo)
    info = rustc_info(cargo, manifest.parent)
    check_toolchain(info)
    env = dict(os.environ)
    if not re.search(r"-(nightly|dev)\b", info.get("release", "")):
        env["RUSTC_BOOTSTRAP"] = "1"  # -Zbuild-std / -Zjson-target-spec on a stable toolchain
    package, crate = library_target(cargo, manifest, env)
    if args.foundation_wal:
        if not args.foundation_wal.is_file():
            raise CargoError(f"--foundation-wal: {args.foundation_wal} not found; build Foundation first "
                             "(python foundation/boot/build.py OUT)")
        driver_args = ["--wax3", "--page-align", "--pie", "--import-wal", str(args.foundation_wal.resolve()),
                       *driver_args]
    if args.target_dir:
        target_dir = args.target_dir.resolve()
    else:
        result = subprocess.run([cargo, "metadata", "--no-deps", "--format-version", "1",
                                 "--manifest-path", str(manifest)], capture_output=True, text=True, env=env)
        target_dir = Path(json.loads(result.stdout)["target_directory"])
    # A fixed path: when Cargo finds the crate fresh it does not run rustc, and the IR from the
    # previous (identical) build is still the right one.
    ir = target_dir / TARGET_SPEC.stem / "src32-ir" / args.profile / f"{crate}.ll"
    ir.parent.mkdir(parents=True, exist_ok=True)
    cargo_build(args, cargo, manifest, env, ir)
    if not ir.is_file():
        # The IR was deleted while Cargo's build of the crate stayed fresh: rebuild the crate.
        clean = [cargo, "clean", "--manifest-path", str(manifest), "-p", package, "--profile", args.profile,
                 "--target", str(TARGET_SPEC), "-Zjson-target-spec"]
        if args.target_dir:
            clean += ["--target-dir", str(args.target_dir)]
        subprocess.run(clean, env=env, check=False)
        cargo_build(args, cargo, manifest, env, ir)
        if not ir.is_file():
            raise CargoError(f"rustc did not write {ir}")
    pruned = ir.with_suffix(".src32.ll")
    remove_dead_globals(ir, pruned)
    if args.emit_rust_ir:
        args.emit_rust_ir.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(pruned, args.emit_rust_ir)
    command = [sys.executable, str(DRIVER), str(pruned), *driver_args]
    if args.verbose:
        print(" ".join(command), file=sys.stderr)
    return subprocess.run(command).returncode


def main(argv=None) -> int:
    args, driver_args = parser().parse_known_args(argv)
    try:
        return run(args, driver_args)
    except CargoError as exc:
        print(f"cargo-src32: error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
