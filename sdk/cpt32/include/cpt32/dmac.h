#ifndef CPT32_DMAC_H
#define CPT32_DMAC_H
#include <cpt32/mmio.h>
#include <stdbool.h>
#include <stdint.h>
/* DMAC MMIO and BitBlt (doc/spec_BMC_DMAC.md §2.2, §2.4). */
#define DMAC_BASE 0x80050000u
#define DMAC_ID (DMAC_BASE + 0x000u)
#define DMAC_GLOBAL_CONTROL (DMAC_BASE + 0x004u)
#define DMAC_CHANNEL_ENABLE (DMAC_BASE + 0x008u)
#define DMAC_IRQ_ENABLE (DMAC_BASE + 0x00Cu)
#define DMAC_IRQ_STATUS (DMAC_BASE + 0x010u)
#define DMAC_ABORT (DMAC_BASE + 0x014u)
#define DMAC_CAPS (DMAC_BASE + 0x018u)
#define DMAC_CAPS_BLIT 1u
#define DMAC_CHANNEL_COUNT 6u

#define DMAC_CH_SRC(n) (DMAC_BASE + 0x100u + (n) * 0x20u)
#define DMAC_CH_DST(n) (DMAC_BASE + 0x104u + (n) * 0x20u)
#define DMAC_CH_COUNT(n) (DMAC_BASE + 0x108u + (n) * 0x20u)
#define DMAC_CH_CONTROL(n) (DMAC_BASE + 0x10Cu + (n) * 0x20u)
#define DMAC_CH_STATUS(n) (DMAC_BASE + 0x110u + (n) * 0x20u)
#define DMAC_CH_REMAIN(n) (DMAC_BASE + 0x114u + (n) * 0x20u)
#define DMAC_CH_PRIORITY(n) (DMAC_BASE + 0x118u + (n) * 0x20u)
#define DMAC_CH_ROWS(n) (DMAC_BASE + 0x200u + (n) * 0x20u)
#define DMAC_CH_SRC_STRIDE(n) (DMAC_BASE + 0x204u + (n) * 0x20u)
#define DMAC_CH_DST_STRIDE(n) (DMAC_BASE + 0x208u + (n) * 0x20u)
#define DMAC_CH_KEY(n) (DMAC_BASE + 0x20Cu + (n) * 0x20u)
#define DMAC_CH_ROWS_REMAIN(n) (DMAC_BASE + 0x210u + (n) * 0x20u)

/* CHn_CONTROL fields. */
#define DMAC_WIDTH_8 0u
#define DMAC_WIDTH_16 1u
#define DMAC_WIDTH_32 2u
#define DMAC_SRC_FIXED (1u << 2)
#define DMAC_DST_FIXED (1u << 4)
#define DMAC_MODE_2D (1u << 6)
#define DMAC_KEY_EN (1u << 7)
#define DMAC_START (1u << 8)
#define DMAC_ROP_COPY (0u << 10)
#define DMAC_ROP_AND (1u << 10)
#define DMAC_ROP_OR (2u << 10)
#define DMAC_ROP_XOR (3u << 10)

/* CHn_STATUS fields. */
#define DMAC_STATUS_BUSY 1u
#define DMAC_STATUS_DONE 2u
#define DMAC_STATUS_ERROR 4u

/* Results. 1..3 are the channel's ERROR_CODE. */
#define DMAC_OK 0u
#define DMAC_E_CONFIG 1u    /* 設定不正（開始時に拒否） */
#define DMAC_E_ADDRESS 2u   /* 不許可アドレス／Bus Error */
#define DMAC_E_ABORTED 3u   /* 中止 */
#define DMAC_E_UNAVAILABLE 4u /* BitBlt非対応、channel番号不正、またはBUSY */

/* One BitBlt request. Addresses are physical Main RAM/VRAM/PCMRAM addresses.
 * row_bytes is CHn_COUNT (bytes per row); strides are signed byte offsets
 * between row starts. flags is a mask of DMAC_WIDTH_*, DMAC_SRC_FIXED,
 * DMAC_DST_FIXED, DMAC_KEY_EN and DMAC_ROP_*; dmac_blit_start adds
 * DMAC_MODE_2D and DMAC_START itself. key is used only with DMAC_KEY_EN. */
typedef struct {
  uint32_t src;
  uint32_t dst;
  uint32_t row_bytes;
  uint32_t rows;
  int32_t src_stride;
  int32_t dst_stride;
  uint32_t flags;
  uint32_t key;
} dmac_blit_t;

/* Reports whether the DMAC implements BitBlt (CAPS bit 0). */
/* DMACがBitBltに対応していればtrueを返します。 */
bool dmac_has_blit(void);
/* Starts a 2D BitBlt on channel 0..5 and returns without waiting. Enables the
 * DMAC and the channel (other channels' enable bits are kept) and clears this
 * channel's DONE/ERROR first. Returns DMAC_OK when started, DMAC_E_CONFIG when
 * the DMAC rejected the settings, or DMAC_E_UNAVAILABLE. Foundation reserves
 * channels 0 (disk) and 1 (kernel copy) and the bare-metal idc.h uses 0, so
 * use 2..5. Does not touch IRQ_ENABLE. */
/* BitBltを開始し、完了を待たずに戻ります（0=開始、1=設定不正、4=利用不可）。 */
uint32_t dmac_blit_start(uint32_t channel, const dmac_blit_t *blit);
/* Polls the channel until it is no longer BUSY. Returns DMAC_OK on DONE, or
 * the channel's ERROR_CODE (1..3). */
/* channelの転送終了を待ちます（0=成功、1〜3=ERROR_CODE）。 */
uint32_t dmac_wait(uint32_t channel);
/* Starts a BitBlt and waits for it (dmac_blit_start + dmac_wait). */
/* BitBltを開始して完了まで待ちます。戻り値はdmac_blit_startとdmac_waitと同じ。 */
uint32_t dmac_blit(uint32_t channel, const dmac_blit_t *blit);
/* Fills a rectangle of rows x row_bytes at dst (rows dst_stride apart) with
 * value, using width DMAC_WIDTH_8/16/32. dst, dst_stride and row_bytes must be
 * multiples of the width. Synchronous; the source is a per-channel word in
 * this library, so do not run two fills on the same channel at once. */
/* 矩形をvalueで塗りつぶし、完了まで待ちます。 */
uint32_t dmac_fill(uint32_t channel, uint32_t dst, int32_t dst_stride,
                   uint32_t row_bytes, uint32_t rows, uint32_t width,
                   uint32_t value);
#endif
