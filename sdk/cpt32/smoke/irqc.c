#include <cpt32/irqc.h>
#include <stdint.h>
static volatile uint32_t callbacks;
static volatile uint32_t checksum;
void smoke_vsync(void) {
  ++callbacks;
  checksum = callbacks * 37u + 19u;
}
int main(void) {
  uint32_t a = 17, b = 31;
  irqc_init();
  if (irqc_register(16, smoke_vsync) != 1)
    return 101;
  if (irqc_register(0, smoke_vsync) != 0)
    return 102;
  irqc_enable(1);
  while (irqc_vsync_count() < 3) {
    /* State must survive asynchronous callback register/stack clobbering. */
    a += b;
    a -= b;
    b += a;
    b -= a;
  }
  irqc_disable(1);
  if (a != 17 || b != 31)
    return 103;
  if (callbacks < 3 || checksum != callbacks * 37u + 19u)
    return 104;
  if (irqc_irq_number() != -1)
    return 105;
  return 42;
}
