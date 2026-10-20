#ifndef CPT32_VPU_H
#define CPT32_VPU_H
#include <cpt32/mmio.h>
#include <stdint.h>

#define VPU_FIFO_DATA 0x8003002Cu
#define VPU_STATUS 0x80030004u
#define VPU_MATRIX_MODEL 0u
#define VPU_MATRIX_VIEW 1u
#define VPU_MATRIX_PROJECTION 2u
#define VPU_SHADE_NONE 0u
#define VPU_SHADE_FLAT 1u
#define VPU_SHADE_GOURAUD 2u
#define VPU_MODEL_UPLOAD 0x1Au
#define VPU_DRAW_MODEL 0x1Bu

/* FIFOに空きができるまで待ち、32-bitワードを1つ送信します。 */
void vpu_emit(uint32_t word);
/* VPUの処理中ビットがクリアされるまで待ちます。 */
void vpu_wait_idle(void);
/* opcodeの下位8 bitとpayload長の下位24 bitからコマンドワードを作ります。 */
uint32_t vpu_command_word(uint32_t opcode, uint32_t payload_words);
/* flagsで指定したカラー・深度バッファのクリアコマンドを送信します。 */
void vpu_clear(uint32_t flags, uint32_t rgba, uint32_t depth);
/* 指定したVPU状態IDに値を設定します。 */
void vpu_set_state(uint32_t state_id, uint32_t value);
/* 固定機能ライティングの有効・無効を設定します。 */
void vpu_set_lighting(uint32_t enabled);
/* 行優先のbinary32ビット列16ワードで指定行列を設定します。 */
void vpu_set_matrix(uint32_t matrix_id, const uint32_t *matrix_words);
/* 指定行列を4×4の単位行列に設定します。 */
void vpu_set_identity_matrix(uint32_t matrix_id);
/* 指定ライトの有効状態と、色・位置のbinary32ビット列15ワードを設定します。 */
void vpu_set_light(uint32_t index, uint32_t enabled,
                   const uint32_t *light_words);
/* 2画素を1ワードに詰めたテクスチャを登録します（配列長は(width*height+1)/2）。 */
void vpu_upload_texture(uint32_t texture_id, uint32_t width, uint32_t height,
                        const uint32_t *packed_pixels);
/* 位置・UV・色の計18ワードから、指定テクスチャの三角形を描画します。 */
void vpu_draw_textured_triangle(uint32_t texture_id, uint32_t shading_mode,
                                const uint32_t *vertex_words);
/* 位置・法線・UV・色の計27ワードから、照明対応のテクスチャ三角形を描画します。 */
void vpu_draw_textured_lit_triangle(uint32_t texture_id, uint32_t shading_mode,
                                    const uint32_t *vertex_words);
/* 三角形ごとに28ワードのレコードを持つモデルを登録します。 */
void vpu_upload_model(uint32_t model_id, uint32_t triangle_count,
                      const uint32_t *record_words);
/* 登録済みモデルを指定シェーディング方式で描画します。 */
void vpu_draw_model(uint32_t model_id, uint32_t shading_mode);
/* 位置・色の計12ワードから、行列変換付き三角形を描画します。 */
void vpu_draw_tl_triangle(const uint32_t *vertex_words);
/* カラー基底・Z基底・幅・高さ・各strideの6ワードで描画先を設定します。 */
void vpu_set_target(const uint32_t *target_words);
#endif
