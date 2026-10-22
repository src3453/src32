#ifndef CPT32_BMC_H
#define CPT32_BMC_H
#include <cpt32/mmio.h>
#include <stdint.h>
/* BMC MMIO (doc/spec_BMC_DMAC.md §1.4). */
#define BMC_BASE 0x80060000u
#define BMC_ID (BMC_BASE + 0x000u)
#define BMC_MAIN_RAM_SIZE (BMC_BASE + 0x080u)
#define BMC_VRAM_SIZE (BMC_BASE + 0x084u)
#define BMC_PCMRAM_SIZE (BMC_BASE + 0x088u)
/* 実装されているMain RAMのbyte数を返します。0は未報告（古いハードウェア）。 */
#define bmc_main_ram_bytes() CPT32_MMIO32(BMC_MAIN_RAM_SIZE)
/* 実装されているVRAMのbyte数を返します。0は未報告。 */
#define bmc_vram_bytes() CPT32_MMIO32(BMC_VRAM_SIZE)
/* 実装されているPCMRAMのbyte数を返します。0は未報告。 */
#define bmc_pcmram_bytes() CPT32_MMIO32(BMC_PCMRAM_SIZE)
#endif
