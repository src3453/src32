#include <cpt32/irqc.h>
#include <cpt32/pec_rng.h>
#include <cpt32/vpu.h>
#include <stdint.h>

#define FP_ONE 1024
#define VDP_ENABLE 0x80000000u
#define VDP_MODE 0x80000001u
#define VDP_BORDER 0x80000003u
#define VDP_BITMAP_COLOR_MODE 0x80000004u
#define VPU_OUTPUT_GP 0x80030014u
#define SGC_CONTROL 0x80010004u
#define SGC_OUTPUT_GP 0x80010018u
#define SGC_FIFO 0x80010020u
#define PEC_EVENT_DATA 0x80040030u
#define PEC_EVENT_STATUS 0x80040034u
#define PEC_EVENT_POP 0x80040038u
#define GP0_BITMAP 0x10000000u
#define GP0_CLUT 0x10012C00u
#define GP0_BITMAP_PIXELS 76800u
#define U8(address) (*(volatile uint8_t *)(uintptr_t)(address))
#define U32(address) (*(volatile uint32_t *)(uintptr_t)(address))

static int camera_distance = 240;
static int rotation_speed = 2;
static int light_red = 12;
static int light_green = 12;
static int light_blue = 12;
static int overlay_dirty = 1;
static int light_dirty;

void emit(uint32_t word) { vpu_emit(word); }

/* Convert the cube's signed Q10 values to IEEE-754 binary32 bits. */
uint32_t q10_to_f32(int value) {
  uint32_t sign = 0;
  uint32_t magnitude;
  int scale = -10;
  if (value < 0) {
    sign = 0x80000000u;
    magnitude = (uint32_t)(-value);
  } else {
    magnitude = (uint32_t)value;
  }
  if (magnitude == 0)
    return 0;
  while (magnitude >= 1024u) {
    magnitude >>= 1;
    ++scale;
  }
  while (magnitude < 512u) {
    magnitude <<= 1;
    --scale;
  }
  return ((uint32_t)(scale + 136) << 23) | ((magnitude - 512u) << 14) | sign;
}

int sin_deg(int angle) {
  int a = angle % 360;
  int sign = 1;
  int t;
  if (a < 0)
    a += 360;
  if (a >= 180) {
    a -= 180;
    sign = -1;
  }
  t = a * (180 - a);
  return sign * (t * 4 * FP_ONE) / (40500 - t);
}

uint32_t noise_texel(void) {
  uint32_t n = pec_rng_rand() & 15u;
  return (n << 12) | (n << 8) | (n << 4) | 15u;
}

void setup_gp_noise(void) {
  uint32_t i;
  for (i = 64; i < 256; ++i) {
    uint32_t gray = ((i - 64u) * 255u) / 191u;
    uint32_t clut = GP0_CLUT + i * 3u;
    U8(clut) = (uint8_t)gray;
    U8(clut + 1u) = (uint8_t)gray;
    U8(clut + 2u) = (uint8_t)gray;
  }
  for (i = 0; i < GP0_BITMAP_PIXELS; ++i) {
    U8(GP0_BITMAP + i) = (uint8_t)(64u + ((pec_rng_rand() & 255u) % 192u));
  }
}

void setup_noise_texture(void) {
  uint32_t i;
  emit(0x17000203u); /* TEXTURE_UPLOAD, three descriptor words + 512 data words
                      */
  emit(1);           /* texture ID */
  emit(32);
  emit(32);
  for (i = 0; i < 512; ++i) {
    uint32_t pair = (noise_texel() << 16) | noise_texel();
    emit(pair);
  }
}

void set_view_matrix(int distance) {
  emit(0x02000011u);
  emit(1); /* VIEW */
  emit(0x3F800000u);
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0x3F800000u);
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0x3F800000u);
  emit(q10_to_f32(distance * FP_ONE));
  emit(0);
  emit(0);
  emit(0);
  emit(0x3F800000u);
}

void set_projection_matrix(void) {
  emit(0x02000011u);
  emit(2); /* PROJECTION */
  emit(0x3FC00000u);
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0x40000000u);
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0x3F800000u);
  emit(0xBF800000u);
  emit(0);
  emit(0);
  emit(0x3F800000u);
  emit(0);
}

void set_model_matrix(int yaw, int pitch) {
  int sy = sin_deg(yaw);
  int cy = sin_deg(yaw + 90);
  int sx = sin_deg(pitch);
  int cx = sin_deg(pitch + 90);
  int m10 = (sx * sy) / FP_ONE;
  int m12 = -(sx * cy) / FP_ONE;
  int m20 = -(cx * sy) / FP_ONE;
  int m22 = (cx * cy) / FP_ONE;

  emit(0x02000011u);
  emit(0); /* MODEL */
  emit(q10_to_f32(cy));
  emit(0);
  emit(q10_to_f32(sy));
  emit(0);
  emit(q10_to_f32(m10));
  emit(q10_to_f32(cx));
  emit(q10_to_f32(m12));
  emit(0);
  emit(q10_to_f32(m20));
  emit(q10_to_f32(sx));
  emit(q10_to_f32(m22));
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0x3F800000u);
}

void clear_frame(void) {
  emit(0x04000003u);
  emit(3);
  emit(0);
  emit(0x3F800000u); /* Reset depth to the far plane. */
}

void emit_cube_vertex(unsigned index, uint32_t u, uint32_t v, uint32_t rgba) {
  emit((index & 1u) == 0 ? 0x42400000u : 0xC2400000u);
  emit((index & 2u) == 0 ? 0x42400000u : 0xC2400000u);
  emit((index & 4u) == 0 ? 0x42400000u : 0xC2400000u);
  emit(u);
  emit(v);
  emit(rgba);
}

void triangle(unsigned a, unsigned b, unsigned c, uint32_t u0, uint32_t v0,
              uint32_t u1, uint32_t v1, uint32_t u2, uint32_t v2,
              uint32_t rgba) {
  emit(0x18000014u); /* texture 1, Flat, three position/UV/color vertices */
  emit(1);
  emit(1);
  emit_cube_vertex(a, u0, v0, rgba);
  emit_cube_vertex(b, u1, v1, rgba);
  emit_cube_vertex(c, u2, v2, rgba);
}

void face(unsigned a, unsigned b, unsigned c, unsigned d, uint32_t rgba) {
  triangle(a, b, c, 0, 0, 0x3F800000u, 0, 0x3F800000u, 0x3F800000u, rgba);
  triangle(a, c, d, 0, 0, 0x3F800000u, 0x3F800000u, 0, 0x3F800000u, rgba);
}

void draw_cube(void) {
  face(1, 5, 7, 3, 0xE05030FFu); /* -X */
  face(0, 2, 6, 4, 0x3060E0FFu); /* +X */
  face(2, 3, 7, 6, 0xE0C030FFu); /* -Y */
  face(0, 4, 5, 1, 0x30C070FFu); /* +Y */
  face(4, 6, 7, 5, 0xC040C0FFu); /* -Z */
  face(0, 1, 3, 2, 0x20C0D0FFu); /* +Z */
}

void setup_light(int red, int green, int blue) {
  emit(0x15000011u); /* SET_LIGHT, light 0 enabled */
  emit(0);
  emit(1);
  emit(0x3E4CCCCDu);
  emit(0x3E4CCCCDu);
  emit(0x3E4CCCCDu);
  emit(q10_to_f32(red * 68));
  emit(q10_to_f32(green * 68));
  emit(q10_to_f32(blue * 68));
  emit(0x3E4CCCCDu);
  emit(0x3E4CCCCDu);
  emit(0x3E4CCCCDu);
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0);
  emit(0xC2C80000u); /* View-space point light. */
  emit(0x01000002u); /* Enable fixed-function lighting. */
  emit(1);
  emit(1);
}

int clamp(int value, int delta, int minimum, int maximum) {
  int next = value + delta;
  if (next < minimum)
    next = minimum;
  if (next > maximum)
    next = maximum;
  return next;
}

void draw_glyph(uint8_t code, int x, int y) {
  U32(SGC_FIFO) = 0x13000000u;
  U32(SGC_FIFO) = code;
  U32(SGC_FIFO) = (uint32_t)x;
  U32(SGC_FIFO) = (uint32_t)y;
}

void draw_text(const char *text, int x, int y) {
  unsigned i = 0;
  while (text[i] != '\0') {
    draw_glyph((uint8_t)text[i], x + (int)(i * 8u), y);
    ++i;
  }
}

void draw_number2(unsigned value, int x, int y) {
  draw_glyph((uint8_t)('0' + value / 10u), x, y);
  draw_glyph((uint8_t)('0' + value % 10u), x + 8, y);
}

void draw_number3(unsigned value, int x, int y) {
  draw_glyph((uint8_t)('0' + value / 100u), x, y);
  draw_glyph((uint8_t)('0' + (value / 10u) % 10u), x + 8, y);
  draw_glyph((uint8_t)('0' + value % 10u), x + 16, y);
}

void draw_overlay(void) {
  U32(SGC_CONTROL) = 2; /* Reset clears the old text plane. */
  U32(SGC_OUTPUT_GP) = 7;
  U32(SGC_CONTROL) = 1;
  U32(SGC_FIFO) = 0x0100003Fu;
  draw_text("W/S DIST A/D SPD", 8, 8);
  draw_text("J/K R U/I G N/M B", 8, 24);
  draw_text("D:", 8, 48);
  draw_number3((unsigned)camera_distance, 24, 48);
  draw_text("S:", 64, 48);
  draw_number2((unsigned)rotation_speed, 80, 48);
  draw_text("R:", 112, 48);
  draw_number2((unsigned)light_red, 128, 48);
  draw_text("G:", 160, 48);
  draw_number2((unsigned)light_green, 176, 48);
  draw_text("B:", 208, 48);
  draw_number2((unsigned)light_blue, 224, 48);
}

void adjust_value(int *value, int delta, int low, int high, int changes_light) {
  int next = clamp(*value, delta, low, high);
  if (next != *value) {
    *value = next;
    overlay_dirty = 1;
    if (changes_light)
      light_dirty = 1;
  }
}

void handle_key(unsigned key) {
  switch (key) {
  case 26:
    adjust_value(&camera_distance, -8, 128, 512, 0);
    break; /* W */
  case 22:
    adjust_value(&camera_distance, 8, 128, 512, 0);
    break; /* S */
  case 4:
    adjust_value(&rotation_speed, -1, 0, 12, 0);
    break; /* A */
  case 7:
    adjust_value(&rotation_speed, 1, 0, 12, 0);
    break; /* D */
  case 13:
    adjust_value(&light_red, -1, 0, 15, 1);
    break; /* J */
  case 14:
    adjust_value(&light_red, 1, 0, 15, 1);
    break; /* K */
  case 24:
    adjust_value(&light_green, -1, 0, 15, 1);
    break; /* U */
  case 12:
    adjust_value(&light_green, 1, 0, 15, 1);
    break; /* I */
  case 17:
    adjust_value(&light_blue, -1, 0, 15, 1);
    break; /* N */
  case 16:
    adjust_value(&light_blue, 1, 0, 15, 1);
    break; /* M */
  default:
    break;
  }
}

void poll_keyboard(void) {
  while ((U32(PEC_EVENT_STATUS) & 1u) != 0) {
    uint32_t event = U32(PEC_EVENT_DATA);
    unsigned usage = (event >> 16) & 255u;
    if (((event >> 28) & 15u) == 0 && ((event >> 24) & 15u) == 0 &&
        (event & 1u) != 0) {
      handle_key(usage);
    }
    U32(PEC_EVENT_POP) = 1;
  }
}

int main(void) {
  int yaw = 25;
  int pitch = 20;
  irqc_init();
  U8(VDP_ENABLE) = 1;
  U8(VDP_MODE) = 0;
  U8(VDP_BORDER) = 0;
  U32(VPU_OUTPUT_GP) = 6; /* Keep GP7 above the cube for SGC text. */
  U32(SGC_CONTROL) = 2;
  U32(SGC_OUTPUT_GP) = 7;
  U8(VDP_BITMAP_COLOR_MODE) = 0;
  pec_rng_initPRNG();
  setup_gp_noise();
  setup_noise_texture();
  set_view_matrix(camera_distance);
  set_projection_matrix();
  setup_light(light_red, light_green, light_blue);
  draw_overlay();
  irqc_ack(1);
  irqc_enable(1);

  for (;;) {
    poll_keyboard();
    if (light_dirty) {
      setup_light(light_red, light_green, light_blue);
      light_dirty = 0;
    }
    if (overlay_dirty) {
      draw_overlay();
      overlay_dirty = 0;
    }
    clear_frame();
    set_view_matrix(camera_distance);
    set_model_matrix(yaw, pitch);
    draw_cube();
    yaw = (yaw + rotation_speed) % 360;
    pitch = (pitch + rotation_speed) % 360;
    irqc_wait_vsync();
  }
}
