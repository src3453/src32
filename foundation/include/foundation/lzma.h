#ifndef FOUNDATION_LZMA_H
#define FOUNDATION_LZMA_H
/* conde（<foundation/conde.h>）に準拠する。 */
#include <foundation/conde.h>

#define CPT_LZMA_OK CONDE_OK
#define CPT_LZMA_DATA CONDE_DATA
#define CPT_LZMA_MEMORY CONDE_MEMORY
#define CPT_LZMA_UNSUPPORTED CONDE_UNSUPPORTED
#define CPT_LZMA_PARAMETER CONDE_PARAMETER
#define CPT_LZMA_INPUT_EOF CONDE_INPUT_EOF
#define CPT_LZMA_OUTPUT_EOF CONDE_OUTPUT_EOF

/* .lzma (LZMA-Alone) のヘッダーを含む圧縮。*dst_size は容量を入力し実長を返す。 */
int32_t cpt_lzma_compress(uint8_t *dst, uint32_t *dst_size,
                          const uint8_t *src, uint32_t src_size,
                          void *work, uint32_t work_size,
                          uint32_t level, uint32_t dictionary_size);

/* 単一 .lzma ストリームを展開。末尾の余剰データを拒否する。作業領域は呼出ごとに再利用可能。 */
int32_t cpt_lzma_decompress(uint8_t *dst, uint32_t *dst_size,
                            const uint8_t *src, uint32_t src_size,
                            void *work, uint32_t work_size);

/* 13 byte のヘッダーを含め、圧縮結果が必ず収まるサイズ（SDK の見積り式）。表せなければ 0xFFFFFFFF。 */
uint32_t cpt_lzma_compress_bound(uint32_t src_size);
#endif
