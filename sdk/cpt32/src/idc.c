#include <cpt32/idc.h>

void idc_enable(void) {
  CPT32_MMIO32(IDC_DMA_GLOBAL_CONTROL) = 1u;
  CPT32_MMIO32(IDC_DMA_CHANNEL_ENABLE) = 1u;
  CPT32_MMIO32(IDC_DMA_CH0_STATUS) = IDC_CLEAR_BITS;
  CPT32_MMIO32(IDC_STATUS) = IDC_CLEAR_BITS;
}

void idc_wait_clear(uint32_t address, uint32_t mask) {
  while ((CPT32_MMIO32(address) & mask) != 0u) {
  }
}

uint32_t idc_identify(uint32_t slot, uint32_t *capacity) {
  if (capacity == 0)
    return 1u;
  idc_enable();
  CPT32_MMIO32(IDC_DRIVE_SELECT) = slot;
  CPT32_MMIO32(IDC_COMMAND) = IDC_CMD_IDENTIFY;
  idc_wait_clear(IDC_STATUS, IDC_BIT_BUSY);
  if ((CPT32_MMIO32(IDC_STATUS) & IDC_BIT_ERROR) != 0u)
    return 1u;
  *capacity = CPT32_MMIO32(IDC_CAPACITY);
  return 0u;
}

uint32_t idc_error_code(void) { return CPT32_MMIO32(IDC_ERROR_CODE); }

uint32_t idc_read(uint32_t slot, uint32_t lba, uint32_t sectors, void *buffer) {
  idc_enable();
  CPT32_MMIO32(IDC_DRIVE_SELECT) = slot;
  CPT32_MMIO32(IDC_LBA) = lba;
  CPT32_MMIO32(IDC_SECTOR_COUNT) = sectors;
  CPT32_MMIO32(IDC_DMA_CH0_SRC) = IDC_DATA;
  CPT32_MMIO32(IDC_DMA_CH0_DST) = (uint32_t)(uintptr_t)buffer;
  CPT32_MMIO32(IDC_DMA_CH0_COUNT) = sectors << 9;
  CPT32_MMIO32(IDC_COMMAND) = IDC_CMD_READ;
  CPT32_MMIO32(IDC_DMA_CH0_CONTROL) = IDC_DMA_READ_CONTROL;
  idc_wait_clear(IDC_DMA_CH0_STATUS, IDC_DMA_BIT_BUSY);
  idc_wait_clear(IDC_STATUS, IDC_BIT_BUSY);
  if ((CPT32_MMIO32(IDC_DMA_CH0_STATUS) & IDC_DMA_BIT_ERROR) != 0u)
    return 1u;
  if ((CPT32_MMIO32(IDC_STATUS) & IDC_BIT_ERROR) != 0u)
    return 2u;
  return 0u;
}

uint32_t idc_write(uint32_t slot, uint32_t lba, uint32_t sectors,
                   const void *buffer) {
  idc_enable();
  CPT32_MMIO32(IDC_DRIVE_SELECT) = slot;
  CPT32_MMIO32(IDC_LBA) = lba;
  CPT32_MMIO32(IDC_SECTOR_COUNT) = sectors;
  CPT32_MMIO32(IDC_DMA_CH0_SRC) = (uint32_t)(uintptr_t)buffer;
  CPT32_MMIO32(IDC_DMA_CH0_DST) = IDC_DATA;
  CPT32_MMIO32(IDC_DMA_CH0_COUNT) = sectors << 9;
  CPT32_MMIO32(IDC_COMMAND) = IDC_CMD_WRITE;
  CPT32_MMIO32(IDC_DMA_CH0_CONTROL) = IDC_DMA_WRITE_CONTROL;
  idc_wait_clear(IDC_DMA_CH0_STATUS, IDC_DMA_BIT_BUSY);
  idc_wait_clear(IDC_STATUS, IDC_BIT_BUSY);
  if ((CPT32_MMIO32(IDC_DMA_CH0_STATUS) & IDC_DMA_BIT_ERROR) != 0u)
    return 1u;
  if ((CPT32_MMIO32(IDC_STATUS) & IDC_BIT_ERROR) != 0u)
    return 2u;
  return 0u;
}
