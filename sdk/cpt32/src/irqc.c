#include <cpt32/irqc.h>
#include <cpt32/mmio.h>

static irqc_handler handlers[IRQC_LINES];
static volatile uint32_t vsyncs;

void irqc_init(void) {
  unsigned line;
  CPT32_MMIO16(IRQC_ENABLE) = 0;
  CPT32_MMIO16(IRQC_PENDING) = 0xFFFFu;
  for (line = 0; line < IRQC_LINES; ++line)
    handlers[line] = 0;
  vsyncs = 0;
}
int irqc_register(unsigned line, irqc_handler handler) {
  if (line >= IRQC_LINES)
    return 1;
  handlers[line] = handler;
  return 0;
}
void irqc_enable(uint16_t mask) { CPT32_MMIO16(IRQC_ENABLE) |= mask; }
void irqc_disable(uint16_t mask) {
  CPT32_MMIO16(IRQC_ENABLE) &= (uint16_t)~mask;
}
void irqc_ack(uint16_t mask) { CPT32_MMIO16(IRQC_PENDING) = mask; }
int irqc_irq_number(void) {
  uint16_t active = CPT32_MMIO16(IRQC_PENDING) & CPT32_MMIO16(IRQC_ENABLE);
  unsigned line;
  for (line = 0; line < IRQC_LINES; ++line) {
    if (active & (1u << line))
      return (int)line;
  }
  return -1;
}
uint32_t irqc_vsync_count(void) { return vsyncs; }
void irqc_wait_vsync(void) {
  uint32_t before = vsyncs;
  while (vsyncs == before) {
  }
}
void irqc_dispatch(void) {
  int line;
  while ((line = irqc_irq_number()) >= 0) {
    irqc_handler handler = handlers[line];
    irqc_ack((uint16_t)(1u << line));
    if (line == 0)
      ++vsyncs;
    if (handler)
      handler();
  }
}
