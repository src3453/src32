#include <cpt32/dmac.h>

/* Fill sources must be in DMA-readable RAM; one word per channel. */
static volatile uint32_t fill_words[DMAC_CHANNEL_COUNT];

bool dmac_has_blit(void) {
  return (CPT32_MMIO32(DMAC_CAPS) & DMAC_CAPS_BLIT) != 0u;
}

uint32_t dmac_blit_start(uint32_t channel, const dmac_blit_t *blit) {
  if (channel >= DMAC_CHANNEL_COUNT || blit == 0 || !dmac_has_blit())
    return DMAC_E_UNAVAILABLE;
  if ((CPT32_MMIO32(DMAC_CH_STATUS(channel)) & DMAC_STATUS_BUSY) != 0u)
    return DMAC_E_UNAVAILABLE;
  CPT32_MMIO32(DMAC_GLOBAL_CONTROL) = 1u;
  CPT32_MMIO32(DMAC_CHANNEL_ENABLE) |= 1u << channel;
  CPT32_MMIO32(DMAC_CH_STATUS(channel)) = DMAC_STATUS_DONE | DMAC_STATUS_ERROR;
  CPT32_MMIO32(DMAC_CH_SRC(channel)) = blit->src;
  CPT32_MMIO32(DMAC_CH_DST(channel)) = blit->dst;
  CPT32_MMIO32(DMAC_CH_COUNT(channel)) = blit->row_bytes;
  CPT32_MMIO32(DMAC_CH_ROWS(channel)) = blit->rows;
  CPT32_MMIO32(DMAC_CH_SRC_STRIDE(channel)) = (uint32_t)blit->src_stride;
  CPT32_MMIO32(DMAC_CH_DST_STRIDE(channel)) = (uint32_t)blit->dst_stride;
  CPT32_MMIO32(DMAC_CH_KEY(channel)) = blit->key;
  CPT32_MMIO32(DMAC_CH_CONTROL(channel)) =
      (blit->flags & ~DMAC_START) | DMAC_MODE_2D | DMAC_START;
  /* A rejected start sets ERROR without BUSY before this read. */
  if ((CPT32_MMIO32(DMAC_CH_STATUS(channel)) & DMAC_STATUS_ERROR) != 0u)
    return (CPT32_MMIO32(DMAC_CH_STATUS(channel)) >> 4) & 0xFu;
  return DMAC_OK;
}

uint32_t dmac_wait(uint32_t channel) {
  uint32_t status;
  if (channel >= DMAC_CHANNEL_COUNT)
    return DMAC_E_UNAVAILABLE;
  do {
    status = CPT32_MMIO32(DMAC_CH_STATUS(channel));
  } while ((status & DMAC_STATUS_BUSY) != 0u);
  if ((status & DMAC_STATUS_ERROR) != 0u)
    return (status >> 4) & 0xFu;
  return DMAC_OK;
}

uint32_t dmac_blit(uint32_t channel, const dmac_blit_t *blit) {
  uint32_t result = dmac_blit_start(channel, blit);
  return result != DMAC_OK ? result : dmac_wait(channel);
}

uint32_t dmac_fill(uint32_t channel, uint32_t dst, int32_t dst_stride,
                   uint32_t row_bytes, uint32_t rows, uint32_t width,
                   uint32_t value) {
  dmac_blit_t blit;
  uint32_t shift;
  if (channel >= DMAC_CHANNEL_COUNT || width > DMAC_WIDTH_32)
    return DMAC_E_UNAVAILABLE;
  /* Big-endian: the first (1 << width) bytes of the word hold the value. */
  shift = 32u - (8u << width);
  fill_words[channel] = shift == 0u ? value : value << shift;
  blit.src = (uint32_t)(uintptr_t)&fill_words[channel];
  blit.dst = dst;
  blit.row_bytes = row_bytes;
  blit.rows = rows;
  blit.src_stride = 0;
  blit.dst_stride = dst_stride;
  blit.flags = width | DMAC_SRC_FIXED;
  blit.key = 0u;
  return dmac_blit(channel, &blit);
}
