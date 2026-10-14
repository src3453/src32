#!/usr/bin/env python3
"""Build a CPT32 .cpt32card ZIP package from a WASM module and optional assets."""

import argparse
import json
from pathlib import Path, PurePosixPath
import sys
import zipfile

ASSET_LIMIT = 64 * 1024 * 1024


def archive_path(value: str) -> str:
    if not value or "\\" in value or value.startswith("/"):
        raise argparse.ArgumentTypeError(f"invalid package path: {value!r}")
    parts = value.split("/")
    if any(part in ("", ".", "..") for part in parts):
        raise argparse.ArgumentTypeError(f"invalid package path: {value!r}")
    return str(PurePosixPath(value))


def parse_asset(value: str) -> tuple[str, Path]:
    try:
        package_name, source_name = value.split("=", 1)
    except ValueError as error:
        raise argparse.ArgumentTypeError("asset must be PACKAGE_PATH=SOURCE_FILE") from error
    try:
        return archive_path(package_name), Path(source_name)
    except argparse.ArgumentTypeError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("module", type=Path, help="compiled WASM module")
    parser.add_argument("output", type=Path, help="output .cpt32card path")
    parser.add_argument("--module-path", type=archive_path, default="wasm/card.wasm")
    parser.add_argument(
        "--asset",
        action="append",
        type=parse_asset,
        default=[],
        metavar="PACKAGE_PATH=SOURCE_FILE",
        help="include an asset; IDs are assigned in command-line order",
    )
    args = parser.parse_args()

    try:
        if args.module_path == "manifest.yaml":
            raise ValueError("module path must not be manifest.yaml")
        if not args.module.is_file():
            raise ValueError(f"WASM module is not a file: {args.module}")
        output_path = args.output.resolve()
        if output_path == args.module.resolve():
            raise ValueError("output path must not overwrite the WASM module")
        destinations = {"manifest.yaml", args.module_path}
        total_asset_size = 0
        assets = []
        for destination, source in args.asset:
            if destination in destinations:
                raise ValueError(f"duplicate or reserved package path: {destination}")
            if not source.is_file():
                raise ValueError(f"asset is not a file: {source}")
            if output_path == source.resolve():
                raise ValueError(f"output path must not overwrite asset {source}")
            destinations.add(destination)
            size = source.stat().st_size
            total_asset_size += size
            if total_asset_size > ASSET_LIMIT:
                raise ValueError("assets exceed the 64 MiB uncompressed package limit")
            assets.append((destination, source))

        manifest = ["abi_version: 1", f"module: {json.dumps(args.module_path)}"]
        if assets:
            manifest.append("asset:")
            for asset_id, (destination, _) in enumerate(assets):
                manifest.extend(
                    [f"  - id: {asset_id}", f"    path: {json.dumps(destination)}"]
                )
        manifest_bytes = ("\n".join(manifest) + "\n").encode("utf-8")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(
            args.output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
        ) as package:
            package.writestr("manifest.yaml", manifest_bytes)
            package.write(args.module, args.module_path)
            for destination, source in assets:
                package.write(source, destination)
        print(f"Created {args.output} ({len(assets)} asset(s))")
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"package.py: error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
