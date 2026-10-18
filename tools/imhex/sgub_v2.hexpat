// SGUB v2 stream: pool directory/payloads, variable-length operations and frames.
// This visualizes declared structures; StreamPlayer performs semantic validation.
// ImHex Pattern Language references:
// https://docs.werwolv.net/pattern-language/core-language/data-types
// https://docs.werwolv.net/pattern-language/core-language/variable-placement
#pragma author "CPT32"
#pragma description "SGUB v2 header, pools, operations and frames"
#pragma magic [53 47 55 42 02] @ 0x00
#pragma endian big

enum SGUBV2PoolType : u8 {
    Wave = 1,
    PCM = 2
};

enum SGUBV2Opcode : u8 {
    WRITE = 0,
    FILL = 1,
    COPY_POOL = 2
};

struct SGUBV2Header {
    char magic[4];
    u8 version;
    u8 voice_count;
    u8 flags;
    u8 reserved;
    u32 frame_count;
    u32 pool_count;
    u32 initial_operation_count;
    u32 pool_directory_offset;
    u32 initial_operations_offset;
    u32 frames_offset;
    u32 total_size;
    u16 header_size;
    u16 reserved_2;
};

struct SGUBV2PoolEntry {
    u16 pool_id;
    SGUBV2PoolType pool_type;
    u8 codec;
    u32 raw_size;
    u32 stored_size;
    u32 payload_offset;
    u8 payload[stored_size] @ payload_offset;
};

struct SGUBV2Operation {
    SGUBV2Opcode opcode;
    u8 target;
    u24 address;

    if (opcode == SGUBV2Opcode::WRITE) {
        u8 value;
    } else if (opcode == SGUBV2Opcode::FILL) {
        u32 length;
        u8 value;
    } else if (opcode == SGUBV2Opcode::COPY_POOL) {
        u16 pool_id;
        u32 source_offset;
        u32 length;
    }
};

struct SGUBV2Frame {
    u16 operation_count;
    SGUBV2Operation operations[operation_count];
};

SGUBV2Header header @ 0x00;
SGUBV2PoolEntry pools[header.pool_count] @ header.pool_directory_offset;
SGUBV2Operation initial_operations[header.initial_operation_count]
    @ header.initial_operations_offset;
SGUBV2Frame frames[header.frame_count] @ header.frames_offset;
