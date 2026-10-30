// SQV4 v1/v2/v3 binary layout. Code bytes are shown packed; this pattern does
// not unpack the 3-bit/4-bit ADPCM codes.
// ImHex Pattern Language references:
// https://docs.werwolv.net/pattern-language/core-language/data-types
// https://docs.werwolv.net/pattern-language/core-language/preprocessor
#pragma author "CPT32"
#pragma description "SqueezVOX 4 (SQV4) v1/v2/v3 header and channel fragments"
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

// v3: 23 samples per fragment, each normalized and decodable on its own.
// v1/v2: 64 samples per fragment, predictor state carried through the file.
fn samples_per_fragment() {
    if (header.version == 3)
        return 23;
    return 64;
};

struct SQV4ChannelFragment {
    u8 volume;

    if (header.version == 3) {
        // Initial step index 0..88, then 23 codes padded with zero bits to a byte.
        u8 step_index;
        if (header.variant == SQV4Variant::H) {
            u8 packed_codes_h[12];
        } else {
            u8 packed_codes_l_or_lplus[9];
        }
    } else {
        // 64 codes: 32 H or 24 L/L+ bytes.
        if (header.variant == SQV4Variant::H) {
            u8 packed_codes_h[32];
        } else {
            u8 packed_codes_l_or_lplus[24];
        }
    }
};

// Fragment order is time-major, then ascending channel number. The final
// fragment is present for every channel whenever samples_per_channel is nonzero.
SQV4ChannelFragment fragments[
    (header.samples_per_channel / samples_per_fragment()
        + (header.samples_per_channel % samples_per_fragment() != 0 ? 1 : 0))
        * header.channel_count
] @ 0x10;
