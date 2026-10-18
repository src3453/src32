// SGUB v1 stream: fixed 20-byte header, five-byte writes and counted frames.
// This visualizes declared records; it does not validate target/address ranges.
// ImHex Pattern Language references:
// https://docs.werwolv.net/pattern-language/core-language/data-types
// https://docs.werwolv.net/pattern-language/core-language/variable-placement
#pragma author "CPT32"
#pragma description "SGUB v1 header, initialization records and frames"
#pragma magic [53 47 55 42 01] @ 0x00
#pragma endian big

struct SGUBV1Header {
    char magic[4];
    u8 version;
    u8 voice_count;
    u8 flags;
    u8 reserved;
    u32 frame_count;
    u32 initial_record_count;
    u32 total_size;
};

struct SGUBV1Record {
    u8 target;
    u24 address;
    u8 value;
};

struct SGUBV1Frame {
    u16 record_count;
    SGUBV1Record records[record_count];
};

SGUBV1Header header @ 0x00;
SGUBV1Record initial_records[header.initial_record_count] @ 0x14;
SGUBV1Frame frames[header.frame_count]
    @ 0x14 + header.initial_record_count * 5;
