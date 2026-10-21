"""Generate distribution notices from resolved Cargo and npm packages.

No network requests: cargo fetch/npm ci must be run beforehand. Upstream
notices omitted from crate archives are reviewed and stored in overrides.json.
Unknown licenses and missing texts fail generation instead of implying approval.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "THIRD_PARTY_NOTICES.txt"


def notice_files(root: Path, recursive: bool = True) -> list[Path]:
    return sorted(
        p for p in (root.rglob("*") if recursive else root.iterdir())
        if p.is_file() and (
            p.name.lower().startswith(("license", "licence", "copying", "copyright", "notice"))
            or p.name.lower() == "ofl.txt"
        )
    )


def generate() -> str:
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked", "--offline"],
        cwd=ROOT, encoding="utf-8",
    ))
    overrides_path = ROOT / "licenses" / "overrides.json"
    overrides = json.loads(overrides_path.read_text(encoding="utf-8"))
    reviewed = json.loads((ROOT / "licenses/reviewed-expressions.json").read_text(encoding="utf-8"))
    records = []
    texts: dict[str, str] = {}

    def add(name, version, license_id, source, notices):
        if not license_id or not notices:
            raise ValueError(f"Missing license or notice text: {name} {version}")
        refs = []
        for label, content in notices:
            content = "\n".join(line.rstrip() for line in content.splitlines()).strip() + "\n"
            digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
            texts[digest] = content
            refs.append(f"  {label}: {digest}")
        records.append(f"{name} {version}".rstrip() + f"\nLicense: {license_id}\nSource: {source}\n" + "\n".join(refs))

    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["source"] is None:
            continue
        if package["license"] not in reviewed["cargo"]:
            raise ValueError(f"Unreviewed license expression: {package['name']}: {package['license']}")
        root = Path(package["manifest_path"]).parent
        notices = [(p.relative_to(root).as_posix(), p.read_text(encoding="utf-8"))
                   for p in notice_files(root)]
        # Dear ImGui embeds stb's separate copyright and dual-license in headers.
        if package["name"] == "imgui-sys":
            for path in sorted(root.rglob("imstb_*.h")):
                content = path.read_text(encoding="utf-8")
                marker = "This software is available under 2 licenses"
                if marker not in content:
                    raise ValueError(f"Review changed stb license header: {path}")
                notices.append((path.relative_to(root).as_posix() + " (license section)",
                                content[content.index(marker):]))
        key = f"{package['name']}@{package['version']}"
        notices += [(entry["url"], entry["text"]) for entry in overrides.get(key, [])]
        add(package["name"], package["version"], package["license"],
            f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download", notices)

    extension = ROOT / "tools" / "vscode-sol-syntax"
    lock = json.loads((extension / "package-lock.json").read_text(encoding="utf-8"))
    for path, package in sorted(lock["packages"].items()):
        if not path or package.get("dev"):
            continue
        root = extension / path
        name = path.rsplit("node_modules/", 1)[1]
        if package.get("license") not in reviewed["npm"]:
            raise ValueError(f"Unreviewed npm license expression: {name}: {package.get('license')}")
        installed = json.loads((root / "package.json").read_text(encoding="utf-8"))
        if installed["version"] != package["version"]:
            raise ValueError(f"npm install differs from lockfile: {path}")
        add("npm:" + name, package["version"], package.get("license"), package["resolved"],
            [(p.name, p.read_text(encoding="utf-8")) for p in notice_files(root, False)])

    for path, name, license_id in [
        ("assets/UNIFONT-LICENSE.txt", "GNU Unifont bitmap data", "OFL-1.1 (selected alternative)"),
        ("licenses/UNIFONT-18.0.01-NOTICE.txt", "GNU Unifont 18.0.01 release attribution", "OFL-1.1 (selected alternative)"),
        ("licenses/UNIFONT-OFL-1.1.txt", "GNU Unifont official OFL terms", "OFL-1.1"),
        ("assets/UNSCII-LICENSE.txt", "UNSCII-8 CP437 font bank", "CC0-1.0 / public domain"),
        ("licenses/UNSCII-UPSTREAM.md", "UNSCII upstream declaration", "CC0-1.0 / public domain"),
        ("licenses/CC0-1.0.txt", "CC0 standard terms", "CC0-1.0"),
        ("licenses/SOURCE-SANS-PRO.txt", "Source Sans Pro in architecture SVG", "OFL-1.1"),
        ("licenses/UNIFONT-UPSTREAM.txt", "GNU Unifont upstream terms", "OFL-1.1 OR GPL-2.0-or-later WITH Font-exception-2.0"),
        ("examples/softfloat-c/vendor/softfloat/COPYING.txt", "Berkeley SoftFloat Release 3e", "BSD-3-Clause"),
    ]:
        add(name, "", license_id, path, [(path, (ROOT / path).read_text(encoding="utf-8"))])

    return (
        "CPT32 third-party acknowledgements and license notices\n"
        "Generated by: python scripts/generate_license_notices.py\n"
        "Thank you to the authors and maintainers of these projects.\n"
        "Scope: root Cargo resolution, all features, all target platforms, including build/dev\n"
        "dependencies; sol VS Code extension production npm dependencies; bundled fonts/SoftFloat.\n"
        "Extra alternatives/texts are retained; listing a license does not relicense CPT32.\n"
        "MPL-2.0 covered source: option-ext 0.2.0, unmodified, available at\n"
        "https://crates.io/api/v1/crates/option-ext/0.2.0/download\n"
        "GNU Unifont 18.0.01 attribution was verified against the official release archive;\n"
        "its font data and the CPT32 glyph ROM retain OFL-1.1.\n"
        "See doc/license-compliance.md for distribution obligations and audit scope.\n"
        "Package notice references are SHA-256 identifiers of the full texts below.\n\n"
        + "\n\n".join(records)
        + "\n\n==================== FULL LICENSE / NOTICE TEXTS ====================\n\n"
        + "\n".join(f"--- {digest} ---\n{content}" for digest, content in sorted(texts.items()))
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if checked-in notices differ")
    args = parser.parse_args()
    content = generate()
    extension_output = ROOT / "tools/vscode-sol-syntax/THIRD_PARTY_NOTICES.txt"
    outputs = [OUTPUT, extension_output]
    if args.check:
        stale = [str(path.relative_to(ROOT)) for path in outputs
                 if not path.exists() or path.read_text(encoding="utf-8") != content]
        if stale:
            print("Stale notices: " + ", ".join(stale), file=sys.stderr)
            return 1
    else:
        for path in outputs:
            path.write_text(content, encoding="utf-8", newline="\n")
    print(f"License notices {'verified' if args.check else 'generated'} ({len(content)} characters)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
