#ifndef CPT32_MMIO_H
#define CPT32_MMIO_H
#include <stdint.h>
/* CPU words and halfwords are big-endian. PCG scroll byte lanes are low first.
 */
#define CPT32_MMIO8(address) (*(volatile uint8_t *)(uintptr_t)(address))
#define CPT32_MMIO16(address) (*(volatile uint16_t *)(uintptr_t)(address))
#define CPT32_MMIO32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#endif
