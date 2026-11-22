/* Flappy bird style bare-metal game for CPT32.
 *
 * Drawing: GP0 256-color bitmap (320x240, 1 byte/pixel at VRAM 0x10000000).
 * The VDP samples GP0 at the start of each host frame, before IRQ0, so the
 * whole frame is redrawn right after irqc_wait_vsync() and is shown complete
 * at the next frame -- as long as drawing fits in one frame (800,000 cycles).
 *
 * Every DMAC request costs about a thousand CPU cycles of setup and polling,
 * so the frame is built from a few large BitBlts (channel 2) of pieces that
 * are drawn once into Main RAM at WORK: the background (four copies shifted by
 * 0..3 bytes, so the parallax scroll can use 32-bit beats), one pipe row
 * repeated with source stride 0, the pipe cap, a ground strip and cached text
 * labels. The bird and labels are color-keyed blits (index 0 = transparent).
 *
 * Controls: Space / Up / W / Enter = flap (and start / retry).
 */
#include <cpt32/dmac.h>
#include <cpt32/irqc.h>
#include <cpt32/mmio.h>
#include <stdint.h>

#define U8(a) CPT32_MMIO8(a)
#define U32(a) CPT32_MMIO32(a)

#define VRAM 0x10000000u
#define CLUT 0x10012C00u
#define SCREEN_W 320
#define SCREEN_H 240
#define CH 2u

/* Colors: SGC owns CLUT 0..63, so the game uses 64 and up. Index 0 is the
 * transparent key of the work sprites and is never drawn. */
enum {
  C_SKY = 64,
  C_SKY2,
  C_CLOUD,
  C_CITY,
  C_CITY_WIN,
  C_BUSH,
  C_BUSH_DARK,
  C_PIPE,
  C_PIPE_LIGHT,
  C_PIPE_DARK,
  C_OUTLINE,
  C_GRASS,
  C_GRASS_DARK,
  C_SAND,
  C_SAND_DARK,
  C_YELLOW,
  C_ORANGE,
  C_WHITE,
  C_WING,
  C_PANEL,
  C_PANEL_EDGE,
  C_RED,
  C_COUNT
};

static const uint8_t PALETTE[C_COUNT - C_SKY][3] = {
    {112, 197, 206}, /* sky */
    {150, 214, 220}, /* sky near horizon */
    {236, 250, 248}, /* cloud */
    {170, 220, 196}, /* city */
    {214, 238, 214}, /* city windows */
    {94, 196, 72},   /* bush */
    {72, 160, 56},   /* bush dark */
    {116, 190, 46},  /* pipe */
    {170, 232, 96},  /* pipe light */
    {74, 128, 30},   /* pipe dark */
    {44, 36, 52},    /* outline */
    {124, 206, 64},  /* grass */
    {90, 164, 44},   /* grass dark */
    {222, 214, 150}, /* sand */
    {204, 190, 120}, /* sand dark */
    {250, 208, 40},  /* bird yellow */
    {240, 112, 32},  /* beak orange */
    {255, 255, 255}, /* white */
    {255, 244, 196}, /* wing */
    {222, 214, 150}, /* panel */
    {124, 88, 52},   /* panel edge */
    {226, 72, 56},   /* red */
};

/* Layout and physics (positions and speeds in 1/256 px). */
#define GROUND_Y 200
#define GROUND_H (SCREEN_H - GROUND_Y)
#define BIRD_X 76
#define BIRD_W 17
#define BIRD_H 12
#define PIPE_W 36
#define CAP_W 42
#define CAP_H 12
#define GAP 70
#define PIPE_SPACING 152
#define PIPE_COUNT 3
#define SPEED 384    /* 1.5 px/frame */
#define GRAVITY 58   /* 0.23 px/frame^2 */
#define FLAP (-1100) /* -4.3 px/frame */
#define MAX_FALL 1700
#define FP 256

/* Main RAM work area (above the program and its stack at 1 MiB). */
#define WORK 0x00200000u
#define BG_SIZE (SCREEN_W * GROUND_Y)
#define BG(k) (WORK + (uint32_t)(k) * BG_SIZE)
#define PIPE_ROW (WORK + 4u * BG_SIZE)
#define PIPE_CAP (PIPE_ROW + 64u)
#define GROUND_STRIDE (SCREEN_W + 32)
#define GROUND (PIPE_CAP + CAP_W * CAP_H + 64u)
#define LABELS (GROUND + GROUND_STRIDE * GROUND_H + 64u)
#define LABEL_SIZE (SCREEN_W * 32)

/* Bird frames: K outline, Y body, W eye, L wing, O beak, '.' transparent. */
static const char *const BIRD_ART[3][BIRD_H] = {
    {".....KKKKKK......", "...KKYYYYKWWK....", "..KYYYYYKWWWWK...",
     ".KKKKYYYKWWWKWK..", "KLLLLKYYKWWWKWK..", "KLLLLLKYYKWWWWK..",
     "KYLLLYKYYYKKKKKK.", ".KYYYKYYYKOOOOOOK", "..KKKYYYKOKKKKKK.",
     "...KYYYYYKOOOOOK.", "....KKYYYYKKKKK..", "......KKKKK......"},
    {".....KKKKKK......", "...KKYYYYKWWK....", "..KYYYYYKWWWWK...",
     ".KYYYYYYKWWWKWK..", "KKKKKYYYKWWWKWK..", "KLLLLLKYYKWWWWK..",
     "KLLLLLLKYYKKKKKK.", ".KYYYYKYYKOOOOOOK", "..KKKYYYKOKKKKKK.",
     "...KYYYYYKOOOOOK.", "....KKYYYYKKKKK..", "......KKKKK......"},
    {".....KKKKKK......", "...KKYYYYKWWK....", "..KYYYYYKWWWWK...",
     ".KYYYYYYKWWWKWK..", "KYYYYKYYKWWWKWK..", "KYYYYYKYYKWWWWK..",
     "KKKKKKKYYYKKKKKK.", "KLLLLLKYYKOOOOOOK", "KLLLLKYYKOKKKKKK.",
     ".KKKKYYYYKOOOOOK.", "....KKYYYYKKKKK..", "......KKKKK......"},
};

static uint8_t bird_px[3][BIRD_W * BIRD_H];

enum { TITLE, PLAY, DYING, OVER };

typedef struct {
  int32_t x; /* left edge of the body, 1/256 px */
  int gap_y; /* top of the gap */
  int passed;
} pipe_t;

static pipe_t pipes[PIPE_COUNT];
static int state, state_timer, frame;
static int32_t bird_y, bird_vy, scroll;
static int score, best, flash;
static uint32_t rng = 0x2545F491u;
static uint8_t keys[256];
static int flap_pressed;

/* ---- drawing primitives ---- */

/* The surface that rect() draws into. */
typedef struct {
  uint32_t base;
  int stride, w, h;
} canvas_t;

static const canvas_t SCREEN = {VRAM, SCREEN_W, SCREEN_W, SCREEN_H};
static canvas_t canvas = {VRAM, SCREEN_W, SCREEN_W, SCREEN_H};

static void rect(int x, int y, int w, int h, int color) {
  uint32_t width = DMAC_WIDTH_8, value = (uint32_t)color & 255u;
  uint32_t dst;
  if (x < 0) { w += x; x = 0; }
  if (y < 0) { h += y; y = 0; }
  if (x + w > canvas.w) w = canvas.w - x;
  if (y + h > canvas.h) h = canvas.h - y;
  if (w <= 0 || h <= 0) return;
  dst = canvas.base + (uint32_t)(y * canvas.stride + x);
  if (((dst | (uint32_t)w | (uint32_t)canvas.stride) & 3u) == 0) {
    width = DMAC_WIDTH_32;
    value *= 0x01010101u;
  }
  dmac_fill(CH, dst, canvas.stride, (uint32_t)w, (uint32_t)h, width, value);
}

static void disc(int cx, int cy, int r, int color) {
  int dy, dx;
  for (dy = -r; dy <= r; ++dy) {
    dx = 0;
    while ((dx + 1) * (dx + 1) + dy * dy <= r * r) ++dx;
    rect(cx - dx, cy + dy, 2 * dx + 1, 1, color);
  }
}

/* Copies a w x h block from src (rows src_stride apart; 0 repeats one row)
 * to (x, y) of dst, clipped. Uses 32-bit beats when everything is aligned. */
static void blit_to(const canvas_t *dst, uint32_t src, int src_stride, int x,
                    int y, int w, int h, int keyed) {
  dmac_blit_t b;
  if (y < 0) { src += (uint32_t)(-y * src_stride); h += y; y = 0; }
  if (y + h > dst->h) h = dst->h - y;
  if (x < 0) { src += (uint32_t)-x; w += x; x = 0; }
  if (x + w > dst->w) w = dst->w - x;
  if (w <= 0 || h <= 0) return;
  b.src = src;
  b.dst = dst->base + (uint32_t)(y * dst->stride + x);
  b.row_bytes = (uint32_t)w;
  b.rows = (uint32_t)h;
  b.src_stride = src_stride;
  b.dst_stride = dst->stride;
  b.flags = DMAC_WIDTH_8;
  if (keyed)
    b.flags |= DMAC_KEY_EN;
  else if (((b.src | b.dst | b.row_bytes | (uint32_t)src_stride |
             (uint32_t)dst->stride) & 3u) == 0)
    b.flags = DMAC_WIDTH_32;
  b.key = 0;
  dmac_blit(CH, &b);
}

static void blit(uint32_t src, int src_stride, int x, int y, int w, int h,
                 int keyed) {
  blit_to(&SCREEN, src, src_stride, x, y, w, h, keyed);
}

/* ---- text ---- */

/* 5x7 font, one byte per row, bit 4 = leftmost column. */
static const uint8_t *glyph(char c) {
  static const uint8_t font[][7] = {
      {0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E}, /* 0 */
      {0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E}, /* 1 */
      {0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F}, /* 2 */
      {0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E}, /* 3 */
      {0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02}, /* 4 */
      {0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E}, /* 5 */
      {0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E}, /* 6 */
      {0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08}, /* 7 */
      {0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E}, /* 8 */
      {0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C}, /* 9 */
      {0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11}, /* A */
      {0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E}, /* B */
      {0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E}, /* C */
      {0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E}, /* D */
      {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F}, /* E */
      {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10}, /* F */
      {0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F}, /* G */
      {0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11}, /* H */
      {0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E}, /* I */
      {0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F}, /* L */
      {0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11}, /* M */
      {0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11}, /* N */
      {0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E}, /* O */
      {0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10}, /* P */
      {0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11}, /* R */
      {0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E}, /* S */
      {0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04}, /* T */
      {0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E}, /* U */
      {0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04}, /* V */
      {0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04}, /* Y */
  };
  static const char letters[] = "ABCDEFGHILMNOPRSTUVY";
  int i;
  if (c >= '0' && c <= '9') return font[c - '0'];
  for (i = 0; letters[i]; ++i)
    if (letters[i] == c) return font[10 + i];
  return 0; /* space and unsupported characters */
}

static void glyph_runs(const uint8_t *g, int x, int y, int s, int color) {
  int row, col, start;
  for (row = 0; row < 7; ++row) {
    for (col = 0; col < 5;) {
      if (!(g[row] & (0x10 >> col))) { ++col; continue; }
      start = col;
      while (col < 5 && (g[row] & (0x10 >> col))) ++col;
      rect(x + start * s, y + row * s, (col - start) * s, s, color);
    }
  }
}

/* A label is text pre-rendered (with a 1-pixel dark outline) into its own
 * work buffer and redrawn only when its text, scale or color changes. */
enum {
  L_TITLE,
  L_PRESS,
  L_GAME_OVER,
  L_SCORE,
  L_BEST,
  L_BEST_WHITE,
  L_SCORE_NUM,
  L_PANEL_SCORE,
  L_PANEL_BEST,
  L_TITLE_BEST,
  LABEL_COUNT
};

typedef struct {
  char str[16];
  int scale, color, w, h;
} label_t;

static label_t labels[LABEL_COUNT];

static void label_render(int slot, const char *t, int s, int color) {
  label_t *l = &labels[slot];
  int i, n, pass, same = l->scale == s && l->color == color;
  const uint8_t *g;
  for (n = 0; t[n] && n < 15; ++n)
    if (l->str[n] != t[n]) same = 0;
  if (same && l->str[n] == 0) return;
  for (i = 0; i < n; ++i) l->str[i] = t[i];
  l->str[n] = 0;
  l->scale = s;
  l->color = color;
  l->w = n * 6 * s - s + 2;
  l->h = 7 * s + 2;
  canvas.base = LABELS + (uint32_t)slot * LABEL_SIZE;
  canvas.stride = canvas.w = l->w;
  canvas.h = l->h;
  rect(0, 0, l->w, l->h, 0);
  for (pass = 0; pass < 5; ++pass) {
    static const int8_t dx[5] = {0, 2, 0, 2, 1}, dy[5] = {0, 0, 2, 2, 1};
    for (i = 0; i < n; ++i) {
      g = glyph(t[i]);
      if (g)
        glyph_runs(g, i * 6 * s + dx[pass], dy[pass], s,
                   pass < 4 ? C_OUTLINE : color);
    }
  }
  canvas = SCREEN;
}

/* Draws a label with its text's top-left at (x, y); x < 0 centers it. */
static void label(int slot, const char *t, int x, int y, int s, int color) {
  label_render(slot, t, s, color);
  if (x < 0) x = (SCREEN_W - labels[slot].w) / 2 + 1;
  blit(LABELS + (uint32_t)slot * LABEL_SIZE, labels[slot].w, x - 1, y - 1,
       labels[slot].w, labels[slot].h, 1);
}

static void label_number(int slot, int value, int x, int y, int s,
                         int color) {
  char buf[12];
  int n = 11;
  buf[n] = 0;
  do {
    buf[--n] = (char)('0' + value % 10);
    value /= 10;
  } while (value && n > 0);
  label(slot, buf + n, x, y, s, color);
}

/* ---- one-time setup ---- */

static void set_palette(void) {
  int i;
  for (i = 0; i < C_COUNT - C_SKY; ++i) {
    U8(CLUT + (uint32_t)(C_SKY + i) * 3u + 0u) = PALETTE[i][0];
    U8(CLUT + (uint32_t)(C_SKY + i) * 3u + 1u) = PALETTE[i][1];
    U8(CLUT + (uint32_t)(C_SKY + i) * 3u + 2u) = PALETTE[i][2];
  }
}

static void decode_bird(void) {
  int f, i;
  for (f = 0; f < 3; ++f) {
    for (i = 0; i < BIRD_W * BIRD_H; ++i) {
      uint8_t c = 0;
      switch (BIRD_ART[f][i / BIRD_W][i % BIRD_W]) {
      case 'K': c = C_OUTLINE; break;
      case 'Y': c = C_YELLOW; break;
      case 'W': c = C_WHITE; break;
      case 'L': c = C_WING; break;
      case 'O': c = C_ORANGE; break;
      }
      bird_px[f][i] = c;
    }
  }
}

static void use_canvas(uint32_t base, int stride, int w, int h) {
  canvas.base = base;
  canvas.stride = stride;
  canvas.w = w;
  canvas.h = h;
}

static void build_background(void) {
  int i, k, x, h;
  use_canvas(BG(0), SCREEN_W, SCREEN_W, GROUND_Y);
  rect(0, 0, SCREEN_W, GROUND_Y, C_SKY);
  rect(0, 120, SCREEN_W, GROUND_Y - 120, C_SKY2);
  /* Clouds: overlapping discs along the horizon. */
  for (i = 0; i < 11; ++i) {
    x = i * 32 + (i * 7) % 13;
    disc(x, 136 + (i * 5) % 9, 14 + (i * 3) % 7, C_CLOUD);
  }
  rect(0, 140, SCREEN_W, GROUND_Y - 140, C_CLOUD);
  /* City skyline. */
  for (x = 0, i = 0; x < SCREEN_W; ++i) {
    int w = 14 + (i * 11) % 13;
    h = 18 + (i * 17) % 26;
    rect(x, 168 - h, w, h + 32, C_CITY);
    for (int wy = 168 - h + 4; wy < 176; wy += 6)
      for (int wx = x + 3; wx + 2 < x + w - 2; wx += 5)
        rect(wx, wy, 2, 3, C_CITY_WIN);
    x += w + (i % 3 == 0 ? 2 : 0);
  }
  /* Bushes. */
  for (i = 0; i < 14; ++i) {
    x = i * 24 + (i * 5) % 9;
    disc(x, 186 + (i * 3) % 5, 12 + (i * 7) % 5, C_BUSH_DARK);
  }
  for (i = 0; i < 14; ++i) {
    x = i * 24 + 10 + (i * 3) % 7;
    disc(x, 190 + (i * 5) % 4, 10 + (i * 5) % 4, C_BUSH);
  }
  rect(0, 192, SCREEN_W, GROUND_Y - 192, C_BUSH);
  /* Copy k holds each row rotated left by k pixels: BG(k)[j] = BG(0)[j+k]. */
  for (k = 1; k < 4; ++k) {
    canvas_t dst = {BG(k), SCREEN_W, SCREEN_W, GROUND_Y};
    blit_to(&dst, BG(0) + (uint32_t)k, SCREEN_W, 0, 0, SCREEN_W - k, GROUND_Y,
            0);
    blit_to(&dst, BG(0), SCREEN_W, SCREEN_W - k, 0, k, GROUND_Y, 0);
  }
  canvas = SCREEN;
}

static void build_sprites(void) {
  int x;
  /* One row of the pipe body; the body is this row repeated. */
  use_canvas(PIPE_ROW, PIPE_W, PIPE_W, 1);
  rect(0, 0, PIPE_W, 1, C_OUTLINE);
  rect(1, 0, PIPE_W - 2, 1, C_PIPE);
  rect(4, 0, 6, 1, C_PIPE_LIGHT);
  rect(PIPE_W - 8, 0, 6, 1, C_PIPE_DARK);
  use_canvas(PIPE_CAP, CAP_W, CAP_W, CAP_H);
  rect(0, 0, CAP_W, CAP_H, C_OUTLINE);
  rect(1, 1, CAP_W - 2, CAP_H - 2, C_PIPE);
  rect(4, 1, 6, CAP_H - 2, C_PIPE_LIGHT);
  rect(CAP_W - 8, 1, 6, CAP_H - 2, C_PIPE_DARK);
  /* Ground strip, 32 pixels wider than the screen; its pattern repeats every
   * 32 pixels, so scrolling is an offset of 0..31 into it. */
  use_canvas(GROUND, GROUND_STRIDE, GROUND_STRIDE, GROUND_H);
  rect(0, 0, GROUND_STRIDE, 2, C_OUTLINE);
  rect(0, 2, GROUND_STRIDE, 8, C_GRASS);
  for (x = 0; x < GROUND_STRIDE; x += 16) rect(x, 2, 8, 8, C_GRASS_DARK);
  rect(0, 10, GROUND_STRIDE, 2, C_GRASS_DARK);
  rect(0, 12, GROUND_STRIDE, GROUND_H - 12, C_SAND);
  for (x = 0; x < GROUND_STRIDE; x += 32) rect(x, 20, 16, 4, C_SAND_DARK);
  canvas = SCREEN;
}

/* ---- game ---- */

static uint32_t rand32(void) {
  rng ^= rng << 13;
  rng ^= rng >> 17;
  rng ^= rng << 5;
  return rng;
}

static int random_gap(void) {
  int lo = 36, hi = GROUND_Y - 36 - GAP;
  return lo + (int)(rand32() % (uint32_t)(hi - lo + 1));
}

static void reset_world(void) {
  int i;
  for (i = 0; i < PIPE_COUNT; ++i) {
    pipes[i].x = (SCREEN_W + 60 + i * PIPE_SPACING) * FP;
    pipes[i].gap_y = random_gap();
    pipes[i].passed = 0;
  }
  bird_y = 100 * FP;
  bird_vy = 0;
  score = 0;
}

static void keyboard(void) {
  while (U32(0x80040034u) & 1u) {
    uint32_t event = U32(0x80040030u);
    unsigned usage = (event >> 16) & 255u;
    if ((event >> 24) == 0) {
      int down = (event & 0xFFFFu) != 0;
      int flap_key = usage == 44 || usage == 82 || usage == 26 || usage == 40;
      if (down && !keys[usage] && flap_key) flap_pressed = 1;
      keys[usage] = (uint8_t)down;
    }
    U32(0x80040038u) = 1;
  }
}

static int hits_pipe(void) {
  int i, by = bird_y / FP;
  int bx0 = BIRD_X + 2, bx1 = BIRD_X + BIRD_W - 2;
  int by0 = by + 2, by1 = by + BIRD_H - 2;
  for (i = 0; i < PIPE_COUNT; ++i) {
    int px = pipes[i].x / FP;
    int cx0 = px - (CAP_W - PIPE_W) / 2, cx1 = cx0 + CAP_W;
    if (bx1 <= cx0 || bx0 >= cx1) continue;
    if (by0 < pipes[i].gap_y || by1 > pipes[i].gap_y + GAP) return 1;
  }
  return 0;
}

static void update(void) {
  int i;
  ++frame;
  ++state_timer;
  if (state == TITLE || state == PLAY) scroll += SPEED;
  switch (state) {
  case TITLE:
    /* Gentle bobbing: triangle wave of period 64 frames. */
    i = frame & 63;
    bird_y = (96 + (i < 32 ? i : 64 - i) / 4) * FP;
    if (flap_pressed) {
      rng ^= (uint32_t)frame * 2654435761u;
      if (!rng) rng = 1;
      reset_world();
      state = PLAY;
      state_timer = 0;
      bird_vy = FLAP;
    }
    break;
  case PLAY:
#ifdef FLAPPY_AUTOPILOT
    /* Test build: flap when falling below the next gap's middle. */
    {
      int next = -1;
      for (i = 0; i < PIPE_COUNT; ++i)
        if (pipes[i].x / FP + PIPE_W >= BIRD_X - 4 &&
            (next < 0 || pipes[i].x < pipes[next].x))
          next = i;
      if (next >= 0 && bird_vy > 0 &&
          bird_y / FP + BIRD_H > pipes[next].gap_y + GAP / 2 + 12)
        flap_pressed = 1;
    }
#endif
    if (flap_pressed) bird_vy = FLAP;
    bird_vy += GRAVITY;
    if (bird_vy > MAX_FALL) bird_vy = MAX_FALL;
    bird_y += bird_vy;
    if (bird_y < 0) { bird_y = 0; bird_vy = 0; }
    for (i = 0; i < PIPE_COUNT; ++i) {
      pipes[i].x -= SPEED;
      if (!pipes[i].passed && pipes[i].x / FP + PIPE_W < BIRD_X) {
        pipes[i].passed = 1;
        ++score;
        if (score > best) best = score;
      }
      if (pipes[i].x / FP + CAP_W < 0) {
        int32_t farthest = pipes[0].x;
        int j;
        for (j = 1; j < PIPE_COUNT; ++j)
          if (pipes[j].x > farthest) farthest = pipes[j].x;
        pipes[i].x = farthest + PIPE_SPACING * FP;
        pipes[i].gap_y = random_gap();
        pipes[i].passed = 0;
      }
    }
    if (hits_pipe() || bird_y / FP + BIRD_H >= GROUND_Y) {
      state = DYING;
      state_timer = 0;
      flash = 3;
      if (bird_vy < 0) bird_vy = 0;
    }
    break;
  case DYING:
    bird_vy += GRAVITY * 2;
    if (bird_vy > MAX_FALL * 2) bird_vy = MAX_FALL * 2;
    bird_y += bird_vy;
    if (bird_y / FP + BIRD_H >= GROUND_Y) {
      bird_y = (GROUND_Y - BIRD_H) * FP;
      state = OVER;
      state_timer = 0;
    }
    break;
  case OVER:
    if (flap_pressed && state_timer > 30) {
      state = TITLE;
      state_timer = 0;
      reset_world();
    }
    break;
  }
  flap_pressed = 0;
}

static void draw_pipe(const pipe_t *p) {
  int x = p->x / FP;
  int cx = x - (CAP_W - PIPE_W) / 2;
  int top_cap = p->gap_y - CAP_H, bottom = p->gap_y + GAP + CAP_H;
  blit(PIPE_ROW, 0, x, 0, PIPE_W, top_cap, 0);
  blit(PIPE_ROW, 0, x, bottom, PIPE_W, GROUND_Y - bottom, 0);
  blit(PIPE_CAP, CAP_W, cx, top_cap, CAP_W, CAP_H, 0);
  blit(PIPE_CAP, CAP_W, cx, p->gap_y + GAP, CAP_W, CAP_H, 0);
}

static void draw_bird(void) {
  int f;
  if (state == DYING || state == OVER)
    f = 1;
  else if (state == PLAY && bird_vy > 600)
    f = 1; /* gliding down: wing level */
  else
    f = (frame / 5) % 3;
  blit((uint32_t)(uintptr_t)bird_px[f], BIRD_W, BIRD_X, bird_y / FP, BIRD_W,
       BIRD_H, 1);
}

static void draw(void) {
  int i, off, q;
  uint32_t bg;
  if (flash > 0) {
    --flash;
    rect(0, 0, SCREEN_W, SCREEN_H, C_WHITE);
    return;
  }
  /* Parallax background at 1/4 of the ground speed. */
  off = (scroll / FP / 4) % SCREEN_W;
  q = off & ~3;
  bg = BG(off & 3);
  blit(bg + (uint32_t)q, SCREEN_W, 0, 0, SCREEN_W - q, GROUND_Y, 0);
  if (q) blit(bg, SCREEN_W, SCREEN_W - q, 0, q, GROUND_Y, 0);
  if (state != TITLE)
    for (i = 0; i < PIPE_COUNT; ++i) draw_pipe(&pipes[i]);
  blit(GROUND + (uint32_t)((scroll / FP) % 32), GROUND_STRIDE, 0, GROUND_Y,
       SCREEN_W, GROUND_H, 0);
  draw_bird();
  switch (state) {
  case TITLE:
    label(L_TITLE, "FLAPPY BIRD", -1, 40, 3, C_YELLOW);
    if ((frame / 30) & 1) label(L_PRESS, "PRESS SPACE", -1, 150, 2, C_WHITE);
    if (best) {
      label(L_BEST_WHITE, "BEST", 116, 218, 2, C_WHITE);
      label_number(L_TITLE_BEST, best, 176, 218, 2, C_WHITE);
    }
    break;
  case PLAY:
  case DYING:
    label_number(L_SCORE_NUM, score, -1, 16, 4, C_WHITE);
    break;
  case OVER:
    label(L_GAME_OVER, "GAME OVER", -1, 48, 3, C_RED);
    rect(76, 92, 168, 72, C_OUTLINE);
    rect(78, 94, 164, 68, C_PANEL_EDGE);
    rect(81, 97, 158, 62, C_PANEL);
    label(L_SCORE, "SCORE", 92, 106, 2, C_ORANGE);
    label_number(L_PANEL_SCORE, score, 92, 128, 2, C_WHITE);
    label(L_BEST, "BEST", 172, 106, 2, C_ORANGE);
    label_number(L_PANEL_BEST, best, 172, 128, 2, C_WHITE);
    if (state_timer > 30 && ((frame / 30) & 1))
      label(L_PRESS, "PRESS SPACE", -1, 176, 2, C_WHITE);
    break;
  }
}

/* Renders the fixed labels up front so they never cost a frame later. */
static void build_labels(void) {
  label_render(L_TITLE, "FLAPPY BIRD", 3, C_YELLOW);
  label_render(L_PRESS, "PRESS SPACE", 2, C_WHITE);
  label_render(L_GAME_OVER, "GAME OVER", 3, C_RED);
  label_render(L_SCORE, "SCORE", 2, C_ORANGE);
  label_render(L_BEST, "BEST", 2, C_ORANGE);
  label_render(L_BEST_WHITE, "BEST", 2, C_WHITE);
  label_render(L_SCORE_NUM, "0", 4, C_WHITE);
}

#ifdef FLAPPY_PROFILE
/* Debug build (-DFLAPPY_PROFILE, link print.c): prints the cycles each draw()
 * takes, measured with PeC timer 0 at the 48 MHz system clock, to the UART. */
#include <cpt32/print.h>
static void profile_start(void) {
  U32(0x80040400u) = 0x12u; /* timer 0: reset, free-running */
  U32(0x80040420u) = 0u;    /* prescaler: count every cycle */
  U32(0x80040184u) = 1u;
  U32(0x80040400u) = 0x06u; /* free-running, enable */
}
static uint32_t profile_now(void) { return U32(0x80040408u); }
static void profile_print(uint32_t v) {
  char b[11];
  int n = 10;
  b[n] = 0;
  do {
    b[--n] = (char)('0' + v % 10);
    v /= 10;
  } while (v);
  print_prnln(b + n);
}
#endif

int main(void) {
  irqc_init();
  U8(0x80000000u) = 1; /* display on */
  U8(0x80000001u) = 0; /* graphics mode (GP0 bitmap) */
  U8(0x80000004u) = 0; /* 256-color CLUT */
  set_palette();
  if (!dmac_has_blit()) {
    for (;;) {} /* this game draws only with DMAC BitBlt */
  }
  decode_bird();
  build_background();
  build_sprites();
  build_labels();
  reset_world();
  state = TITLE;
#ifdef FLAPPY_PROFILE
  profile_start();
#endif
  irqc_ack(1);
  irqc_enable(1);
  for (;;) {
    irqc_wait_vsync();
    keyboard();
    update();
#ifdef FLAPPY_PROFILE
    {
      uint32_t t0 = profile_now();
      draw();
      profile_print(profile_now() - t0);
    }
#else
    draw();
#endif
  }
}
