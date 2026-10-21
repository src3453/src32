// WAX v2 sparse container. Version 1 uses a different layout and is not parsed.
// ImHex Pattern Language references:
// https://docs.werwolv.net/pattern-language/core-language/data-types
// https://docs.werwolv.net/pattern-language/core-language/variable-placement
// https://docs.werwolv.net/pattern-language/core-language/preprocessor
#pragma author "CPT32"
#pragma description "WAX v2 header, block table, payload blocks and debug data"
#pragma magic [57 41 58 00 00 02] @ 0x00
#pragma endian big

enum WAXType : u16 {
    Executable = 1,
    Library = 2
};

enum WAXArchitecture : u16 {
    SRC32 = 1
};

struct WAXHeader {
    char magic[4];
    u16 version;
    WAXType type;
    WAXArchitecture architecture;
    u16 flags;
    u32 header_size;
    u32 base_addr;
    u32 entry_offset;
    u32 block_table_offset;
    u32 block_count;
    u32 block_entry_size;
    u32 debug_offset;
    u32 debug_size;
    u32 file_size;
    // flags bit1 = PROTECTED: user-mode stack fields, 16-byte block entries.
    u32 user_stack_top;
    u32 user_stack_size;
    u8 reserved[8];
    // Optional NOLOAD data is shown at its file offset without enlarging the header.
    u8 debug_payload[debug_size] @ debug_offset;
};

WAXHeader header @ 0x00;

bitfield WAXPermission {
    read : 1;
    write : 1;
    execute : 1;
    padding : 29;
};

struct WAXBlockEntry {
    u32 load_address;
    u32 file_offset;
    u32 size;
    if (header.block_entry_size == 16) {
        WAXPermission perm;
    }

    // This is placed at file_offset and does not extend the table entry.
    u8 payload[size] @ file_offset;
};

// v2 block entries are 12 bytes (16 with PROTECTED permissions). Each payload is displayed at its file offset,
// while load_address records where the emulator maps it in guest memory.
WAXBlockEntry blocks[header.block_count] @ header.block_table_offset;

