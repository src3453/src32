#ifndef FOUNDATION_CONDE_H
#define FOUNDATION_CONDE_H
/* conde (compression-decompression): 圧縮ライブラリ共通の API 規約。今後の圧縮ライブラリもこれに従う。各 WAL（lzma、rle ほか）は同じ形の関数を <algo> の名前で公開する。
 *
 *   int32_t cpt_<algo>_compress(uint8_t *dst, uint32_t *dst_size, const uint8_t *src, uint32_t src_size,
 *                               void *work, uint32_t work_size, uint32_t level, uint32_t param);
 *   int32_t cpt_<algo>_decompress(uint8_t *dst, uint32_t *dst_size, const uint8_t *src, uint32_t src_size,
 *                                 void *work, uint32_t work_size);
 *   uint32_t cpt_<algo>_compress_bound(uint32_t src_size);
 *
 * - 戻り値は CONDE_*（0 が成功）。失敗時は *dst_size を 0 にし、dst の内容は不定。
 * - *dst_size は容量を入力し、実際に書いた長さを返す。容量不足は CONDE_OUTPUT_EOF。
 * - work は呼出側が用意する 8 byte 整列の作業領域。不要なアルゴリズム（rle）は NULL／0 でもよい。
 * - level は 0..9（アルゴリズムが使わなければ無視）、param はアルゴリズム固有（lzma は辞書サイズ、
 *   rle は 0 のみ）。
 * - compress_bound は圧縮が必ず収まる出力サイズ。表せない場合は 0xFFFFFFFF。
 * - ライブラリ間で関数ポインタは渡せない（WAL の制約）ため、共通化は命名・シグネチャ・コードで行う。 */
#include <stdint.h>

#define CONDE_OK 0
#define CONDE_DATA 1
#define CONDE_MEMORY 2
#define CONDE_UNSUPPORTED 4
#define CONDE_PARAMETER 5
#define CONDE_INPUT_EOF 6
#define CONDE_OUTPUT_EOF 7
#endif
