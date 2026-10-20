#include <cpt32/irqc.h>
#include <cpt32/vpu.h>
#include <stdint.h>

#define Q 1024
#define COLS 12
#define ROWS 18
#define STEP 24
#define WORLD (24576 * Q)
#define U8(a) CPT32_MMIO8(a)
#define U32(a) CPT32_MMIO32(a)

/* All geometry uses integer Q10. Only FIFO values are binary32 bit patterns. */
static int camera_x, camera_z;
static int altitude = 76, yaw, pitch = 12, speed = 2, paused;
static uint8_t keys[256];
static uint32_t vertices[ROWS + 1][COLS + 1][4];
static int heights[ROWS + 3][COLS + 3];

static uint32_t fp(int value) {
  uint32_t sign = 0, m;
  int exponent = 126; /* Q10: 136 - 10; fp(1024) must be 0x3F800000. */
  if (value < 0) {
    sign = 0x80000000u;
    m = (uint32_t)(-value);
  } else {
    m = (uint32_t)value;
  }
  if (!m)
    return 0;
  while (m >= 1024u) {
    m >>= 1;
    ++exponent;
  }
  while (m < 512u) {
    m <<= 1;
    --exponent;
  }
  return sign | ((uint32_t)exponent << 23) | ((m - 512u) << 14);
}

static int sine(int angle) {
  int a = angle % 360, sign = 1, t;
  if (a < 0)
    a += 360;
  if (a >= 180) {
    a -= 180;
    sign = -1;
  }
  t = a * (180 - a);
  return sign * (4 * t * Q / (40500 - t));
}

static int clamp(int v, int low, int high) {
  return v < low ? low : (v > high ? high : v);
}

static int wrap(int v, int period) {
  int r = v % period;
  return r < 0 ? r + period : r;
}

/* Periodic lattice noise: unsigned hashing avoids signed overflow UB.
 * Floor/remainder also work on negative coordinates, across world wrapping. */
static int lattice(int x, int z, int period) {
  uint32_t h = (uint32_t)wrap(x, period) * 73856093u ^
               (uint32_t)wrap(z, period) * 19349663u ^ 0x9E3779B9u;
  h ^= h >> 13;
  h *= 0x85EBCA6Bu;
  h ^= h >> 16;
  return (int)(h & 1023u) - 512;
}

static int noise(int x, int z, int cell) {
  int rx = wrap(x, cell), rz = wrap(z, cell);
  int ix = (x - rx) / cell, iz = (z - rz) / cell;
  int u = rx * Q / cell, v = rz * Q / cell;
  int a, b;
  /* Smoothstep interpolation keeps broad hills continuous at lattice edges. */
  u = u * u / Q * (3 * Q - 2 * u) / Q;
  v = v * v / Q * (3 * Q - 2 * v) / Q;
  a = lattice(ix, iz, 24576 / cell);
  b = lattice(ix + 1, iz, 24576 / cell);
  a += (b - a) * u / Q;
  b = lattice(ix, iz + 1, 24576 / cell);
  b += (lattice(ix + 1, iz + 1, 24576 / cell) - b) * u / Q;
  return a + (b - a) * v / Q;
}

static int height(int x, int z) {
  return noise(x, z, 192) * 48 / Q + noise(x, z, 48) * 12 / Q;
}

static uint32_t color(int h, int slope_x, int slope_z, int distance) {
  int light = clamp(176 + slope_x + slope_z, 96, 240);
  uint32_t base = h > 15 ? 0xAEB8B0u : (h > 3 ? 0x86765Au : 0x39784Au);
  int r = (int)(base >> 16) * light / 192;
  int g = (int)((base >> 8) & 255u) * light / 192;
  int b = (int)(base & 255u) * light / 192;
  int fog = clamp((distance - 192) * Q / 240, 0, Q);
  r += (158 - r) * fog / Q;
  g += (172 - g) * fog / Q;
  b += (174 - b) * fog / Q;
  return ((uint32_t)r << 24) | ((uint32_t)g << 16) | ((uint32_t)b << 8) | 255u;
}

static void triangle(const uint32_t *a, const uint32_t *b, const uint32_t *c) {
  unsigned i;
  vpu_emit(0x1400000Cu);
  for (i = 0; i < 4; ++i)
    vpu_emit(a[i]);
  for (i = 0; i < 4; ++i)
    vpu_emit(b[i]);
  for (i = 0; i < 4; ++i)
    vpu_emit(c[i]);
}

static void sky(void) {
  const uint32_t a[4] = {0xBF800000u, 0x3F800000u, 0x3F7FFF00u, 0x18345EFFu};
  const uint32_t b[4] = {0x3F800000u, 0x3F800000u, 0x3F7FFF00u, 0x18345EFFu};
  const uint32_t c[4] = {0x3F800000u, 0xBF800000u, 0x3F7FFF00u, 0xFFC58AFFu};
  const uint32_t d[4] = {0xBF800000u, 0xBF800000u, 0x3F7FFF00u, 0xFFC58AFFu};
  vpu_set_identity_matrix(0);
  vpu_set_identity_matrix(1);
  vpu_set_identity_matrix(2);
  /* Put the sky below far depth 1, behind all terrain (z <= 408).
   * The current FIFO implementation supports state IDs 0/1 only. */
  triangle(a, b, c);
  triangle(a, c, d);
}

static void generate_terrain(void) {
  int x, z, sy = sine(yaw), cy = sine(yaw + 90);
  for (z = -1; z <= ROWS + 1; ++z) {
    for (x = -1; x <= COLS + 1; ++x) {
      /* Widen distant rows to cover the whole perspective frustum. */
      int lx = (x - COLS / 2) * (STEP + z * 2), lz = (z - 1) * STEP;
      int wx = (camera_x + lx * cy + lz * sy) / Q;
      int wz = (camera_z - lx * sy + lz * cy) / Q;
      int h = height(wx, wz);
      heights[z + 1][x + 1] = h;
      if (x >= 0 && x <= COLS && z >= 0 && z <= ROWS) {
        vertices[z][x][0] = fp(lx * Q);
        vertices[z][x][1] = fp(h * Q);
        vertices[z][x][2] = fp(lz * Q);
      }
    }
  }
  /* Shade shared vertices from cached neighbor heights, avoiding four extra
   * noise samples per vertex. A one-cell halo supplies edge derivatives. */
  for (z = 0; z <= ROWS; ++z) {
    for (x = 0; x <= COLS; ++x) {
      int sx =
          (heights[z + 1][x] - heights[z + 1][x + 2]) * 32 / (STEP + z * 2);
      int sz = (heights[z][x + 1] - heights[z + 2][x + 1]) * 20 / STEP;
      vertices[z][x][3] = color(heights[z + 1][x + 1], sx, sz, (z - 1) * STEP);
    }
  }
}

static void terrain(void) {
  int x, z;
  int sp = sine(pitch), cp = sine(pitch + 90);
  uint32_t view[16] = {fp(Q), 0,       0,      0,
                       0,     fp(cp),  fp(sp), fp(-altitude * cp),
                       0,     fp(-sp), fp(cp), fp(altitude * sp),
                       0,     0,       0,      fp(Q)};
  const uint32_t projection[16] = {
      0x3FC00000u, 0, 0,           0,           0, 0x40000000u, 0,           0,
      0,           0, 0x3F800000u, 0xBF800000u, 0, 0,           0x3F800000u, 0};
  vpu_set_matrix(1, view);
  vpu_set_matrix(2, projection);
  for (z = 0; z < ROWS; ++z) {
    for (x = 0; x < COLS; ++x) {
      triangle(vertices[z][x], vertices[z][x + 1], vertices[z + 1][x + 1]);
      triangle(vertices[z][x], vertices[z + 1][x + 1], vertices[z + 1][x]);
    }
  }
}

static void reset(void) {
  camera_x = camera_z = yaw = paused = 0;
  altitude = 76;
  pitch = 12;
  speed = 2;
}

static void keyboard(void) {
  while (U32(0x80040034u) & 1u) {
    uint32_t event = U32(0x80040030u);
    unsigned usage = (event >> 16) & 255u;
    if ((event >> 24) == 0) {
      int down = (event & 65535u) != 0;
      if (down && !keys[usage]) {
        if (usage == 44)
          paused = !paused; /* Space */
        if (usage == 21)
          reset(); /* R */
      }
      keys[usage] = (uint8_t)down;
    }
    U32(0x80040038u) = 1;
  }
}

static void move(void) {
  int side;
  yaw = wrap(yaw + 2 * ((int)keys[79] - (int)keys[80]), 360);
  pitch = clamp(pitch + (int)keys[22] - (int)keys[26], -15, 40); /* S/W */
  altitude = clamp(altitude + 2 * ((int)keys[82] - (int)keys[81]), 40, 160);
  speed = clamp(speed + (int)keys[8] - (int)keys[20], 0, 8); /* E/Q */
  side = 3 * ((int)keys[7] - (int)keys[4]);                  /* D/A */
  if (!paused) {
    camera_x =
        wrap(camera_x + speed * sine(yaw) + side * sine(yaw + 90), WORLD);
    camera_z =
        wrap(camera_z + speed * sine(yaw + 90) - side * sine(yaw), WORLD);
  }
}

static void text(const char *s, int y) {
  int x = 8;
  while (*s) {
    /* SGC drains commands when VDP renders. Reserve one complete glyph;
     * status bit 3 is a sticky overflow flag, not a live full predicate. */
    while ((U32(0x80010024u) & 65535u) < 4u) {
    }
    U32(0x80010020u) = 0x13000000u;
    U32(0x80010020u) = (uint8_t)*s++;
    U32(0x80010020u) = (uint32_t)x;
    U32(0x80010020u) = (uint32_t)y;
    x += 8;
  }
}

int main(void) {
  irqc_init();
  U8(0x80000000u) = 1;
  U8(0x80000001u) = 0;
  U8(0x80000003u) = 0;
  U32(0x80030014u) = 6; /* VPU on GP6, SGC help above it on GP7. */
  U32(0x80010004u) = 2;
  U32(0x80010018u) = 7;
  U32(0x80010004u) = 1;
  U32(0x80010020u) = 0x0100003Fu;
  text("ARROWS: TURN / ALT  WASD: PITCH / SLIDE", 8);
  text("Q/E: SPEED  SPACE: PAUSE  R: RESET", 24);
  vpu_set_state(0, 1);
  vpu_set_lighting(0);
  irqc_ack(1);
  irqc_enable(1);
  for (;;) {
    keyboard();
    move();
    generate_terrain();
    vpu_clear(3, 0, 0x3F800000u); /* LESS requires far depth, not zero. */
    sky();
    terrain();
    vpu_wait_idle();
    irqc_wait_vsync();
  }
}
