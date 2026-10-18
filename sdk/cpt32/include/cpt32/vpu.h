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

void vpu_emit(uint32_t word);
void vpu_wait_idle(void);
uint32_t vpu_command_word(uint32_t opcode, uint32_t payload_words);
void vpu_clear(uint32_t flags, uint32_t rgba, uint32_t depth);
void vpu_set_state(uint32_t state_id, uint32_t value);
void vpu_set_lighting(uint32_t enabled);
void vpu_set_matrix(uint32_t matrix_id, const uint32_t *matrix_words);
void vpu_set_identity_matrix(uint32_t matrix_id);
void vpu_set_light(uint32_t index, uint32_t enabled,
                   const uint32_t *light_words);
void vpu_upload_texture(uint32_t texture_id, uint32_t width, uint32_t height,
                        const uint32_t *packed_pixels);
void vpu_draw_textured_triangle(uint32_t texture_id, uint32_t shading_mode,
                                const uint32_t *vertex_words);
void vpu_draw_textured_lit_triangle(uint32_t texture_id, uint32_t shading_mode,
                                    const uint32_t *vertex_words);
void vpu_upload_model(uint32_t model_id, uint32_t triangle_count,
                      const uint32_t *record_words);
void vpu_draw_model(uint32_t model_id, uint32_t shading_mode);
void vpu_draw_tl_triangle(const uint32_t *vertex_words);
void vpu_set_target(const uint32_t *target_words);
#endif
