// DSFS v1 raw disk image. This visualizes metadata and each used file's
// declared bytes; run tools/dsfs/dsfs.py verify for mount-time validation.
// ImHex Pattern Language references:
// https://docs.werwolv.net/pattern-language/core-language/data-types
// https://docs.werwolv.net/pattern-language/core-language/variable-placement
#pragma author "CPT32"
#pragma description "DSFS v1 boot sector, superblock, bitmap, directory and file data"
#pragma magic [44 53 46 53] @ 0x200
#pragma endian big

struct DSFSBootSector {
    u8 boot_code[510];
    u16 signature;
};

struct DSFSSuperblock {
    char magic[4];
    u16 version;
    u16 flags;
    u32 total_blocks;
    u32 bitmap_start;
    u32 bitmap_blocks;
    u32 dir_start;
    u32 dir_blocks;
    u32 data_start;
    u32 created;
    u8 reserved[476];
};

struct DSFSDirectoryEntry {
    u8 status;
    char name[11];
    u8 reserved[4];
    u32 start_block;
    u32 size_bytes;
    u32 created;
    u32 modified;

    // Guard the dynamic file view against empty and out-of-range entries.
    if (status == 1 && size_bytes > 0
        && start_block >= superblock.data_start
        && start_block < superblock.total_blocks
        && size_bytes / 512 + (size_bytes % 512 != 0 ? 1 : 0)
            <= superblock.total_blocks - start_block) {
        u8 file_data[size_bytes] @ start_block * 512;
    }
};

DSFSBootSector boot_sector @ 0x000;
DSFSSuperblock superblock @ 0x200;
u8 bitmap[superblock.bitmap_blocks * 512] @ superblock.bitmap_start * 512;
DSFSDirectoryEntry directory[256] @ superblock.dir_start * 512;
