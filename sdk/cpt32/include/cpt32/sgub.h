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
  SGUB_STATUS_ERROR_STATE = -5
} sgub_status_t;

typedef struct {
  const uint8_t *stream;
  size_t size;
  size_t frame_cursor;
  uint32_t frame_count;
  uint32_t frame_index;
  uint8_t version;
  uint8_t repeat;
  sgub_status_t status;
} sgub_state_t;

/*
 * Validate the complete bounded v1/v2/v3 stream before performing any MMIO.
 * On success, execute initialization operations once and enter PLAYING.
 * The stream storage must remain readable and unchanged until playback ends.
 */
/* SGUB v1/v2/v3全体を検証し、初期化列を実行して再生を開始します。 */
sgub_status_t sgub_init(const uint8_t *stream, size_t size);

/* Run one VSync/IRQ0 frame. Returns READY while a frame was applied,
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
