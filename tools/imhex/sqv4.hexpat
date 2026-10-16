// SQV4 v1/v2 binary layout. Code bytes are shown packed; this pattern does not
// unpack the 3-bit/4-bit ADPCM codes.
// ImHex Pattern Language references:
// https://docs.werwolv.net/pattern-language/core-language/data-types
// https://docs.werwolv.net/pattern-language/core-language/preprocessor
#pragma author "CPT32"
#pragma description "SqueezVOX 4 (SQV4) v1/v2 header and channel fragments"
#pragma magic [53 51 56 34] @ 0x00
#pragma endian little

enum SQV4Variant : u8 {
    H = 0,
    L = 1,
    LPlus = 2
};

struct SQV4Header {
    char magic[4];
    u8 version;
    SQV4Variant variant;
    u8 channel_count;
    u8 reserved;
    u32 sample_rate;
    u32 samples_per_channel;
};

SQV4Header header @ 0x00;

struct SQV4ChannelFragment {
    u8 volume;

    // A channel fragment is 1 volume byte plus 32 H or 24 L/L+ code bytes.
    if (header.variant == SQV4Variant::H) {
        u8 packed_codes_h[32];
    } else {
        u8 packed_codes_l_or_lplus[24];
    }
};

// Fragment order is time-major, then ascending channel number. The final
// fragment is present for every channel whenever samples_per_channel is nonzero.
SQV4ChannelFragment fragments[
    (header.samples_per_channel / 64
        + (header.samples_per_channel % 64 != 0 ? 1 : 0))
        * header.channel_count
] @ 0x10;
