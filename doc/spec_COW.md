# COW v1 sidecar format

COW v1 stores writable overlays for immutable base images. The base is never changed. The sidecar is append-only during normal operation and contains the latest full-block replacement for each modified logical block. Integers are little-endian. There is no compression or on-disk index.

## Header

The header is exactly 64 bytes:

| Offset | Size | Field |
|---|---:|---|
| `0x00` | 8 | Magic `CPTCOW1\0` |
| `0x08` | 2 | Version, `1` |
| `0x0A` | 2 | Header size, `64` |
| `0x0C` | 2 | Device type: `0` generic, `1` disk, `2` EEPROM, `3` SRAM |
| `0x0E` | 2 | Flags, currently `0` |
| `0x10` | 4 | Block size, 1–4096 bytes |
| `0x14` | 8 | Logical image size, nonzero |
| `0x1C` | 32 | SHA-256 of all logical base bytes |
| `0x3C` | 4 | CRC32C of bytes `0x00..0x3B` |

The base must have exactly the header's logical size. Its SHA-256, device type, block size, version, header size, flags, and header CRC are checked when opening. A mismatch is an error; opening does not alter the existing sidecar. CRC32C is the Castagnoli CRC.

## Records

Each record is a 32-byte header followed by exactly one full block of data:

| Offset | Size | Field |
|---|---:|---|
| `0x00` | 4 | Magic `BLK1` |
| `0x04` | 8 | Block index |
| `0x0C` | 8 | Sequence number, beginning at 1 and increasing by exactly 1 |
| `0x14` | 4 | CRC32C of the following complete block |
| `0x18` | 4 | CRC32C of the record header with this field zeroed |
| `0x1C` | 4 | Reserved, must be zero |

A block index must be less than `ceil(logical_size / block_size)`. The final logical block may extend beyond the base image; it is stored as a full block and bytes beyond logical size are ignored. Repeated writes to one index append records; the last complete record wins. A complete malformed record (including invalid sequence, range, reserved field, or CRC) invalidates the sidecar. A partial final record is a recoverable tail: verification reports it, baking ignores it, and runtime truncates it to the last complete record boundary before a subsequent append.

A writer appends a complete 32-byte record header and one full data block, calls `sync_data`, and updates its in-memory latest-block index/sequence only after the sync succeeds. Failed appends are rolled back to the previous committed boundary; if rollback fails, later writes through that overlay fail. A new sidecar's complete 64-byte header is synced before any record is appended.

A missing sidecar is not created when a package or runtime disk is opened. It is an empty overlay until the first committed write; at that point the parent directory and sidecar are created as needed, and the complete 64-byte header is written and synced before the first record. Read-only verification and baking treat a missing sidecar as an error input.

## Rust API

`cpt32::cow::BaseImage` is an immutable `File(File)` or `Bytes(Arc<[u8]>)` backing image. `CowOverlay::open(base, path, device_type, block_size)` opens or creates an overlay; a missing sidecar begins as an empty overlay. `read_at(&mut self, offset, output)` reads through the overlay and base and rejects any range past the logical image. `write_block(&mut self, block_index, data)` accepts exactly one complete block.

`scan_cow(base, path, device_type, block_size)` is read-only and returns `CowScan`. It rejects a missing sidecar and validates all complete records. `CowScan.records` is the number of complete records, `recoverable_tail` flags an incomplete final record, `valid_end` is the last committed byte offset, and `latest_blocks` maps each modified block index to its data byte offset in the COW file. That index is suitable for a later baker to read and apply the latest block without rescanning records. `logical_size`, `block_size`, and `device_type` describe the validated store. The scanner does not change the COW file.

## Tool usage

The offline utility accepts exactly these commands:

```text
cpt32-tool cow verify BASE COW
cpt32-tool cow bake BASE COW -o OUTPUT
```

`verify` reports complete record count, unique modified block count, and whether a recoverable tail exists. It succeeds when all complete records are valid, even with that tail. `bake` applies only the latest complete record per block to a copy of the raw base, and never modifies either input. It refuses an existing output or an output that aliases an input; successful output is synced and size-checked. A missing COW file is an error for both commands. Compaction is not part of this format/tool version.