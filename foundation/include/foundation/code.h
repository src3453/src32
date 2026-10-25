#ifndef FOUNDATION_CODE_H
#define FOUNDATION_CODE_H
#include <foundation/conde.h>

/* 文字コード変換。UTF-32BE（コードポイントを 4 byte big endian で並べたもの）を基準の形式とし、
 * 他の符号化との間を cpt_code_<enc>_to_utf32be と cpt_code_utf32be_to_<enc> で変換します。
 * <enc> は utf8、utf8bom、utf16le、utf16be、utf32le。たとえば UTF-16LE から UTF-8 へは
 * utf16le_to_utf32be の出力を utf32be_to_utf8 に渡します（UTF-32BE の内容を直接扱う場合は変換が要りません）。
 * 戻り値は CONDE_*（0 が成功）、*dst_size は容量を入力して実長を返し、失敗時は 0 にします（conde と同じ）。
 * 作業領域は使いません。dst と src は重ねないでください。
 *
 *   int32_t cpt_code_<enc>_to_utf32be(uint8_t *dst, uint32_t *dst_size,
 *                                     const uint8_t *src, uint32_t src_size, uint32_t flags);
 *   int32_t cpt_code_utf32be_to_<enc>(同じ引数);
 *
 * utf8bom は UTF-8 の BOM 付き。変換先では先頭に EF BB BF を必ず書き、変換元では先頭にあれば読み飛ばす。
 * 出力に BOM を付けたい他の符号化には CODE_ADD_BOM を使う。
 *
 * エラー:
 *   CONDE_DATA       不正な列。UTF-8 の過長形式・サロゲート・U+10FFFF 超、UTF-16 の対にならないサロゲート、
 *                    UTF-32 のサロゲート・U+10FFFF 超
 *   CONDE_INPUT_EOF  末尾で符号化単位が途中で切れている（UTF-8 の列の途中、UTF-16 の上位サロゲートだけ、
 *                    端数 byte など）
 *   CONDE_OUTPUT_EOF 出力が *dst_size に収まらない
 *   CONDE_PARAMETER  flags が不正、ポインタが NULL（src_size == 0 なら src は NULL 可）
 * CODE_REPLACE では、不正な部分（Unicode 3.9 の maximal subpart）ごとに U+FFFD を 1 つ書いて続ける。
 * 先頭の U+FEFF は、CODE_STRIP_BOM などで落とさない限り文字として写す（UTF-16LE の FF FE は ZWNBSP になる）。
 * CODE_ADD_BOM と入力側の U+FEFF が重なると BOM が 2 つになる。BOM を整えるなら CODE_STRIP_BOM と組み合わせる。 */
#define CODE_REPLACE 1u    /* 不正な入力を U+FFFD に置き換える（なければ CONDE_DATA／CONDE_INPUT_EOF） */
#define CODE_STRIP_BOM 2u  /* 入力の先頭の U+FEFF を捨てる（utf8bom の変換元は指定なしでも読み飛ばす） */
#define CODE_ADD_BOM 4u    /* 出力の先頭に BOM を付ける（utf8bom の変換先は指定なしでも付く） */

#define CPT_CODE_DECLARE(enc)                                                                           \
    int32_t cpt_code_##enc##_to_utf32be(uint8_t *dst, uint32_t *dst_size, const uint8_t *src,            \
                                        uint32_t src_size, uint32_t flags);                              \
    int32_t cpt_code_utf32be_to_##enc(uint8_t *dst, uint32_t *dst_size, const uint8_t *src,              \
                                      uint32_t src_size, uint32_t flags);
CPT_CODE_DECLARE(utf8)
CPT_CODE_DECLARE(utf8bom)
CPT_CODE_DECLARE(utf16le)
CPT_CODE_DECLARE(utf16be)
CPT_CODE_DECLARE(utf32le)
#undef CPT_CODE_DECLARE

/* 符号化の番号（cpt_code_bound と cpt_code_detect が使う）。 */
enum {
    CODE_UTF8 = 0,
    CODE_UTF8_BOM = 1,
    CODE_UTF16LE = 2,
    CODE_UTF16BE = 3,
    CODE_UTF32LE = 4,
    CODE_UTF32BE = 5
};

/* from から to へ変換した出力が（flags によらず）必ず収まる byte 数。BOM 分を含む。片方が UTF-32BE の
 * 変換用だが、どの組み合わせでも使える。表せない場合や符号化が不正な場合は 0xFFFFFFFF。 */
uint32_t cpt_code_bound(uint32_t src_size, uint32_t from, uint32_t to);

/* 先頭の BOM から符号化を判別する。BOM がなければ CODE_UTF8 を返す。bom_size（NULL 可）には BOM の長さ
 * （なければ 0）を返す。判別できるのは CODE_UTF8_BOM、CODE_UTF16LE／BE、CODE_UTF32LE／BE。
 * FF FE 00 00 は UTF-16LE の BOM と U+0000 でも解釈できるが UTF-32LE とする。src が NULL（src_size > 0）なら負の値。
 * 判別した符号化の関数に src + *bom_size を渡す（utf8bom は BOM ごと渡してよい）。 */
int32_t cpt_code_detect(const uint8_t *src, uint32_t src_size, uint32_t *bom_size);
#endif
