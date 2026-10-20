#ifndef CPT32_IDC_H
#define CPT32_IDC_H
#include <cpt32/mmio.h>
#include <stdint.h>

#define IDC_STATUS 0x80041008u
#define IDC_DRIVE_SELECT 0x80041014u
#define IDC_COMMAND 0x80041018u
#define IDC_LBA 0x8004101Cu
#define IDC_SECTOR_COUNT 0x80041020u
#define IDC_DATA 0x80041024u
#define IDC_CAPACITY 0x80041030u
#define IDC_ERROR_CODE 0x80041038u
#define IDC_CMD_READ 1u
#define IDC_CMD_WRITE 2u
#define IDC_CMD_IDENTIFY 3u
#define IDC_BIT_BUSY 1u
#define IDC_BIT_ERROR 4u
#define IDC_DMA_GLOBAL_CONTROL 0x80050004u
#define IDC_DMA_CHANNEL_ENABLE 0x80050008u
#define IDC_DMA_CH0_SRC 0x80050100u
#define IDC_DMA_CH0_DST 0x80050104u
#define IDC_DMA_CH0_COUNT 0x80050108u
#define IDC_DMA_CH0_CONTROL 0x8005010Cu
#define IDC_DMA_CH0_STATUS 0x80050110u
#define IDC_DMA_READ_CONTROL 0x104u
#define IDC_DMA_WRITE_CONTROL 0x110u
#define IDC_DMA_BIT_BUSY 1u
#define IDC_DMA_BIT_ERROR 4u
#define IDC_CLEAR_BITS 6u

/* Enables DMAC channel 0 and clears sticky IDC/DMAC DONE and ERROR bits. */
/* DMAC ch0を有効にし、IDCとDMACの完了・エラー状態をクリアします。 */
void idc_enable(void);
/* Polls a 32-bit MMIO register until all masked bits clear. */
/* 指定MMIOレジスタのmaskに対応するビットがすべて0になるまで待ちます。 */
void idc_wait_clear(uint32_t address, uint32_t mask);
/* Returns 0 on success and stores the full uint32 sector capacity in
 * *capacity. Returns 1 on null output pointer or absent media/command error.
 * On failure, *capacity is unchanged. */
/* 指定ドライブの容量をセクタ数で取得します（0=成功、1=失敗）。 */
uint32_t idc_identify(uint32_t slot, uint32_t *capacity);
/* Raw IDC ERROR_CODE for the most recently accepted command. */
/* 直近に受理されたIDCコマンドのエラーコードを返します。 */
uint32_t idc_error_code(void);
/* Transfers 1..8,388,607 512-byte sectors. Buffer must be main RAM and
 * hold sectors*512 bytes. Results: 0 success, 1 DMAC error, 2 IDC error. */
/* 指定LBAからRAMへ同期読込みします（0=成功、1=DMA失敗、2=IDC失敗）。 */
uint32_t idc_read(uint32_t slot, uint32_t lba, uint32_t sectors, void *buffer);
/* RAMから指定LBAへ同期書込みします（0=成功、1=DMA失敗、2=IDC失敗）。 */
uint32_t idc_write(uint32_t slot, uint32_t lba, uint32_t sectors,
                   const void *buffer);
#endif
