/* libcode を WAL として import し、UTF-32BE を中心にした変換・BOM・置換・エラーを確認する。 */
#include <foundation/code.h>
#include <string.h>

static uint8_t out[64], mid[64];

static int same(const uint8_t *want, uint32_t want_n, uint32_t size) {
    return size == want_n && !memcmp(out, want, want_n);
}

int main(void) {
    /* "A", U+00E9, U+3042, U+1F600 */
    static const uint8_t u8[] = {'A', 0xC3, 0xA9, 0xE3, 0x81, 0x82, 0xF0, 0x9F, 0x98, 0x80};
    static const uint8_t u16le[] = {'A', 0, 0xE9, 0, 0x42, 0x30, 0x3D, 0xD8, 0x00, 0xDE};
    static const uint8_t u16be[] = {0, 'A', 0, 0xE9, 0x30, 0x42, 0xD8, 0x3D, 0xDE, 0x00};
    static const uint8_t u32le[] = {'A', 0, 0, 0, 0xE9, 0, 0, 0, 0x42, 0x30, 0, 0, 0x00, 0xF6, 0x01, 0};
    static const uint8_t u32be[] = {0, 0, 0, 'A', 0, 0, 0, 0xE9, 0, 0, 0x30, 0x42, 0, 1, 0xF6, 0x00};
    static const uint8_t u8bom[] = {0xEF, 0xBB, 0xBF, 'A', 0xC3, 0xA9, 0xE3, 0x81, 0x82, 0xF0, 0x9F, 0x98, 0x80};
    static const uint8_t u16lebom[] = {0xFF, 0xFE, 'A', 0};
    uint32_t size = sizeof(out), bom = 9;

    if (cpt_code_utf8_to_utf32be(out, &size, u8, sizeof(u8), 0) || !same(u32be, sizeof(u32be), size)) return 101;
    size = sizeof(out);
    if (cpt_code_utf16le_to_utf32be(out, &size, u16le, sizeof(u16le), 0) || !same(u32be, sizeof(u32be), size)) return 102;
    size = sizeof(out);
    if (cpt_code_utf16be_to_utf32be(out, &size, u16be, sizeof(u16be), 0) || !same(u32be, sizeof(u32be), size)) return 103;
    size = sizeof(out);
    if (cpt_code_utf32le_to_utf32be(out, &size, u32le, sizeof(u32le), 0) || !same(u32be, sizeof(u32be), size)) return 104;
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf8(out, &size, u32be, sizeof(u32be), 0) || !same(u8, sizeof(u8), size)) return 105;
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf16le(out, &size, u32be, sizeof(u32be), 0) || !same(u16le, sizeof(u16le), size)) return 106;
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf16be(out, &size, u32be, sizeof(u32be), 0) || !same(u16be, sizeof(u16be), size)) return 107;
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf32le(out, &size, u32be, sizeof(u32be), 0) || !same(u32le, sizeof(u32le), size)) return 108;
    /* BOM */
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf8bom(out, &size, u32be, sizeof(u32be), 0) || !same(u8bom, sizeof(u8bom), size)) return 109;
    size = sizeof(out);
    if (cpt_code_utf8bom_to_utf32be(out, &size, u8bom, sizeof(u8bom), 0) || !same(u32be, sizeof(u32be), size)) return 110;
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf16le(out, &size, u32be, 4, CODE_ADD_BOM) || !same(u16lebom, sizeof(u16lebom), size)) return 111;
    size = sizeof(out);
    if (cpt_code_utf8_to_utf32be(out, &size, u8bom, sizeof(u8bom), CODE_STRIP_BOM) || !same(u32be, sizeof(u32be), size)) return 112;
    if (cpt_code_detect(u16lebom, sizeof(u16lebom), &bom) != CODE_UTF16LE || bom != 2) return 113;
    /* 経由して UTF-16LE → UTF-8 */
    size = sizeof(mid);
    if (cpt_code_utf16le_to_utf32be(mid, &size, u16le, sizeof(u16le), 0)) return 114;
    uint32_t mid_n = size;
    size = sizeof(out);
    if (cpt_code_utf32be_to_utf8(out, &size, mid, mid_n, 0) || !same(u8, sizeof(u8), size)) return 115;
    /* 不正入力 */
    static const uint8_t bad[] = {'a', 0xC0, 0xAF, 'b'};
    static const uint8_t bad_fffd[] = {0, 0, 0, 'a', 0, 0, 0xFF, 0xFD, 0, 0, 0xFF, 0xFD, 0, 0, 0, 'b'};
    size = sizeof(out);
    if (cpt_code_utf8_to_utf32be(out, &size, bad, sizeof(bad), 0) != CONDE_DATA || size) return 116;
    size = sizeof(out);
    if (cpt_code_utf8_to_utf32be(out, &size, bad, sizeof(bad), CODE_REPLACE) || !same(bad_fffd, sizeof(bad_fffd), size)) return 117;
    size = sizeof(out);
    if (cpt_code_utf8_to_utf32be(out, &size, u8, 2, 0) != CONDE_INPUT_EOF || size) return 118;
    static const uint8_t lone[] = {0x3D, 0xD8, 'a', 0};
    size = sizeof(out);
    if (cpt_code_utf16le_to_utf32be(out, &size, lone, sizeof(lone), 0) != CONDE_DATA || size) return 119;
    /* 容量・引数 */
    size = 5;
    if (cpt_code_utf8_to_utf32be(out, &size, u8, sizeof(u8), 0) != CONDE_OUTPUT_EOF || size) return 120;
    if (cpt_code_bound(sizeof(u8), CODE_UTF8, CODE_UTF32BE) < sizeof(u32be)) return 121;
    size = sizeof(out);
    if (cpt_code_utf8_to_utf32be(out, &size, u8, 1, 8) != CONDE_PARAMETER) return 122;
    return 42;
}
