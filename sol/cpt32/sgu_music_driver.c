/* SGUB v1..v4 guest driver. Build with sdk/cpt32/src/sgub.c and irqc.c.
 * v4 streams with a frame period run from PeC timer 0 (IRQ4) instead of
 * VSync (IRQ0); see doc/spec_SGUB_v4.md. */
#include <cpt32/cpu.h>
#include <cpt32/irqc.h>
#include <cpt32/mmio.h>
#include <cpt32/pcg.h>
#include <cpt32/sgub.h>
#include <stdint.h>

#define SGUB_STREAM_BASE UINT32_C(0x00200000)
#define SGUB_MAX_SIZE UINT32_C(0x00E00000)
#define SGUB_CARD0_BASE UINT32_C(0xA0000000)
#define SGUB_3WS88PWN4_ID_OFFSET UINT32_C(0x004003F0)
#define IRQ0_MASK UINT16_C(1)
#define IRQ4_MASK UINT16_C(0x10)
#define PEC_TIMER_ENABLE UINT32_C(0x80040184)
#define PEC_TIMER_ACK UINT32_C(0x8004018C)
#define PEC_TIMER0_CTRL UINT32_C(0x80040400)
#define PEC_TIMER0_COMPARE_LO UINT32_C(0x80040410)
#define PEC_TIMER0_COMPARE_HI UINT32_C(0x80040414)
#define PEC_TIMER0_RELOAD_LO UINT32_C(0x80040418)
#define PEC_TIMER0_RELOAD_HI UINT32_C(0x8004041C)
#define PEC_TIMER0_PRESCALER UINT32_C(0x80040420)
#define PEC_TIMER_RESET UINT32_C(0x10)
/* Periodic mode, ENABLE, IRQ_ENABLE. */
#define PEC_TIMER_PERIODIC_IRQ UINT32_C(0x0D)
#define SGU0_CHANNELS UINT32_C(0x80020800)
#define SGU1_CHANNELS UINT32_C(0x80021800)

static const char hex_digits[] = "0123456789ABCDEF";
static const uint8_t card_id[] = {'3', 'W', 'S', '8', '8', 'P', 'W', 'N', '4'};

static uint8_t sgub_has_3ws88pwn4(void) {
  uint32_t i;
  for (i = 0; i < sizeof(card_id); ++i) {
    if (CPT32_MMIO8(SGUB_CARD0_BASE + SGUB_3WS88PWN4_ID_OFFSET + i) !=
        card_id[i])
      return 0;
  }
  return 1;
}

void sgub_visualizer_put_hex(uint32_t value, uint32_t digits) {
  while (digits != 0u) {
    --digits;
    pcg_putcscr((uint8_t)hex_digits[(value >> (digits * 4u)) & 15u]);
  }
}

void sgub_visualizer_draw(void) {
  uint32_t voice;
  pcg_locate(0u, 0u);
  pcg_prnlnscr("CPT32 SGUB Driver v4");
  /*pcg_prnscr("CPT32 SGUB  VOL  FREQUENCY");
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
  }*/
}

void sgub_music_irq0(void) {
  if (sgub_step() == SGUB_STATUS_COMPLETE) {
    irqc_disable(IRQ0_MASK);
  } else {
    //sgub_visualizer_draw();
  }
}

/* v4 timer frame. Clear the PeC pending bit first; the IRQC dispatcher ACKs
 * IRQ4 itself after this returns. */
void sgub_music_timer(void) {
  CPT32_MMIO32(PEC_TIMER_ACK) = 1u;
  if (sgub_step() != SGUB_STATUS_READY) {
    CPT32_MMIO32(PEC_TIMER0_CTRL) = 0u;
    irqc_disable(IRQ4_MASK);
  }
}

/* PeC timer 0, periodic, first deadline one period after the reset. */
static void sgub_start_timer(uint32_t period) {
  CPT32_MMIO32(PEC_TIMER0_CTRL) = PEC_TIMER_RESET;
  CPT32_MMIO32(PEC_TIMER0_PRESCALER) = 0u;
  CPT32_MMIO32(PEC_TIMER0_COMPARE_HI) = 0u;
  CPT32_MMIO32(PEC_TIMER0_COMPARE_LO) = period;
  CPT32_MMIO32(PEC_TIMER0_RELOAD_HI) = 0u;
  CPT32_MMIO32(PEC_TIMER0_RELOAD_LO) = period;
  CPT32_MMIO32(PEC_TIMER_ACK) = 1u;
  CPT32_MMIO32(PEC_TIMER_ENABLE) = CPT32_MMIO32(PEC_TIMER_ENABLE) | 1u;
  CPT32_MMIO32(PEC_TIMER0_CTRL) = PEC_TIMER_PERIODIC_IRQ;
}

int main(void) {
  const uint8_t *stream = (const uint8_t *)(uintptr_t)SGUB_STREAM_BASE;
  uint32_t size;
  uint32_t size_offset;
  pcg_initPCG();
  pcg_cls(63);
  pcg_colorbg(63);
  pcg_colorfg(0);
  pcg_prnlnscr("CPT32 SGUB Driver v4");
  pcg_prnlnscr("Loading SGUB stream...");

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
  else if (CPT32_MMIO8(SGUB_STREAM_BASE + 4u) == 3u)
    size_offset = 32u;
  else if (CPT32_MMIO8(SGUB_STREAM_BASE + 4u) == 4u)
    size_offset = 32u;
  else
    return 1;
  if (CPT32_MMIO8(SGUB_STREAM_BASE + 4u) == 3u && !sgub_has_3ws88pwn4()) {
    pcg_prnlnscr("ERROR: 3WS88PWN4 not found in slot 0");
    return 1;
  }
  size = ((uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset) << 24) |
         ((uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset + 1u) << 16) |
         ((uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset + 2u) << 8) |
         (uint32_t)CPT32_MMIO8(SGUB_STREAM_BASE + size_offset + 3u);
  if (size < 20u || size > SGUB_MAX_SIZE)
    return 1;

  irqc_init();
  {
    /* v4 checks its declared card IDs itself (any slot, any card). */
    sgub_status_t status = sgub_init(stream, size);
    if (status == SGUB_STATUS_ERROR_DEVICE) {
      pcg_prnlnscr("ERROR: expansion card ID mismatch");
      return 1;
    }
    if (status != SGUB_STATUS_PLAYING)
      return 1;
  }
  //sgub_visualizer_draw();
  if (sgub_state()->frame_period != 0u) {
    if (irqc_register(4u, sgub_music_timer) != 0)
      return 1;
    sgub_start_timer(sgub_state()->frame_period);
    irqc_enable(IRQ4_MASK);
  } else {
    if (irqc_register(0u, sgub_music_irq0) != 0)
      return 1;
    irqc_enable(IRQ0_MASK);
  }
  pcg_prnlnscr("Stream loaded, playing...");

  for (;;) {
    const sgub_state_t *state = sgub_state();
    if (state->status != SGUB_STATUS_PLAYING)
      break;
    cpt32_halt();
  }
  return 0;
}
