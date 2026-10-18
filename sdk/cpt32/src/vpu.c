#include <cpt32/vpu.h>

void vpu_emit(uint32_t word) {
  while ((CPT32_MMIO32(VPU_STATUS) & 2u) != 0u) {
  }
  CPT32_MMIO32(VPU_FIFO_DATA) = word;
}

void vpu_wait_idle(void) {
  while ((CPT32_MMIO32(VPU_STATUS) & 1u) != 0u) {
  }
}

uint32_t vpu_command_word(uint32_t opcode, uint32_t payload_words) {
  return ((opcode & 0xffu) << 24) | (payload_words & 0xffffffu);
}

void vpu_clear(uint32_t flags, uint32_t rgba, uint32_t depth) {
  vpu_emit(vpu_command_word(4u, 3u));
  vpu_emit(flags);
  vpu_emit(rgba);
  vpu_emit(depth);
}
void vpu_set_state(uint32_t state_id, uint32_t value) {
  vpu_emit(vpu_command_word(1u, 2u));
  vpu_emit(state_id);
  vpu_emit(value);
}
void vpu_set_lighting(uint32_t enabled) { vpu_set_state(1u, enabled); }
void vpu_set_matrix(uint32_t id, const uint32_t *p) {
  uint32_t i;
  vpu_emit(vpu_command_word(2u, 17u));
  vpu_emit(id);
  for (i = 0; i < 16u; i++)
    vpu_emit(p[i]);
}
void vpu_set_identity_matrix(uint32_t id) {
  uint32_t i;
  vpu_emit(vpu_command_word(2u, 17u));
  vpu_emit(id);
  for (i = 0; i < 16u; i++)
    vpu_emit((i == 0u || i == 5u || i == 10u || i == 15u) ? 0x3f800000u : 0u);
}
void vpu_set_light(uint32_t index, uint32_t enabled, const uint32_t *p) {
  uint32_t i;
  vpu_emit(vpu_command_word(0x15u, 17u));
  vpu_emit(index);
  vpu_emit(enabled);
  for (i = 0; i < 15u; i++)
    vpu_emit(p[i]);
}
void vpu_upload_texture(uint32_t id, uint32_t w, uint32_t h,
                        const uint32_t *p) {
  uint32_t n = (w * h + 1u) / 2u, i;
  vpu_emit(vpu_command_word(0x17u, n + 3u));
  vpu_emit(id);
  vpu_emit(w);
  vpu_emit(h);
  for (i = 0; i < n; i++)
    vpu_emit(p[i]);
}
void vpu_draw_textured_triangle(uint32_t id, uint32_t mode, const uint32_t *p) {
  uint32_t i;
  vpu_emit(vpu_command_word(0x18u, 20u));
  vpu_emit(id);
  vpu_emit(mode);
  for (i = 0; i < 18u; i++)
    vpu_emit(p[i]);
}
void vpu_draw_textured_lit_triangle(uint32_t id, uint32_t mode,
                                    const uint32_t *p) {
  uint32_t i;
  vpu_emit(vpu_command_word(0x19u, 29u));
  vpu_emit(id);
  vpu_emit(mode);
  for (i = 0; i < 27u; i++)
    vpu_emit(p[i]);
}
void vpu_upload_model(uint32_t id, uint32_t count, const uint32_t *p) {
  uint32_t total = count * 28u, i;
  vpu_emit(vpu_command_word(VPU_MODEL_UPLOAD, 2u + total));
  vpu_emit(id);
  vpu_emit(count);
  for (i = 0; i < total; i++)
    vpu_emit(p[i]);
}
void vpu_draw_model(uint32_t id, uint32_t mode) {
  vpu_emit(vpu_command_word(VPU_DRAW_MODEL, 2u));
  vpu_emit(id);
  vpu_emit(mode);
}
void vpu_draw_tl_triangle(const uint32_t *p) {
  uint32_t i;
  vpu_emit(vpu_command_word(0x14u, 12u));
  for (i = 0; i < 12u; i++)
    vpu_emit(p[i]);
}
void vpu_set_target(const uint32_t *p) {
  uint32_t i;
  vpu_emit(vpu_command_word(3u, 6u));
  for (i = 0; i < 6u; i++)
    vpu_emit(p[i]);
}
