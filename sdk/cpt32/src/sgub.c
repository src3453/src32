#include <cpt32/mmio.h>
#include <cpt32/sgub.h>

#define SGUB_MAX_SIZE UINT32_C(0x00E00000)
#define SGUB_SGU0_BASE UINT32_C(0x80020000)
#define SGUB_SGU1_BASE UINT32_C(0x80021000)
#define SGUB_SGU0_REGS UINT32_C(0x80020800)
#define SGUB_SGU1_REGS UINT32_C(0x80021800)
#define SGUB_PCMRAM_BASE UINT32_C(0x18000000)
#define SGUB_V2_HEADER_SIZE 40u
#define SGUB_V2_POOL_ENTRY_SIZE 16u

#define SGUB_VERSION_V1 1u
#define SGUB_VERSION_V2 2u
#define SGUB_OP_WRITE 0u
#define SGUB_OP_FILL 1u
#define SGUB_OP_COPY_POOL 2u
#define SGUB_POOL_WAVE 1u
#define SGUB_POOL_PCM 2u

typedef struct {
  uint8_t kind;
  uint32_t size;
  uint32_t payload_offset;
} sgub_pool_t;

static sgub_state_t player;
static uint32_t pool_count;
static uint32_t init_count;
static uint32_t directory_offset;
static uint32_t init_offset;
static uint32_t frames_offset;

static uint16_t sgub_read_u16(const uint8_t *p) {
  return (uint16_t)(((uint16_t)p[0] << 8) | p[1]);
}

static uint32_t sgub_read_u24(const uint8_t *p) {
  return ((uint32_t)p[0] << 16) | ((uint32_t)p[1] << 8) | p[2];
}

static uint32_t sgub_read_u32(const uint8_t *p) {
  return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
         ((uint32_t)p[2] << 8) | p[3];
}

static uint8_t sgub_valid_record(const uint8_t *record) {
  uint8_t target = record[0];
  uint32_t address = sgub_read_u24(record + 1);
  if (target <= 1u)
    return (uint8_t)(address <= UINT32_C(0x8ff));
  if (target == 2u)
    return (uint8_t)(address <= UINT32_C(0xfffff));
  return 0;
}

static uint8_t sgub_pool_info(uint32_t id, sgub_pool_t *out) {
  const uint8_t *entry;
  if (id >= pool_count)
    return 0;
  entry =
      player.stream + directory_offset + (size_t)id * SGUB_V2_POOL_ENTRY_SIZE;
  out->kind = entry[2];
  out->size = sgub_read_u32(entry + 4);
  out->payload_offset = sgub_read_u32(entry + 12);
  return 1;
}

/* Parse and validate one complete operation; *cursor advances only on success.
 */
static uint8_t sgub_validate_operation(size_t *cursor, size_t limit) {
  const uint8_t *p;
  uint8_t opcode;
  uint8_t target;
  uint8_t valid = 0;
  uint32_t address;
  uint32_t length;
  size_t operation_size;

  if (*cursor >= limit)
    return 0;
  p = player.stream + *cursor;
  opcode = p[0];
  if (opcode == SGUB_OP_WRITE) {
    operation_size = 6u;
    if (limit - *cursor < operation_size)
      return 0;
    target = p[1];
    address = sgub_read_u24(p + 2);
    valid = (uint8_t)((target <= 1u && address <= UINT32_C(0x8ff)) ||
                      (target == 2u && address <= UINT32_C(0xfffff)));
  } else if (opcode == SGUB_OP_FILL) {
    operation_size = 10u;
    if (limit - *cursor < operation_size)
      return 0;
    target = p[1];
    address = sgub_read_u24(p + 2);
    length = sgub_read_u32(p + 5);
    valid = (uint8_t)(length != 0 &&
                      ((target == 2u && address <= UINT32_C(0xfffff) &&
                        length <= UINT32_C(0x100000) - address) ||
                       (target <= 1u && address <= UINT32_C(0x7ff) &&
                        length <= UINT32_C(0x800) - address)));
  } else if (opcode == SGUB_OP_COPY_POOL) {
    sgub_pool_t pool;
    uint32_t pool_id;
    uint32_t source;
    operation_size = 15u;
    if (limit - *cursor < operation_size)
      return 0;
    target = p[1];
    address = sgub_read_u24(p + 2);
    pool_id = sgub_read_u16(p + 5);
    source = sgub_read_u32(p + 7);
    length = sgub_read_u32(p + 11);
    if (length != 0 && sgub_pool_info(pool_id, &pool) && source <= pool.size &&
        length <= pool.size - source) {
      valid = (uint8_t)((target == 2u && pool.kind == SGUB_POOL_PCM &&
                         address <= UINT32_C(0xfffff) &&
                         length <= UINT32_C(0x100000) - address) ||
                        (target <= 1u && pool.kind == SGUB_POOL_WAVE &&
                         address <= UINT32_C(0x7ff) &&
                         length <= UINT32_C(0x800) - address &&
                         (address & UINT32_C(0xff)) == 0 &&
                         (source & UINT32_C(0xff)) == 0 &&
                         (length & UINT32_C(0xff)) == 0));
    }
  } else {
    return 0;
  }
  if (!valid)
    return 0;
  *cursor += operation_size;
  return 1;
}

static uint8_t sgub_validate_v1(void) {
  const uint8_t *s = player.stream;
  size_t size = player.size;
  size_t cursor;
  uint32_t frame;
  uint32_t i;

  if (size < 20u || s[5] != 16u || s[6] > 1u || s[7] != 0u)
    return 0;
  player.frame_count = sgub_read_u32(s + 8);
  init_count = sgub_read_u32(s + 12);
  if (player.frame_count == 0 || sgub_read_u32(s + 16) != size)
    return 0;
  if (size > SGUB_MAX_SIZE || size < 20u)
    return 0;
  player.repeat = (uint8_t)(s[6] & 1u);

  if ((size - 20u) / 5u < init_count)
    return 0;
  cursor = 20u + (size_t)init_count * 5u;
  for (i = 0; i < init_count; ++i) {
    if (!sgub_valid_record(s + 20u + (size_t)i * 5u))
      return 0;
  }
  for (frame = 0; frame < player.frame_count; ++frame) {
    uint16_t count;
    if (size - cursor < 2u)
      return 0;
    count = sgub_read_u16(s + cursor);
    cursor += 2u;
    if ((size - cursor) / 5u < count)
      return 0;
    for (i = 0; i < count; ++i) {
      if (!sgub_valid_record(s + cursor + (size_t)i * 5u))
        return 0;
    }
    cursor += (size_t)count * 5u;
  }
  if (cursor != size)
    return 0;
  player.frame_cursor = 20u + (size_t)init_count * 5u;
  return 1;
}

static uint8_t sgub_validate_v2(void) {
  const uint8_t *s = player.stream;
  size_t size = player.size;
  uint32_t total;
  uint32_t expected_payload;
  uint32_t i;
  size_t cursor;
  uint32_t frame;

  if (size < SGUB_V2_HEADER_SIZE || s[5] != 16u || s[6] > 1u || s[7] != 0u ||
      sgub_read_u16(s + 36) != SGUB_V2_HEADER_SIZE ||
      sgub_read_u16(s + 38) != 0u)
    return 0;
  player.frame_count = sgub_read_u32(s + 8);
  pool_count = sgub_read_u32(s + 12);
  init_count = sgub_read_u32(s + 16);
  directory_offset = sgub_read_u32(s + 20);
  init_offset = sgub_read_u32(s + 24);
  frames_offset = sgub_read_u32(s + 28);
  total = sgub_read_u32(s + 32);
  player.repeat = (uint8_t)(s[6] & 1u);
  if (player.frame_count == 0 || total != size || size > SGUB_MAX_SIZE ||
      directory_offset != SGUB_V2_HEADER_SIZE || pool_count > 65536u ||
      pool_count > (size - SGUB_V2_HEADER_SIZE) / SGUB_V2_POOL_ENTRY_SIZE)
    return 0;

  expected_payload = SGUB_V2_HEADER_SIZE + pool_count * SGUB_V2_POOL_ENTRY_SIZE;
  if (expected_payload > total)
    return 0;
  for (i = 0; i < pool_count; ++i) {
    const uint8_t *entry =
        s + directory_offset + (size_t)i * SGUB_V2_POOL_ENTRY_SIZE;
    uint32_t raw_size = sgub_read_u32(entry + 4);
    uint32_t stored_size = sgub_read_u32(entry + 8);
    uint32_t payload_offset = sgub_read_u32(entry + 12);
    uint8_t kind = entry[2];
    if (sgub_read_u16(entry) != i || entry[3] != 0u ||
        raw_size != stored_size || raw_size == 0 ||
        payload_offset != expected_payload || payload_offset > total ||
        stored_size > total - payload_offset)
      return 0;
    if ((kind == SGUB_POOL_WAVE && raw_size != 256u) ||
        (kind == SGUB_POOL_PCM && raw_size > UINT32_C(0x100000)) ||
        (kind != SGUB_POOL_WAVE && kind != SGUB_POOL_PCM))
      return 0;
    expected_payload += stored_size;
  }
  if (init_offset != expected_payload || frames_offset < init_offset ||
      frames_offset > total ||
      init_count > (frames_offset - init_offset) / 6u ||
      player.frame_count > (total - frames_offset) / 2u)
    return 0;

  cursor = init_offset;
  for (i = 0; i < init_count; ++i) {
    if (!sgub_validate_operation(&cursor, frames_offset))
      return 0;
  }
  if (cursor != frames_offset)
    return 0;

  for (frame = 0; frame < player.frame_count; ++frame) {
    uint16_t count;
    uint32_t op;
    if (cursor > total || total - cursor < 2u)
      return 0;
    count = sgub_read_u16(s + cursor);
    cursor += 2u;
    for (op = 0; op < count; ++op) {
      if (!sgub_validate_operation(&cursor, total))
        return 0;
    }
  }
  if (cursor != total)
    return 0;
  player.frame_cursor = frames_offset;
  return 1;
}

static void sgub_write_target(uint8_t target, uint32_t address, uint8_t value) {
  uintptr_t base = target == 0u   ? SGUB_SGU0_BASE
                   : target == 1u ? SGUB_SGU1_BASE
                                  : SGUB_PCMRAM_BASE;
  CPT32_MMIO8(base + address) = value;
}

static size_t sgub_apply_operation(size_t cursor) {
  const uint8_t *p = player.stream + cursor;
  uint8_t opcode = p[0];
  uint8_t target = p[1];
  uint32_t address = sgub_read_u24(p + 2);
  uintptr_t base = target == 0u   ? SGUB_SGU0_BASE
                   : target == 1u ? SGUB_SGU1_BASE
                                  : SGUB_PCMRAM_BASE;
  uintptr_t destination = base + address;
  uint32_t i;

  if (opcode == SGUB_OP_WRITE) {
    sgub_write_target(target, address, p[5]);
    return cursor + 6u;
  }
  if (opcode == SGUB_OP_FILL) {
    uint32_t length = sgub_read_u32(p + 5);
    uint8_t value = p[9];
    uint32_t packed = (uint32_t)value * UINT32_C(0x01010101);
    for (i = 0; length - i >= 4u; i += 4u)
      CPT32_MMIO32(destination + i) = packed;
    for (; i < length; ++i)
      CPT32_MMIO8(destination + i) = value;
    return cursor + 10u;
  }
  {
    uint32_t pool_id = sgub_read_u16(p + 5);
    uint32_t source_offset = sgub_read_u32(p + 7);
    uint32_t length = sgub_read_u32(p + 11);
    sgub_pool_t pool;
    uint32_t payload_offset;
    (void)sgub_pool_info(pool_id, &pool);
    payload_offset = pool.payload_offset + source_offset;
    for (i = 0; length - i >= 4u; i += 4u) {
      uint32_t word = ((uint32_t)player.stream[payload_offset + i] << 24) |
                     ((uint32_t)player.stream[payload_offset + i + 1u] << 16) |
                     ((uint32_t)player.stream[payload_offset + i + 2u] << 8) |
                     (uint32_t)player.stream[payload_offset + i + 3u];
      CPT32_MMIO32(destination + i) = word;
    }
    for (; i < length; ++i)
      CPT32_MMIO8(destination + i) = player.stream[payload_offset + i];
    return cursor + 15u;
  }
}

static void sgub_apply_record(const uint8_t *record) {
  sgub_write_target(record[0], sgub_read_u24(record + 1), record[4]);
}

static void sgub_stop_all(void) {
  uint32_t voice;
  for (voice = 0; voice < 16u; ++voice) {
    uintptr_t base = voice < 8u ? SGUB_SGU0_REGS + (voice % 8u) * 0x20u
                                : SGUB_SGU1_REGS + (voice % 8u) * 0x20u;
    CPT32_MMIO8(base + 3u) = 0;
    CPT32_MMIO8(base + 0x19u) = 0;
  }
}

sgub_status_t sgub_init(const uint8_t *stream, size_t size) {
  uint8_t valid;
  uint8_t version;
  if (stream == (const uint8_t *)0 || size == 0u) {
    player.status = SGUB_STATUS_ERROR_ARGUMENT;
    return player.status;
  }
  player.stream = stream;
  player.size = size;
  player.frame_cursor = 0;
  player.frame_count = 0;
  player.frame_index = 0;
  player.version = 0;
  player.repeat = 0;
  player.status = SGUB_STATUS_ERROR_FORMAT;
  pool_count = 0;
  init_count = 0;
  directory_offset = 0;
  init_offset = 0;
  frames_offset = 0;
  if (size < 5u || stream[0] != 'S' || stream[1] != 'G' || stream[2] != 'U' ||
      stream[3] != 'B')
    return player.status;
  version = stream[4];
  if (version != SGUB_VERSION_V1 && version != SGUB_VERSION_V2) {
    player.status = SGUB_STATUS_ERROR_VERSION;
    return player.status;
  }
  if (size > SGUB_MAX_SIZE) {
    player.status = SGUB_STATUS_ERROR_BOUNDS;
    return player.status;
  }
  player.version = version;
  valid = version == SGUB_VERSION_V1 ? sgub_validate_v1() : sgub_validate_v2();
  if (!valid)
    return player.status;

  if (version == SGUB_VERSION_V1) {
    size_t cursor = 20u;
    uint32_t i;
    for (i = 0; i < init_count; ++i) {
      sgub_apply_record(stream + cursor);
      cursor += 5u;
    }
  } else {
    size_t cursor = init_offset;
    uint32_t i;
    for (i = 0; i < init_count; ++i)
      cursor = sgub_apply_operation(cursor);
    player.frame_cursor = frames_offset;
  }
  player.status = SGUB_STATUS_PLAYING;
  return player.status;
}

sgub_status_t sgub_step(void) {
  size_t cursor;
  uint16_t count;
  uint32_t i;
  if (player.status == SGUB_STATUS_COMPLETE)
    return SGUB_STATUS_COMPLETE;
  if (player.status != SGUB_STATUS_PLAYING)
    return SGUB_STATUS_ERROR_STATE;

  if (player.frame_index >= player.frame_count) {
    if (player.repeat) {
      player.frame_index = 0;
      player.frame_cursor = player.version == SGUB_VERSION_V1
                                ? 20u + (size_t)init_count * 5u
                                : frames_offset;
    } else {
      sgub_stop_all();
      player.status = SGUB_STATUS_COMPLETE;
      return player.status;
    }
  }

  cursor = player.frame_cursor;
  count = sgub_read_u16(player.stream + cursor);
  cursor += 2u;
  for (i = 0; i < count; ++i) {
    if (player.version == SGUB_VERSION_V1) {
      sgub_apply_record(player.stream + cursor);
      cursor += 5u;
    } else {
      cursor = sgub_apply_operation(cursor);
    }
  }
  player.frame_cursor = cursor;
  ++player.frame_index;
  return SGUB_STATUS_READY;
}

const sgub_state_t *sgub_state(void) { return &player; }
