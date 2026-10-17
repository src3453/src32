# IDC (IDE Disk Controller) driver (doc/spec_IDC.md).
#
# Raw 512-byte sector access to the drive in IDC slot 0-3, polled: no
# IRQs are used, so the caller keeps running the CPU while a transfer
# completes. All READ/WRITE transfers use DMAC channel 0 with the IDC
# DATA endpoint (0x80041024, 8-bit fixed); buffers must be in main RAM
# and hold sectors*512 bytes. idc_enable is called by every transfer
# and is idempotent, so callers do not initialize the DMAC themselves.
#
# Transfer helpers return 0 on success, 1 on a DMAC error and 2 on an
# IDC error. After an IDC error, idc_error_code returns the raw
# ERROR_CODE (1 invalid command/config, 2 no media, 3 LBA range,
# 4 write protect, 5 DMA mismatch/abort, 6 media I/O failure).

!namespace idc -
!const IDC_STATUS 0x80041008
!const IDC_DRIVE_SELECT 0x80041014
!const IDC_COMMAND 0x80041018
!const IDC_LBA 0x8004101C
!const IDC_SECTOR_COUNT 0x80041020
!const IDC_DATA 0x80041024
!const IDC_CAPACITY 0x80041030
!const IDC_ERROR_CODE 0x80041038
!const IDC_CMD_READ 1
!const IDC_CMD_WRITE 2
!const IDC_CMD_IDENTIFY 3
!const IDC_BIT_BUSY 1
!const IDC_BIT_ERROR 4
!const IDC_DMA_GLOBAL_CONTROL 0x80050004
!const IDC_DMA_CHANNEL_ENABLE 0x80050008
!const IDC_DMA_CH0_SRC 0x80050100
!const IDC_DMA_CH0_DST 0x80050104
!const IDC_DMA_CH0_COUNT 0x80050108
!const IDC_DMA_CH0_CONTROL 0x8005010C
!const IDC_DMA_CH0_STATUS 0x80050110
# CH0_CONTROL value: START plus 8-bit width and address modes.
!const IDC_DMA_READ_CONTROL 0x104   # src = DATA fixed, dst = RAM increment
!const IDC_DMA_WRITE_CONTROL 0x110  # src = RAM increment, dst = DATA fixed
!const IDC_DMA_BIT_BUSY 1
!const IDC_DMA_BIT_ERROR 4
# Sticky DONE/ERROR bits are W1C; live BUSY/DRQ bits ignore writes.
!const IDC_CLEAR_BITS 6

# Enable the DMAC and channel 0, and clear sticky DONE/ERROR on both
# devices so an earlier failure cannot be misread as this transfer's.
fn idc_enable () :
    1 IDC_DMA_GLOBAL_CONTROL st
    1 IDC_DMA_CHANNEL_ENABLE st
    6 IDC_DMA_CH0_STATUS st
    6 IDC_STATUS st
    retn
;

# Spin until the mask bits at addr are all clear (polled completion).
fn idc_wait_clear (addr mask) :
    while
        addr ld mask and 0 neq
    end
    retn
;

# IDENTIFY the selected slot: returns its capacity in sectors, or -1
# when the slot is empty or the command failed.
fn idc_identify (slot) :
    idc_enable
    slot IDC_DRIVE_SELECT st
    IDC_CMD_IDENTIFY IDC_COMMAND st
    IDC_STATUS IDC_BIT_BUSY idc_wait_clear
    IDC_STATUS ld IDC_BIT_ERROR and 0 neq if -1 ret end
    IDC_CAPACITY ld ret
;

# The raw IDC ERROR_CODE of the most recent command (0 when none).
fn idc_error_code () :
    IDC_ERROR_CODE ld ret
;

# Read `sectors` sectors starting at `lba` of `slot` into `buffer`.
fn idc_read (slot lba sectors buffer) :
    idc_enable
    slot IDC_DRIVE_SELECT st
    lba IDC_LBA st
    sectors IDC_SECTOR_COUNT st
    IDC_DATA IDC_DMA_CH0_SRC st
    buffer IDC_DMA_CH0_DST st
    sectors 9 shl IDC_DMA_CH0_COUNT st
    IDC_CMD_READ IDC_COMMAND st
    IDC_DMA_READ_CONTROL IDC_DMA_CH0_CONTROL st
    IDC_DMA_CH0_STATUS IDC_DMA_BIT_BUSY idc_wait_clear
    IDC_STATUS IDC_BIT_BUSY idc_wait_clear
    IDC_DMA_CH0_STATUS ld IDC_DMA_BIT_ERROR and 0 neq if 1 ret end
    IDC_STATUS ld IDC_BIT_ERROR and 0 neq if 2 ret end
    0 ret
;

# Write `sectors` sectors starting at `lba` of `slot` from `buffer`.
# A read-only medium fails with 2 (IDC ERROR_CODE 4, write protect).
fn idc_write (slot lba sectors buffer) :
    idc_enable
    slot IDC_DRIVE_SELECT st
    lba IDC_LBA st
    sectors IDC_SECTOR_COUNT st
    buffer IDC_DMA_CH0_SRC st
    IDC_DATA IDC_DMA_CH0_DST st
    sectors 9 shl IDC_DMA_CH0_COUNT st
    IDC_CMD_WRITE IDC_COMMAND st
    IDC_DMA_WRITE_CONTROL IDC_DMA_CH0_CONTROL st
    IDC_DMA_CH0_STATUS IDC_DMA_BIT_BUSY idc_wait_clear
    IDC_STATUS IDC_BIT_BUSY idc_wait_clear
    IDC_DMA_CH0_STATUS ld IDC_DMA_BIT_ERROR and 0 neq if 1 ret end
    IDC_STATUS ld IDC_BIT_ERROR and 0 neq if 2 ret end
    0 ret
;
!end
