#ifndef FOUNDATION_LZMA_H
#define FOUNDATION_LZMA_H
#include <stdint.h>

#define CPT_LZMA_OK 0
#define CPT_LZMA_DATA 1
#define CPT_LZMA_MEMORY 2
#define CPT_LZMA_UNSUPPORTED 4
#define CPT_LZMA_PARAMETER 5
#define CPT_LZMA_INPUT_EOF 6
#define CPT_LZMA_OUTPUT_EOF 7

/* .lzma (LZMA-Alone) のヘッダーを含む圧縮。*dst_size は容量を入力し実長を返す。 */
int32_t cpt_lzma_compress(uint8_t *dst, uint32_t *dst_size,
                          const uint8_t *src, uint32_t src_size,
                          void *work, uint32_t work_size,
                          uint32_t level, uint32_t dictionary_size);

/* 単一 .lzma ストリームを展開。末尾の余剰データを拒否する。作業領域は呼出ごとに再利用可能。 */
int32_t cpt_lzma_decompress(uint8_t *dst, uint32_t *dst_size,
                            const uint8_t *src, uint32_t src_size,
                            void *work, uint32_t work_size);
#endif
