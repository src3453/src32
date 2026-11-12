#include <cpt32/mmio.h>
#include <cpt32/sgub.h>

#define SGUB_MAX_SIZE UINT32_C(0x00E00000)
#define SGUB_SGU0_BASE UINT32_C(0x80020000)
#define SGUB_SGU1_BASE UINT32_C(0x80021000)
#define SGUB_PCMRAM_BASE UINT32_C(0x18000000)
#define SGUB_CARD0_BASE UINT32_C(0xA0000000)
#define SGUB_SLOT_APERTURE UINT32_C(0x02000000)
#define SGUB_SI22_BASE UINT32_C(0x80022000)
#define SGUB_V2_HEADER_SIZE 40u
#define SGUB_V4_HEADER_SIZE 48u
#define SGUB_V4_CARD_ENTRY_SIZE 32u
#define SGUB_V2_POOL_ENTRY_SIZE 16u
#define SGUB_TARGET_SI22 11u
#define SGUB_MIN_FRAME_PERIOD UINT32_C(48000)
#define SGUB_MAX_FRAME_PERIOD UINT32_C(48000000)

#define SGUB_VERSION_V1 1u
#define SGUB_VERSION_V2 2u
#define SGUB_VERSION_V3 3u
#define SGUB_VERSION_V4 4u
#define SGUB_OP_WRITE 0u
#define SGUB_OP_FILL 1u
#define SGUB_OP_COPY_POOL 2u
#define SGUB_POOL_WAVE 1u
#define SGUB_POOL_PCM 2u
#define SGUB_POOL_S3HS 3u

typedef struct {
  uint8_t kind;
  uint32_t size;
  uint32_t payload_offset;
} sgub_pool_t;

static uint8_t sgub_pool_info(uint32_t id, sgub_pool_t *out);

static sgub_state_t player;
static const sgub_output_t *output;
static uint8_t io_failed;
static uint32_t pool_count;
static uint32_t init_count;
static uint32_t directory_offset;
static uint32_t init_offset;
static uint32_t frames_offset;
static uint32_t card_count;
static uint8_t uses_si22;


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

static uint32_t sgub_read_address(const uint8_t *p) {
  return player.version >= SGUB_VERSION_V3 ? sgub_read_u32(p) : sgub_read_u24(p);
}

/* v4 card requirement entry of a slot, or NULL. */
static const uint8_t *sgub_card_entry(uint32_t slot) {
  uint32_t i;
  for (i = 0; i < card_count; ++i) {
    const uint8_t *entry = player.stream + SGUB_V4_HEADER_SIZE +
                           (size_t)i * SGUB_V4_CARD_ENTRY_SIZE;
    if (entry[0] == slot)
      return entry;
  }
  return (const uint8_t *)0;
}

static uintptr_t sgub_target_base(uint8_t target) {
  if (target == 0u)
    return SGUB_SGU0_BASE;
  if (target == 1u)
    return SGUB_SGU1_BASE;
  if (target == 2u)
    return SGUB_PCMRAM_BASE;
  if (target == SGUB_TARGET_SI22)
    return SGUB_SI22_BASE;
  return SGUB_CARD0_BASE + (uintptr_t)(target - 3u) * SGUB_SLOT_APERTURE;
}

/* SGUB v4: one fixed-size operation. Bulk ranges come from the target and,
 * for cards, the slot's bulk window. */
static uint8_t sgub_validate_v4_operation(const uint8_t *p, uint8_t opcode,
                                          uint32_t length) {
  uint8_t target = p[1];
  uint32_t address = sgub_read_u32(p + 2);
  uint32_t limit;
  uint32_t bulk_start = 0;
  uint32_t bulk_length = 0;
  if (target <= 1u) {
    limit = UINT32_C(0x8ff);
    bulk_length = UINT32_C(0x800);
  } else if (target == 2u) {
    limit = UINT32_C(0xfffff);
    bulk_length = UINT32_C(0x100000);
  } else if (target <= 10u) {
    const uint8_t *entry = sgub_card_entry(target - 3u);
    if (!entry)
      return 0;
    limit = SGUB_SLOT_APERTURE - 1u;
    bulk_start = sgub_read_u32(entry + 8);
    bulk_length = sgub_read_u32(entry + 12);
  } else if (target == SGUB_TARGET_SI22) {
    limit = UINT32_C(0x3f);
    uses_si22 = 1;
  } else {
    return 0;
  }
  if (opcode == SGUB_OP_WRITE)
    return (uint8_t)(address <= limit);
  /* Windows were checked to fit the aperture, so these cannot overflow. */
  if (length == 0 || bulk_length == 0 || address < bulk_start ||
      address - bulk_start > bulk_length ||
      length > bulk_length - (address - bulk_start))
    return 0;
  if (opcode == SGUB_OP_COPY_POOL) {
    sgub_pool_t pool;
    uint32_t source = sgub_read_u32(p + 8);
    if (!sgub_pool_info(sgub_read_u16(p + 6), &pool) || source > pool.size ||
        length > pool.size - source)
      return 0;
    if (pool.kind == SGUB_POOL_WAVE)
      return (uint8_t)(target <= 1u && (address & UINT32_C(0xff)) == 0 &&
                       (source & UINT32_C(0xff)) == 0 &&
                       (length & UINT32_C(0xff)) == 0);
    if (pool.kind == SGUB_POOL_PCM)
      return (uint8_t)(target == 2u);
    return (uint8_t)(target >= 3u && target <= 10u);
  }
  return 1;
}

static uint8_t sgub_supported(uint8_t target) {
  if (output && (target >= 32u || !(output->targets & (1u << target)))) {
    player.status = SGUB_STATUS_ERROR_TARGET;
    return 0;
  }
  return 1;
}

static uint8_t sgub_valid_record(const uint8_t *record) {
  uint8_t target = record[0];
  uint32_t address = sgub_read_u24(record + 1);
  if (!sgub_supported(target)) return 0;
  if (target <= 1u)
    return (uint8_t)(address <= UINT32_C(0x8ff));
  if (target == 2u)
    return (uint8_t)(address <= UINT32_C(0xfffff));
  if (target == 3u && player.version == SGUB_VERSION_V3)
    return (uint8_t)(address <= UINT32_C(0x1ffffff));
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
  size_t address_size = player.version >= SGUB_VERSION_V3 ? 4u : 3u;
  size_t operation_size;

  if (*cursor >= limit)
    return 0;
  p = player.stream + *cursor;
  opcode = p[0];
  if (player.version == SGUB_VERSION_V4) {
    operation_size = opcode == SGUB_OP_WRITE       ? 7u
                     : opcode == SGUB_OP_FILL      ? 11u
                     : opcode == SGUB_OP_COPY_POOL ? 16u
                                                   : 0u;
    if (operation_size == 0u || limit - *cursor < operation_size)
      return 0;
    length = opcode == SGUB_OP_WRITE ? 0u
             : opcode == SGUB_OP_FILL ? sgub_read_u32(p + 6)
                                      : sgub_read_u32(p + 12);
    if (!sgub_validate_v4_operation(p, opcode, length) ||
        !sgub_supported(p[1]))
      return 0;
    *cursor += operation_size;
    return 1;
  }
  if (opcode == SGUB_OP_WRITE) {
    operation_size = 2u + address_size + 1u;
    if (limit - *cursor < operation_size)
      return 0;
    target = p[1];
    address = sgub_read_address(p + 2);
    valid = (uint8_t)((target <= 1u && address <= UINT32_C(0x8ff)) ||
                      (target == 2u && address <= UINT32_C(0xfffff)) ||
                      (target == 3u && player.version == SGUB_VERSION_V3 && address <= UINT32_C(0x1ffffff)));
  } else if (opcode == SGUB_OP_FILL) {
    operation_size = 2u + address_size + 4u + 1u;
    if (limit - *cursor < operation_size)
      return 0;
    target = p[1];
    address = sgub_read_address(p + 2);
    length = sgub_read_u32(p + 2u + address_size);
    valid = (uint8_t)(length != 0 &&
                      ((target == 2u && address <= UINT32_C(0xfffff) &&
                        length <= UINT32_C(0x100000) - address) ||
                       (target == 3u && player.version == SGUB_VERSION_V3 && address <= UINT32_C(0x3fffff) &&
                        length <= UINT32_C(0x400000) - address) ||
                       (target <= 1u && address <= UINT32_C(0x7ff) &&
                        length <= UINT32_C(0x800) - address)));
  } else if (opcode == SGUB_OP_COPY_POOL) {
    sgub_pool_t pool;
    uint32_t pool_id;
    uint32_t source;
    operation_size = 2u + address_size + 2u + 4u + 4u;
    if (limit - *cursor < operation_size)
      return 0;
    target = p[1];
    address = sgub_read_address(p + 2);
    pool_id = sgub_read_u16(p + 2u + address_size);
    source = sgub_read_u32(p + 4u + address_size);
    length = sgub_read_u32(p + 8u + address_size);
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
                         (length & UINT32_C(0xff)) == 0) ||
                        (target == 3u && player.version == SGUB_VERSION_V3 && pool.kind == SGUB_POOL_S3HS &&
                         address <= UINT32_C(0x3fffff) && length <= UINT32_C(0x400000) - address));
    }
  } else {
    return 0;
  }
  if (!valid)
    return 0;
  if (!sgub_supported(target)) return 0;
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

  uint32_t header_size =
      player.version == SGUB_VERSION_V4 ? SGUB_V4_HEADER_SIZE : SGUB_V2_HEADER_SIZE;

  if (size < header_size || s[5] != 16u || s[6] > 1u || s[7] != 0u ||
      sgub_read_u16(s + 36) != header_size)
    return 0;
  if (player.version != SGUB_VERSION_V4) {
    if (sgub_read_u16(s + 38) != 0u)
      return 0;
  } else {
    card_count = s[38];
    player.frame_period = sgub_read_u32(s + 40);
    if (s[39] != 0u || sgub_read_u32(s + 44) != 0u || card_count > 8u ||
        (player.frame_period != 0u &&
         (player.frame_period < SGUB_MIN_FRAME_PERIOD ||
          player.frame_period > SGUB_MAX_FRAME_PERIOD)) ||
        size - header_size < card_count * SGUB_V4_CARD_ENTRY_SIZE)
      return 0;
    for (i = 0; i < card_count; ++i) {
      const uint8_t *entry =
          s + SGUB_V4_HEADER_SIZE + (size_t)i * SGUB_V4_CARD_ENTRY_SIZE;
      uint32_t id_length = entry[1];
      uint32_t id_offset = sgub_read_u32(entry + 4);
      uint32_t bulk_start = sgub_read_u32(entry + 8);
      uint32_t bulk_length = sgub_read_u32(entry + 12);
      uint32_t pad;
      if (entry[0] > 7u ||
          (i > 0 && entry[0] <= entry[-(int)SGUB_V4_CARD_ENTRY_SIZE]) ||
          id_length == 0u || id_length > 16u || entry[2] != 0u ||
          entry[3] != 0u || id_offset > SGUB_SLOT_APERTURE - id_length ||
          bulk_start > SGUB_SLOT_APERTURE ||
          bulk_length > SGUB_SLOT_APERTURE - bulk_start)
        return 0;
      for (pad = 16u + id_length; pad < SGUB_V4_CARD_ENTRY_SIZE; ++pad)
        if (entry[pad] != 0u)
          return 0;
    }
    header_size += card_count * SGUB_V4_CARD_ENTRY_SIZE;
  }
  player.frame_count = sgub_read_u32(s + 8);
  pool_count = sgub_read_u32(s + 12);
  init_count = sgub_read_u32(s + 16);
  directory_offset = sgub_read_u32(s + 20);
  init_offset = sgub_read_u32(s + 24);
  frames_offset = sgub_read_u32(s + 28);
  total = sgub_read_u32(s + 32);
  player.repeat = (uint8_t)(s[6] & 1u);
  if (player.frame_count == 0 || total != size || size > SGUB_MAX_SIZE ||
      directory_offset != header_size || pool_count > 65536u ||
      pool_count > (size - header_size) / SGUB_V2_POOL_ENTRY_SIZE)
    return 0;

  expected_payload = header_size + pool_count * SGUB_V2_POOL_ENTRY_SIZE;
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
        (kind == SGUB_POOL_S3HS &&
         (player.version < SGUB_VERSION_V3 ||
          raw_size > (player.version == SGUB_VERSION_V4 ? SGUB_SLOT_APERTURE
                                                        : UINT32_C(0x400000)))) ||
        (kind != SGUB_POOL_WAVE && kind != SGUB_POOL_PCM && kind != SGUB_POOL_S3HS))
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
  if (output) {
    if (!io_failed && output->write(output->context, target, address, &value, 1u) < 0)
      io_failed = 1;
    return;
  }
  CPT32_MMIO8(sgub_target_base(target) + address) = value;
}

static size_t sgub_apply_operation(size_t cursor) {
  const uint8_t *p = player.stream + cursor;
  uint8_t opcode = p[0];
  uint8_t target = p[1];
  uint32_t address = sgub_read_address(p + 2);
  size_t address_size = player.version >= SGUB_VERSION_V3 ? 4u : 3u;
  uintptr_t destination = sgub_target_base(target) + address;
  uint32_t i;

  if (opcode == SGUB_OP_WRITE) {
    sgub_write_target(target, address, p[2u + address_size]);
    return cursor + 2u + address_size + 1u;
  }
  if (opcode == SGUB_OP_FILL) {
    uint32_t length = sgub_read_u32(p + 2u + address_size);
    uint8_t value = p[6u + address_size];
    uint32_t packed = (uint32_t)value * UINT32_C(0x01010101);
    if (output) {
      uint8_t bytes[4096];
      for (i = 0; i < sizeof bytes; ++i) bytes[i] = value;
      for (i = 0; i < length && !io_failed;) {
        uint32_t n = length - i < sizeof bytes ? length - i : sizeof bytes;
        if (output->write(output->context, target, address + i, bytes, n) < 0)
          io_failed = 1;
        i += n;
      }
      return cursor + 2u + address_size + 4u + 1u;
    }
    /* Cards take bytes: their MMIO has no wider access contract. */
    for (i = 0; target < 3u && length - i >= 4u; i += 4u)
      CPT32_MMIO32(destination + i) = packed;
    for (; i < length; ++i)
      CPT32_MMIO8(destination + i) = value;
    return cursor + 2u + address_size + 4u + 1u;
  }
  {
    uint32_t pool_id = sgub_read_u16(p + 2u + address_size);
    uint32_t source_offset = sgub_read_u32(p + 4u + address_size);
    uint32_t length = sgub_read_u32(p + 8u + address_size);
    sgub_pool_t pool;
    uint32_t payload_offset;
    (void)sgub_pool_info(pool_id, &pool);
    payload_offset = pool.payload_offset + source_offset;
    if (output) {
      if (!io_failed && output->write(output->context, target, address,
                                      player.stream + payload_offset, length) < 0)
        io_failed = 1;
      return cursor + 2u + address_size + 2u + 4u + 4u;
    }
    for (i = 0; target < 3u && length - i >= 4u; i += 4u) {
      uint32_t word = ((uint32_t)player.stream[payload_offset + i] << 24) |
                     ((uint32_t)player.stream[payload_offset + i + 1u] << 16) |
                     ((uint32_t)player.stream[payload_offset + i + 2u] << 8) |
                     (uint32_t)player.stream[payload_offset + i + 3u];
      CPT32_MMIO32(destination + i) = word;
    }
    for (; i < length; ++i)
      CPT32_MMIO8(destination + i) = player.stream[payload_offset + i];
    return cursor + 2u + address_size + 2u + 4u + 4u;
  }
}

static void sgub_apply_record(const uint8_t *record) {
  sgub_write_target(record[0], sgub_read_u24(record + 1), record[4]);
}

static void sgub_stop_all(void) {
  uint32_t voice;
  for (voice = 0; voice < 16u; ++voice) {
    if (output && !(output->targets & (1u << (voice / 8u)))) continue;
    uint32_t address = 0x800u + (voice % 8u) * 0x20u;
    sgub_write_target((uint8_t)(voice / 8u), address + 3u, 0);
    sgub_write_target((uint8_t)(voice / 8u), address + 0x19u, 0);
  }
  /* v4: stop both 34SI22 playback channels if the stream used the chip. */
  if (uses_si22)
    sgub_write_target(SGUB_TARGET_SI22, 0x0cu, 0);
}

/* v4 MMIO playback: every declared slot must expose its ID bytes. */
static uint8_t sgub_cards_present(void) {
  uint32_t i;
  for (i = 0; i < card_count; ++i) {
    const uint8_t *entry = player.stream + SGUB_V4_HEADER_SIZE +
                           (size_t)i * SGUB_V4_CARD_ENTRY_SIZE;
    uintptr_t id = SGUB_CARD0_BASE + (uintptr_t)entry[0] * SGUB_SLOT_APERTURE +
                   sgub_read_u32(entry + 4);
    uint32_t j;
    for (j = 0; j < entry[1]; ++j)
      if (CPT32_MMIO8(id + j) != entry[16u + j])
        return 0;
  }
  return 1;
}

sgub_status_t sgub_init(const uint8_t *stream, size_t size) {
  return sgub_init_output(stream, size, (const sgub_output_t *)0);
}

sgub_status_t sgub_init_output(const uint8_t *stream, size_t size,
                               const sgub_output_t *sink) {
  uint8_t valid;
  uint8_t version;
  output = sink;
  io_failed = 0;
  if (stream == (const uint8_t *)0 || size == 0u || (output && !output->write)) {
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
  player.frame_period = 0;
  player.status = SGUB_STATUS_ERROR_FORMAT;
  card_count = 0;
  uses_si22 = 0;
  pool_count = 0;
  init_count = 0;
  directory_offset = 0;
  init_offset = 0;
  frames_offset = 0;
  if (size < 5u || stream[0] != 'S' || stream[1] != 'G' || stream[2] != 'U' ||
      stream[3] != 'B')
    return player.status;
  version = stream[4];
  if (version < SGUB_VERSION_V1 || version > SGUB_VERSION_V4) {
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
  if (!output && !sgub_cards_present()) {
    player.status = SGUB_STATUS_ERROR_DEVICE;
    return player.status;
  }

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
  player.status = io_failed ? SGUB_STATUS_ERROR_IO : SGUB_STATUS_PLAYING;
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
      player.status = io_failed ? SGUB_STATUS_ERROR_IO : SGUB_STATUS_COMPLETE;
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
  if (io_failed) {
    player.status = SGUB_STATUS_ERROR_IO;
    return player.status;
  }
  return SGUB_STATUS_READY;
}

const sgub_state_t *sgub_state(void) { return &player; }
