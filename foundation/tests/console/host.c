/* Run the actual PCG driver against byte-addressed MMIO, including ring rows. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
static uint8_t vram[0x400000], vdp[0x10000];
static uint32_t input;
static uint8_t *mmio8(uint32_t a) {
    if (a >= 0x10000000u && a < 0x10003000u) return &vram[a - 0x10000000u];
    /* Only the wide font area (0x200000 and up) is granted besides the planes: fail
     * on any 1 byte font (0x20000-0x21FFF) access, including init/clear/scroll. */
    if (a >= 0x10200000u && a < 0x10400000u) return &vram[a - 0x10000000u];
    assert(a >= 0x80000000u && a < 0x80010000u);
    return &vdp[a - 0x80000000u];
}
#define MMIO8(a) (*mmio8(a))
#define MMIO32(a) input
static void store32(uint32_t a, uint32_t v) {
    for (uint32_t i = 0; i < 4; ++i) *mmio8(a + i) = (uint8_t)(v >> (24 - 8 * i));
}
#define CON_WIDE_STORE(a, v) store32((a), (v))
#define CON_WIDE_REG(a) (*mmio8(a))
#include "../../services/console/con_pcg.c"
#include "../../services/console/con_wide.c"
static void write_text(const char *s) { con_pcg_write((const uint8_t *)s, (uint32_t)strlen(s)); }
static void cursor(uint32_t x, uint32_t y) {
    uint32_t a, b;
    con_pcg_get_cursor(&a, &b);
    assert(a == x && b == y);
    assert(vdp[0xF005] == x && vdp[0xF006] == ring_row(y));
}
static uint8_t kind(uint32_t x, uint32_t y) { return vram[0x1000 + ring_row(y) * columns + x] >> 6; }
static uint8_t cell(uint32_t x, uint32_t y, uint32_t plane) {
    return vram[plane + ring_row(y) * columns + x];
}
static void mixed_line_ends(void) {
    /* Try every starting column, both before scrolling and after the ring
     * has wrapped twice. Check every visible cell for orphaned wide halves. */
    for (uint32_t scroll = 0; scroll <= 90; scroll += 90) {
        for (uint32_t start = 0; start < 80; ++start) {
            con_pcg_clear();
            for (uint32_t i = 0; i < scroll; ++i) write_text("\n");
            uint32_t expected_x = start, expected_y = scroll ? 29 : 0;
            con_pcg_set_cursor(start, expected_y);
            const uint32_t widths[] = {1, 2, 2, 1, 2, 1}; /* a日本bあz */
            write_text("a\xE6\x97\xA5\xE6\x9C\xAC" "b\xE3\x81\x82z");
            for (uint32_t i = 0; i < sizeof widths / sizeof widths[0]; ++i) {
                if (expected_x + widths[i] > 80) {
                    expected_x = 0;
                    if (expected_y < 29) ++expected_y;
                }
                expected_x += widths[i];
                if (expected_x == 80) {
                    expected_x = 0;
                    if (expected_y < 29) ++expected_y;
                }
            }
            cursor(expected_x, expected_y);
            for (uint32_t y = 0; y < 30; ++y) {
                for (uint32_t x = 0; x < 80; ++x) {
                    uint32_t k = kind(x, y);
                    assert(k <= 2);
                    if (k == 1) assert(x + 1 < 80 && kind(x + 1, y) == 2);
                    if (k == 2) assert(x && kind(x - 1, y) == 1);
                }
            }
        }
    }
}
int main(void) {
    con_pcg_init();
    write_text("ASCII \xE6\x97\xA5\xC2\x9B");
    cursor(7, 0); assert(cell(6, 0, 0) == '?');
    con_pcg_clear();
    write_text("abc\rX\tZ\bQ\nnext");
    assert(cell(0, 0, 0) == 'X' && cell(8, 0, 0) == 'Q');
    cursor(4, 1);
    /* A sequence may be divided at every byte, including ESC and semicolon. */
    const char *seq = "\033[3;5H";
    for (const char *p = seq; *p; ++p) con_pcg_write((const uint8_t *)p, 1);
    cursor(4, 2);
    write_text("\033[A\033[0D"); cursor(3, 1);
    write_text("\033[9999B\033[9999C"); cursor(79, 29);
    write_text("\033[9999A\033[9999D"); cursor(0, 0);
    write_text("\033[4;H"); cursor(0, 3);
    write_text("\033[5;8f\033[2E"); cursor(0, 6);
    write_text("\033[F\033[20G\033[3d"); cursor(19, 2);
    write_text("\033[s\033[H\033[u"); cursor(19, 2);
    write_text("\0337\033[H\0338"); cursor(19, 2);
    write_text("\033[0;0H"); cursor(0, 0);
    /* Colors are mapped to RGB222, reset preserves legacy white on black. */
    write_text("\033[31;44mA\033[1mB\033[7mC\033[22;27mD\033[0mE");
    assert(cell(0, 0, 0x1000) == 32 && cell(0, 0, 0x2000) == 2);
    assert(cell(1, 0, 0x1000) == 53);
    assert(cell(2, 0, 0x1000) == 2 && cell(2, 0, 0x2000) == 53);
    assert(cell(3, 0, 0x1000) == 32);
    assert(cell(4, 0, 0x1000) == 63 && cell(4, 0, 0x2000) == 0);
    for (uint32_t i = 0; i < 16; ++i) {
        char text[40];
        snprintf(text, sizeof text, "\033[%u;%umX", i < 8 ? 30+i : 90+i-8,
                 i < 8 ? 40+i : 100+i-8);
        write_text(text);
        assert(cell(5+i, 0, 0x1000) == colors[i]);
        assert(cell(5+i, 0, 0x2000) == colors[i]);
    }
    write_text("\033[39;49m"); assert(fg == 15 && bg == 0);
    write_text("\033[7m\033[32;45m\033[m");
    assert(fg == 15 && bg == 0 && !bold && !inverse);
    /* Unsupported colors must consume operands rather than apply SGR 1/7/0. */
    write_text("\033[31;38;2;0;1;7mX\033[48;5;0mY");
    assert(fg == 1 && bg == 0 && !bold && !inverse);
    write_text("\033[?25l"); assert(vdp[0xF007] == 0);
    write_text("\033[?25h"); assert(vdp[0xF007] == 1);
    write_text("\033[0?25l"); assert(vdp[0xF007] == 1);
    /* Unknown, private, intermediate, excess and overflow parameters. */
    con_pcg_clear();
    write_text("a\033[999999999999999999999999H\033[?2J\033[1 q\033(B");
    write_text("\033[0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;31m");
    write_text("\033]0;title\007\033Pignored\033\\b");
    cursor(2, 0); assert(cell(1, 0, 0) == 'b' && fg == 1);
    write_text("\033[123\030c\033[12\033[Hd"); cursor(1, 0);
    assert(cell(0, 0, 0) == 'd');
    write_text("\033["); con_pcg_clear(); write_text("ok"); cursor(2, 0);
    /* Erase includes the cursor, preserves cursor and scroll offset. */
    con_pcg_init();
    for (uint32_t i = 0; i < 33; ++i) write_text("line\n");
    assert(top == 4 && vdp[0xF00E] == 32 && vdp[0xF00F] == 0);
    write_text("\033[2;3H\033[32;41m\033[K"); cursor(2, 1);
    assert(cell(1, 1, 0) == 'i' && cell(2, 1, 0) == ' ');
    assert(cell(79, 1, 0x1000) == 8 && cell(79, 1, 0x2000) == 32);
    write_text("\033[1K"); assert(cell(0, 1, 0) == ' '); cursor(2, 1);
    write_text("\033[3;3H\033[1J");
    assert(cell(0, 0, 0) == ' ' && cell(3, 2, 0) == 'e');
    write_text("\033[J"); assert(cell(3, 2, 0) == ' ');
    write_text("\033[2J"); cursor(2, 2); assert(top == 4);
    for (uint32_t y = 0; y < 30; ++y)
        for (uint32_t x = 0; x < 80; ++x) assert(cell(x, y, 0) == ' ');
    write_text("abc\033[2K"); cursor(5, 2); assert(cell(2, 2, 0) == ' ');
    /* Ring wrap keeps the pixel offset below 240; reused rows are blank. */
    for (uint32_t i = 0; i < 60; ++i) write_text("x\n");
    assert(top < 30 && vdp[0xF00F] == 0);
    assert(cell(0, 29, 0) == ' ');
    /* Switches reset saved cursor/parser and clear using the new stride. */
    con_pcg_init();
    write_text("\033[30;80H\033[s\033[31;44m\033[?25l\033[");
    assert(con_pcg_set_display(FND_CON_DISPLAY_40) == FND_OK);
    assert(con_pcg_columns() == 40 && con_pcg_display() == FND_CON_DISPLAY_40);
    assert(vdp[0xF003] == 0 && fg == 1 && bg == 4 && !vdp[0xF007]);
    write_text("\033[u"); cursor(0, 0);
    write_text("1234567890123456789012345678901234567890"); cursor(0, 1);
    assert(cell(39, 0, 0) == '0');
    write_text("\b"); cursor(39, 0);
    write_text("\t"); cursor(0, 1);
    write_text("\033[9999;9999H"); cursor(39, 29);
    write_text("\033[2K"); cursor(39, 29);
    for (uint32_t i = 0; i < 40; ++i) assert(cell(i, 29, 0) == ' ');
    write_text("\r");
    for (uint32_t i = 0; i < 33; ++i) write_text("x\n");
    assert(top == 3 && vdp[0xF00E] == 24 && vdp[0xF00F] == 0);
    assert(con_pcg_set_display(2) == FND_E_BAD_ARG);
    assert(con_pcg_display() == 0 && top == 3); cursor(0, 29);
    assert(con_pcg_set_display(FND_CON_DISPLAY_8X16) == FND_OK);
    assert(con_pcg_columns() == 80 && vdp[0xF003] == 3);
    assert(con_pcg_set_display(FND_CON_DISPLAY_8X16) == FND_OK);
    for (uint32_t i = 0; i < 46; ++i) write_text("row\n");
    assert(top == 17 && vdp[0xF00E] == 16 && vdp[0xF00F] == 1);
    write_text("\033[2;3H\033[K"); cursor(2, 1);
    assert(cell(0, 1, 0) == 'r' && cell(2, 1, 0) == ' ');
    assert(con_pcg_set_display(FND_CON_DISPLAY_80) == FND_OK);
    assert(con_pcg_columns() == 80 && cell_height == 8 && vdp[0xF003] == 1);
    cursor(0, 0); assert(top == 0);
    assert(con_pcg_set_display(FND_CON_DISPLAY_8X16) == FND_OK);
    con_pcg_clear();
    con_pcg_init();
    /* All 256 configured glyphs, including control-byte glyphs, are literal
     * cells through U+E000..E0FF. Test each display geometry and ring mapping. */
    for (uint32_t mode = 0; mode < 4; ++mode) {
        if (mode == 2) continue;
        assert(con_pcg_set_display(mode) == FND_OK);
        for (uint32_t i = 0; i < 256; ++i) {
            uint8_t text[3] = {0xEE, (uint8_t)(0x80 | (i >> 6)), (uint8_t)(0x80 | (i & 63))};
            con_pcg_write(text, 3);
            assert(cell(i % columns, i / columns, 0) == i);
        }
        cursor(256 % columns, 256 / columns);
        write_text("\033[31m\xEE\x80\x9B");
        assert(cell(256 % columns, 256 / columns, 0) == 0x1B);
        assert(cell(256 % columns, 256 / columns, 0x1000) == 32);
    }
    /* Mode 3: JIS X 0208 characters take two cells (WIDE_KIND 01 then 10). */
    con_pcg_init();
    write_text("\xE6\x97\xA5");
    cursor(1, 0); assert(cell(0, 0, 0) == '?'); /* 8x8 modes have no wide font */
    assert(con_pcg_set_display(FND_CON_DISPLAY_8X16) == FND_OK);
    assert(vdp[0xF010] == 0 && vdp[0xF011] == 0 && vdp[0xF012] == 0x20 && vdp[0xF013] == 0);
    /* The installed glyph of U+65E5 is that of the Unifont ROM (not blank). */
    uint32_t sum = 0;
    for (uint32_t i = 0; i < 32; ++i) sum += vram[0x200000 + 0x65E5 * 32 + i];
    assert(sum != 0);
    write_text("a\xE6\x97\xA5\xE3\x81\x82z");
    cursor(6, 0);
    assert(cell(1, 0, 0) == 0x65 && kind(1, 0) == 1 && cell(2, 0, 0) == 0xE5 && kind(2, 0) == 2);
    assert(cell(3, 0, 0) == 0x30 && kind(3, 0) == 1 && cell(4, 0, 0) == 0x42 && kind(4, 0) == 2);
    assert(kind(0, 0) == 0 && kind(5, 0) == 0 && cell(5, 0, 0) == 'z');
    /* Beyond JIS X 0208, supplementary planes and invalid UTF-8 stay one '?'. */
    write_text("\xE2\x82\xAC\xF0\x9F\x98\x80"); cursor(8, 0);
    assert(cell(6, 0, 0) == '?' && cell(7, 0, 0) == '?' && kind(6, 0) == 0);
    /* BS goes back over a whole character; overwriting a half blanks the other. */
    write_text("\xE6\x97\xA5\b"); cursor(8, 0);
    write_text("\b"); cursor(7, 0);
    write_text("\xE6\x97\xA5\bX"); cursor(8, 0); /* wide at 7-8, BS to 7, X at 7 */
    assert(kind(7, 0) == 0 && cell(7, 0, 0) == 'X' && cell(8, 0, 0) == ' ' && kind(8, 0) == 0);
    write_text("\033[1;3H!"); cursor(3, 0); /* writes over the LSB half of the 2nd char */
    assert(cell(1, 0, 0) == ' ' && kind(1, 0) == 0 && cell(2, 0, 0) == '!' && kind(2, 0) == 0);
    write_text("\033[1;5H\033[K"); /* erase from the LSB half of the third char */
    assert(cell(3, 0, 0) == ' ' && kind(3, 0) == 0 && kind(4, 0) == 0);
    /* Colors apply to both cells; the attribute keeps the CLUT index in its low 6 bits. */
    write_text("\033[2;1H\033[31;44m\xE6\x97\xA5\033[0m");
    assert(vram[0x1000 + ring_row(1) * 80] == (32 | 0x40) && vram[0x1000 + ring_row(1) * 80 + 1] == (32 | 0x80));
    assert(cell(0, 1, 0x2000) == 2 && cell(1, 1, 0x2000) == 2);
    /* A wide character never splits at the line end: the last column stays blank. */
    write_text("\033[3;80H\xE6\x97\xA5");
    assert(cell(79, 2, 0) == ' ' && kind(79, 2) == 0 && kind(0, 3) == 1 && kind(1, 3) == 2);
    cursor(2, 3);
    /* BS across a wrapped row lands on the start of a pair at its end. */
    write_text("\033[5;79H\xE6\x97\xA5\b"); cursor(78, 4);
    /* A wide pair ending a row, scrolling and clearing leave no stale kind bits. */
    write_text("\033[30;79H\xE6\x97\xA5"); /* the line end scrolls */
    assert(kind(78, 28) == 1 && kind(79, 28) == 2 && kind(0, 29) == 0);
    write_text("\n");
    for (uint32_t x = 0; x < 80; ++x) assert(kind(x, 29) == 0);
    mixed_line_ends();
    assert(con_pcg_set_display(FND_CON_DISPLAY_80) == FND_OK);
    for (uint32_t y = 0; y < 30; ++y)
        for (uint32_t x = 0; x < 80; ++x) assert(kind(x, y) == 0);
    puts("console host passed");
    return 0;
}
