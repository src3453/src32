#ifndef CPT32_MMIO_H
#define CPT32_MMIO_H
#include <stdint.h>
/* CPU words and halfwords are big-endian. PCG scroll byte lanes are low first.
 */
/* Access to memory-mapped I/O registers. (8-bit) */
#define CPT32_MMIO8(address) (*(volatile uint8_t *)(uintptr_t)(address))
/* Access to memory-mapped I/O registers. (16-bit) */
#define CPT32_MMIO16(address) (*(volatile uint16_t *)(uintptr_t)(address))
/* Access to memory-mapped I/O registers. (32-bit) */
#define CPT32_MMIO32(address) (*(volatile uint32_t *)(uintptr_t)(address))
#endif
