#ifndef FOUNDATION_RLE_H
#define FOUNDATION_RLE_H
#include <foundation/conde.h>

/* PackBits 形式の RLE。制御バイト c: 0..127 は続く c+1 byte をそのまま出力、129..255 は続く 1 byte を
 * 257-c 回（2..128）繰返す。128 は不正。3 byte 以上の連続だけを繰返しにする。work は使わない。
 * 共通規約は <foundation/conde.h>。level は 0..9 を受け付けるが無視し、param は 0 のみ。 */
int32_t cpt_rle_compress(uint8_t *dst, uint32_t *dst_size,
                         const uint8_t *src, uint32_t src_size,
                         void *work, uint32_t work_size,
                         uint32_t level, uint32_t param);

/* 入力全体を展開する。末尾の欠落（制御バイトの後の byte 不足）は CONDE_INPUT_EOF。 */
int32_t cpt_rle_decompress(uint8_t *dst, uint32_t *dst_size,
                           const uint8_t *src, uint32_t src_size,
                           void *work, uint32_t work_size);

/* src_size + ceil(src_size / 128)。src_size > 0xFE000000 は 0xFFFFFFFF。 */
uint32_t cpt_rle_compress_bound(uint32_t src_size);
#endif
