# CPT32PKG cartridge package format

CPT32PKG is a complete, read-only ZIP package for an SRC32 guest program. The package may also declare IDC disk images and existing WASM expansion-card packages. The `.cpt32card` format remains a separate WASM-card format; it is not replaced by CPT32PKG.

## Archive layout and limits

Every package has a root `manifest.yaml`. ZIP member paths use forward slashes and must be relative, non-empty, and contain no empty, `.` or `..` component, backslash, NUL, or drive-prefix/colon. ZIP directory entries are accepted as structure, but never count as resources. Duplicate normalized names are rejected (case-sensitive); a directory's trailing slash is removed for collision checking. Non-regular, non-directory entries and compression methods other than Stored and Deflated are rejected. The root manifest must be a regular file.

The manifest is limited to 1 MiB. Every regular ZIP member, including unreferenced assets, is limited to 64 MiB uncompressed; there is no aggregate-size limit. The loader validates archive structure and member metadata, then decompresses only the manifest and files referenced by it. Other safe files remain opaque and are not eagerly decompressed. Resources are read into bounded allocations and their actual decompressed size must exactly match the ZIP's declared size. The loader never extracts or writes into the archive.

## Manifest

The manifest is UTF-8 YAML. Unknown fields are errors at every schema level. Required fields and exact accepted values:

```yaml
format: cpt32pkg
format_version: 1
id: 00000000-0000-4000-8000-000000000001
title: Example CPT32 software
system:
  architecture: SRC32
entrypoint:
  type: program
  path: program.bin
disks:
  - slot: 0
    type: hdd
    path: disks/system.img
    mode: read_write
plugins:
  - slot: 0
    path: plugins/example.cpt32card
```

`format` must be `cpt32pkg`; `format_version` must be integer `1`. `id` is a canonical lowercase UUID in `8-4-4-4-12` hexadecimal form and is the stable identity used for package user data. `title` must not be empty. `system.architecture` is `SRC32`. `entrypoint.type` is `program`; its path names a non-empty raw guest binary loaded at address zero, not a BIOS boot image, and its size must fit the 16 MiB main RAM. Disk and plugin arrays are optional; omitted arrays mean no mounted devices.

Each disk has a unique `slot` from 0 through 3, `type` of `fdd` or `hdd`, a safe archive `path`, and required `mode` of `read_only` or `read_write`. Its image must be non-empty, have a byte size divisible by 512, and contain no more than the 32-bit sector capacity. Read-only media cannot be modified. Writable package media uses external per-cartridge COW sidecars; the ZIP package remains unchanged.

Each plugin has a unique `slot` from 0 through 7 and a safe `path` naming an existing nested `.cpt32card` package. The nested package is passed to the existing WASM-card validation and asset interface.

The entrypoint, each disk, and each plugin must reference distinct paths, and none may reference `manifest.yaml`. References must resolve to regular files. Unreferenced safe files are opaque bundled assets: guest software accesses bundled data through a declared disk image, and plugins use their existing read-only asset API. CPT32PKG adds no guest asset ABI.

## Example package and creation

Example layout:

```text
manifest.yaml
program.bin
disks/system.img
plugins/example.cpt32card
assets/readme.txt
```

From a directory containing the manifest and referenced files, create the archive with:

```sh
zip -X -q example.cpt32pkg manifest.yaml program.bin disks/system.img plugins/example.cpt32card assets/readme.txt
```

The opaque asset is optional and need not be listed in the manifest. The loader does not extract package members.

## Launch

Launch a package using the normal CPT32 CLI:

```sh
cpt32 example.cpt32pkg
```

The `.cpt32pkg` extension is recognized without case sensitivity. Other guest paths retain raw-binary launch behavior.
