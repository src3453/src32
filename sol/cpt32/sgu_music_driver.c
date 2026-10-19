/* SGUB v1/v2 guest driver. Build with sdk/cpt32/src/sgub.c and irqc.c. */
#include <cpt32/cpu.h>
#include <cpt32/irqc.h>
#include <cpt32/mmio.h>
#include <cpt32/pcg.h>
#include <cpt32/sgub.h>
#include <stdint.h>

#define SGUB_STREAM_BASE UINT32_C(0x00200000)
#define SGUB_MAX_SIZE UINT32_C(0x00E00000)
#define IRQ0_MASK UINT16_C(1)
#define SGU0_CHANNELS UINT32_C(0x80020800)
#define SGU1_CHANNELS UINT32_C(0x80021800)

static const char hex_digits[] = "0123456789ABCDEF";

void sgub_visualizer_put_hex(uint32_t value, uint32_t digits) {
  while (digits != 0u) {
    --digits;
    pcg_putcscr((uint8_t)hex_digits[(value >> (digits * 4u)) & 15u]);
  }
}

void sgub_visualizer_draw(void) {
  uint32_t voice;
  pcg_locate(0u, 0u);
  pcg_prnscr("CPT32 SGUB  VOL  FREQUENCY");
  for (voice = 0; voice < 16u; ++voice) {
    uintptr_t base = voice < 8u
                         ? SGU0_CHANNELS + (uintptr_t)voice * 0x20u
                         : SGU1_CHANNELS + (uintptr_t)(voice - 8u) * 0x20u;
    uint32_t frequency =
        ((uint32_t)CPT32_MMIO8(base) << 8) | CPT32_MMIO8(base + 1u);
    uint32_t volume = CPT32_MMIO8(base + 3u);
    pcg_locate(0u, voice + 2u);
    pcg_prnscr("CH ");
    sgub_visualizer_put_hex(voice, 2u);
    pcg_prnscr("  VOL ");
    sgub_visualizer_put_hex(volume, 2u);
    pcg_prnscr("  FREQ ");
    sgub_visualizer_put_hex(frequency, 4u);
    pcg_prnscr("   ");
  }
}

void sgub_music_irq0(void) {
  if (sgub_step() == SGUB_STATUS_COMPLETE) {
    irqc_disable(IRQ0_MASK);
  } else {
    sgub_visualizer_draw();
  }
}

int main(void) {
  const uint8_t *stream = (const uint8_t *)(uintptr_t)SGUB_STREAM_BASE;
  uint32_t size;
  uint32_t size_offset;

  /* Validate the fixed header before using its declared size. */
  if (CPT32_MMIO8(SGUB_STREAM_BASE) != 'S' ||
      CPT32_MMIO8(SGUB_STREAM_BASE + 1u) != 'G' ||
      CPT32_MMIO8(SGUB_STREAM_BASE + 2u) != 'U' ||
      CPT32_MMIO8(SGUB_STREAM_BASE + 3u) != 'B')
    return 1;
  if (CPT32_MMIO8(SGUB_STREAM_BASE + 4u) == 1u)
    size_offset = 16u;
  else if (CPT32_MMIO8(SGUB_STREAM_BASE + 4u) == 2u)
    size_offset = 32u;
  else
    return 1;
  size = ((uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset) << 24) |
         ((uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset + 1u) << 16) |
         ((uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset + 2u) << 8) |
         (uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset + 3u);
  if (size < 20u || size > SGUB_MAX_SIZE)
    return 1;

  irqc_init();
  if (sgub_init(stream, size) != SGUB_STATUS_PLAYING)
    return 1;
  pcg_initPCG();
  sgub_visualizer_draw();
  if (irqc_register(0u, sgub_music_irq0) != 0)
    return 1;
  irqc_enable(IRQ0_MASK);

  for (;;) {
    const sgub_state_t *state = sgub_state();
    if (state->status != SGUB_STATUS_PLAYING)
      break;
    cpt32_halt();
  }
  return 0;
}
