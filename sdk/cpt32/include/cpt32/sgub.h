#ifndef CPT32_SGUB_H
#define CPT32_SGUB_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
  SGUB_STATUS_READY = 0,
  SGUB_STATUS_PLAYING = 1,
  SGUB_STATUS_COMPLETE = 2,
  SGUB_STATUS_ERROR_ARGUMENT = -1,
  SGUB_STATUS_ERROR_FORMAT = -2,
  SGUB_STATUS_ERROR_VERSION = -3,
  SGUB_STATUS_ERROR_BOUNDS = -4,
  SGUB_STATUS_ERROR_STATE = -5,
  SGUB_STATUS_ERROR_TARGET = -6,
  SGUB_STATUS_ERROR_IO = -7,
  /* v4: a declared expansion card ID did not match (MMIO backend only). */
  SGUB_STATUS_ERROR_DEVICE = -8
} sgub_status_t;

typedef struct {
  const uint8_t *stream;
  size_t size;
  size_t frame_cursor;
  uint32_t frame_count;
  uint32_t frame_index;
  /* v4 frame length in 48 MHz cycles; 0 = one frame per VSync (v1..v3). */
  uint32_t frame_period;
  uint8_t version;
  uint8_t repeat;
  sgub_status_t status;
} sgub_state_t;

/* Optional stream output instead of direct MMIO. targets is a bit mask of
 * accepted target IDs (v4: 0..1 SGU, 2 PCM RAM, 3..10 slots, 11 34SI22).
 * Card ID checks are the output's responsibility. write returns zero on success, negative on failure;
 * it must preserve byte order and duplicate writes, and consume all bytes.
 * The output and context must remain valid until playback ends. */
typedef struct {
  void *context;
  uint32_t targets;
  int32_t (*write)(void *context, uint8_t target, uint32_t address,
                   const uint8_t *data, uint32_t size);
} sgub_output_t;

/* NULL output selects the original MMIO backend. Unsupported targets anywhere
 * in the stream are rejected before the first output callback. */
sgub_status_t sgub_init_output(const uint8_t *stream, size_t size,
                               const sgub_output_t *output);

/*
 * Validate the complete bounded v1..v4 stream before performing any MMIO,
 * check v4 card IDs, then execute initialization operations once and enter
 * PLAYING. The stream storage must remain readable and unchanged until
 * playback ends. The caller drives sgub_step at state->frame_period.
 */
/* SGUB v1〜v4全体を検証し、v4のカードIDを確認して初期化列を実行します。 */
sgub_status_t sgub_init(const uint8_t *stream, size_t size);

/* Run one frame (VSync, or one v4 frame_period). Returns READY while a frame was applied,
 * COMPLETE after the non-repeating final frame has been silenced, or a
 * negative status if no valid player is initialized. */
/* 再生を1フレーム進め、非リピート終端では次の呼出しで全voiceを停止します。 */
sgub_status_t sgub_step(void);

/* Return the current player state; its stream pointer is caller-owned. */
/* 現在のプレイヤー状態への読取り専用ポインタを返します。 */
const sgub_state_t *sgub_state(void);

#ifdef __cplusplus
}
#endif

#endif
