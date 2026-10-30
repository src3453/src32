#include <stdint.h>

#define U8(a) (*(volatile uint8_t *)(uintptr_t)(a))
#define U32(a) (*(volatile uint32_t *)(uintptr_t)(a))
#define SI22 0x80022000u
#define PCM 0x18000000u
#define SGC 0x80010000u
#define CLUT 0x10012c00u
#define COUNT 160u
#define LEFT 0u
#define RIGHT 0x400u

/* The FIFO is drained on VDP ticks. Never write a partial command into a full FIFO. */
static void word(uint32_t value) {
  while ((U32(SGC + 0x24) & 0xffffu) == 0) {}
  U32(SGC + 0x20) = value;
}
static void color(uint32_t c) { word(0x01000000u | c); }
static void line(uint32_t x0, uint32_t y0, uint32_t x1, uint32_t y1) {
  word(0x10000000u); word(x0); word(y0); word(x1); word(y1);
}
static void text(const char *s, uint32_t x, uint32_t y) {
  while (*s) {
    word(0x13000000u); word((uint8_t)*s++); word(x); word(y); x += 8;
  }
}
static void address(uint32_t offset, uint32_t value) {
  U8(SI22 + offset) = (uint8_t)(value >> 16);
  U8(SI22 + offset + 1) = (uint8_t)(value >> 8);
  U8(SI22 + offset + 2) = (uint8_t)value;
}
static uint32_t sample_y(uint32_t base, uint32_t i, int32_t center) {
  uint32_t a = PCM + base + i * 2u;
  uint32_t raw = ((uint32_t)U8(a) << 8) | U8(a + 1);
  int32_t s = raw < 32768u ? (int32_t)raw : (int32_t)raw - 65536;
  return (uint32_t)(center - (s * 45 / 32768));
}
static void trace(uint32_t base, int32_t center) {
  uint32_t i, prev = sample_y(base, 0, center);
  for (i = 1; i < COUNT; ++i) {
    uint32_t y = sample_y(base, i, center);
    line(2u * (i - 1u), prev, 2u * i, y);
    prev = y;
  }
}
int main(void) {
  U8(0x80000000u) = 1;
  U8(0x80000001u) = 0;
  U8(0x80000004u) = 0;
  /* SGC and GP share the palette. */
  U8(CLUT + 0) = 0; U8(CLUT + 1) = 0; U8(CLUT + 2) = 0;
  U8(CLUT + 3) = 48; U8(CLUT + 4) = 48; U8(CLUT + 5) = 48;
  U8(CLUT + 6) = 64; U8(CLUT + 7) = 255; U8(CLUT + 8) = 128;
  U8(CLUT + 9) = 64; U8(CLUT + 10) = 180; U8(CLUT + 11) = 255;
  U8(CLUT + 12) = 255; U8(CLUT + 13) = 255; U8(CLUT + 14) = 255;
  U32(SGC + 4) = 2;
  U32(SGC + 0x18) = 7;
  U32(SGC + 4) = 1;
  address(0x10, LEFT); address(0x13, LEFT + COUNT * 2u);
  address(0x16, RIGHT); address(0x19, RIGHT + COUNT * 2u);
  U8(SI22 + 0x1d) = 0; /* 0 dB */
  U8(SI22 + 0x1e) = 0; U8(SI22 + 0x1f) = 0; /* 48 kHz */
  U8(SI22 + 0x20) = 0; /* no AGC */
  U8(SI22 + 0x22) = 0x11; /* stereo PCM16, big endian */
  for (;;) {
    U8(SI22 + 0x1c) = 0x05; /* start REC A and B */
    while ((U8(SI22 + 0x1c) & 15u) != 15u) {}
    color(0); word(0x14000000u);
    color(1); line(0, 65, 319, 65); line(0, 175, 319, 175);
    color(4); text("SGU V3 INPUT  48KHZ  3.33MS", 8, 0);
    text("L", 8, 16); text("R", 8, 126);
    color(2); trace(LEFT, 65);
    color(3); trace(RIGHT, 175);
  }
}
