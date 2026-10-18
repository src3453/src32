#include <cpt32/sgub.h>
#include <stdint.h>

#define PCM_BASE UINT32_C(0x18000000)
#define SGU0_BASE UINT32_C(0x80020000)
#define SGU1_BASE UINT32_C(0x80021000)

static uint8_t stream[512];

static void put_u16(uint8_t *p, uint16_t value) {
  p[0] = (uint8_t)(value >> 8);
  p[1] = (uint8_t)value;
}

static void put_u24(uint8_t *p, uint32_t value) {
  p[0] = (uint8_t)(value >> 16);
  p[1] = (uint8_t)(value >> 8);
  p[2] = (uint8_t)value;
}

static void put_u32(uint8_t *p, uint32_t value) {
  p[0] = (uint8_t)(value >> 24);
  p[1] = (uint8_t)(value >> 16);
  p[2] = (uint8_t)(value >> 8);
  p[3] = (uint8_t)value;
}

static uint8_t *put_record(uint8_t *p, uint8_t target, uint32_t address,
                           uint8_t value) {
  p[0] = target;
  put_u24(p + 1, address);
  p[4] = value;
  return p + 5;
}

static uint8_t *put_operation(uint8_t *p, uint8_t opcode, uint8_t target,
                              uint32_t address) {
  p[0] = opcode;
  p[1] = target;
  put_u24(p + 2, address);
  return p + 5;
}

static size_t make_v1(void) {
  uint8_t *p = stream;
  p[0] = 'S';
  p[1] = 'G';
  p[2] = 'U';
  p[3] = 'B';
  p[4] = 1;
  p[5] = 16;
  p[6] = 1;
  p[7] = 0;
  put_u32(p + 8, 1);
  put_u32(p + 12, 1);
  put_u32(p + 16, 32);
  p = put_record(p + 20, 2, 0x10, 0x55);
  put_u16(p, 1);
  put_record(p + 2, 2, 0x11, 0x66);
  return 32;
}

static size_t make_v2(void) {
  uint8_t *p = stream;
  uint32_t i;
  p[0] = 'S';
  p[1] = 'G';
  p[2] = 'U';
  p[3] = 'B';
  p[4] = 2;
  p[5] = 16;
  p[6] = 0;
  p[7] = 0;
  put_u32(p + 8, 2);
  put_u32(p + 12, 2);
  put_u32(p + 16, 3);
  put_u32(p + 20, 40);
  put_u32(p + 24, 332);
  put_u32(p + 28, 372);
  put_u32(p + 32, 388);
  put_u16(p + 36, 40);
  put_u16(p + 38, 0);

  /* Pool 0: wave bytes; pool 1: PCM bytes. */
  put_u16(p + 40, 0);
  p[42] = 1;
  p[43] = 0;
  put_u32(p + 44, 256);
  put_u32(p + 48, 256);
  put_u32(p + 52, 72);
  put_u16(p + 56, 1);
  p[58] = 2;
  p[59] = 0;
  put_u32(p + 60, 4);
  put_u32(p + 64, 4);
  put_u32(p + 68, 328);
  for (i = 0; i < 256; ++i)
    p[72 + i] = (uint8_t)(i ^ 0x5a);
  p[328] = 0x11;
  p[329] = 0x22;
  p[330] = 0x33;
  p[331] = 0x44;

  /* Initial silence fill plus wave and PCM pool copies. */
  p = put_operation(p + 332, 1, 2, 0x20);
  put_u32(p, 4);
  p[4] = 0x80;
  p = put_operation(p + 5, 2, 0, 0x100);
  put_u16(p, 0);
  put_u32(p + 2, 0);
  put_u32(p + 6, 256);
  p = put_operation(p + 10, 2, 2, 0x30);
  put_u16(p, 1);
  put_u32(p + 2, 0);
  put_u32(p + 6, 4);
  p += 10;

  /* Two frames; preserve operation order and exercise writes to both chips. */
  put_u16(p, 1);
  p = put_operation(p + 2, 0, 1, 3);
  *p++ = 0x7f;
  put_u16(p, 1);
  p = put_operation(p + 2, 0, 2, 0x20);
  *p++ = 0x42;
  (void)p;
  return 388;
}

static uint8_t check_v1(void) {
  if (sgub_init(stream, make_v1()) != SGUB_STATUS_PLAYING)
    return 1;
  if (*(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x10) != 0x55)
    return 2;
  if (sgub_step() != SGUB_STATUS_READY ||
      *(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x11) != 0x66)
    return 3;
  if (sgub_step() != SGUB_STATUS_READY ||
      *(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x11) != 0x66)
    return 4;
  return 0;
}

static uint8_t check_v2(void) {
  uint32_t i;
  if (sgub_init(stream, make_v2()) != SGUB_STATUS_PLAYING)
    return 5;
  for (i = 0; i < 4; ++i) {
    if (*(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x20 + i) != 0x80 ||
        *(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x30 + i) !=
            0x11u * (i + 1u))
      return 6;
  }
  for (i = 0; i < 256; ++i) {
    if (*(volatile uint8_t *)(uintptr_t)(SGU0_BASE + 0x100 + i) !=
        (uint8_t)(i ^ 0x5a))
      return 7;
  }
  if (sgub_step() != SGUB_STATUS_READY ||
      *(volatile uint8_t *)(uintptr_t)(SGU1_BASE + 3) != 0x7f)
    return 8;
  if (sgub_step() != SGUB_STATUS_READY ||
      *(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x20) != 0x42)
    return 9;
  if (sgub_step() != SGUB_STATUS_COMPLETE)
    return 10;
  return 0;
}

int main(void) {
  uint8_t result = check_v1();
  if (result != 0)
    return result;
  result = check_v2();
  if (result != 0)
    return result;
  /* A malformed final frame must prevent even valid initial writes. */
  make_v2();
  stream[387] = 0x99;
  stream[382] = 9; /* Invalid final operation opcode. */
  *(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x20) = 0x77;
  if (sgub_init(stream, 388) >= 0)
    return 12;
  if (*(volatile uint8_t *)(uintptr_t)(PCM_BASE + 0x20) != 0x77)
    return 13;
  return 42;
}
