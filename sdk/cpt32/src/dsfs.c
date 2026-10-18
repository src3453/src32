#include <cpt32/dsfs.h>
#include <cpt32/idc.h>
#include <stddef.h>
#include <string.h>

static uint32_t dsfs_state_mounted;
static uint32_t dsfs_state_slot;
static uint32_t dsfs_state_total;
static uint32_t dsfs_state_bitmap_start;
static uint32_t dsfs_state_bitmap_blocks;
static uint32_t dsfs_state_dir_start;
static uint32_t dsfs_state_data_start;
static uint32_t dsfs_state_time;
static uint8_t *dsfs_state_bitmap_buffer;

static uint32_t dsfs_read_be16(const uint8_t *p) {
  return ((uint32_t)p[0] << 8) | p[1];
}

static uint32_t dsfs_read_be32(const uint8_t *p) {
  return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
         ((uint32_t)p[2] << 8) | p[3];
}

static void dsfs_write_be32(uint8_t *p, uint32_t value) {
  p[0] = (uint8_t)(value >> 24);
  p[1] = (uint8_t)(value >> 16);
  p[2] = (uint8_t)(value >> 8);
  p[3] = (uint8_t)value;
}

static uint32_t dsfs_sector_count(uint32_t bytes) {
  return bytes / DSFS_SECTOR_SIZE + (bytes % DSFS_SECTOR_SIZE != 0u);
}

static bool dsfs_name_char(uint8_t c) {
  return (c >= (uint8_t)'A' && c <= (uint8_t)'Z') ||
         (c >= (uint8_t)'0' && c <= (uint8_t)'9') || c == (uint8_t)'_' ||
         c == (uint8_t)'-';
}

static bool dsfs_name_field_valid(const uint8_t *field, uint32_t length,
                                  bool required) {
  uint32_t i;
  bool padding = false;
  bool any = false;
  for (i = 0u; i < length; ++i) {
    uint8_t c = field[i];
    if (c == (uint8_t)' ') {
      padding = true;
    } else {
      if (padding || !dsfs_name_char(c))
        return false;
      any = true;
    }
  }
  return any || !required;
}

static bool dsfs_name_valid(const uint8_t *name) {
  return name != NULL && dsfs_name_field_valid(name, 8u, true) &&
         dsfs_name_field_valid(name + 8u, 3u, false);
}

void dsfs_set_time(uint32_t unix_seconds) { dsfs_state_time = unix_seconds; }

uint32_t dsfs_mount(uint32_t slot, void *buffer) {
  uint8_t *bytes = (uint8_t *)buffer;
  uint32_t total;
  uint32_t expected_bitmap_blocks;
  dsfs_state_mounted = 0u;
  dsfs_state_bitmap_buffer = NULL;
  dsfs_state_slot = slot;
  if (buffer == NULL)
    return DSFS_ERR_ARGS;
  if (idc_read(slot, 1u, 1u, buffer) != 0u)
    return DSFS_ERR_IO;
  if (dsfs_read_be32(bytes) != 0x44534653u)
    return DSFS_ERR_MAGIC;
  if (dsfs_read_be16(bytes + 4u) != 1u || dsfs_read_be16(bytes + 6u) != 0u)
    return DSFS_ERR_VERSION;

  total = dsfs_read_be32(bytes + 8u);
  dsfs_state_total = total;
  dsfs_state_bitmap_start = dsfs_read_be32(bytes + 12u);
  dsfs_state_bitmap_blocks = dsfs_read_be32(bytes + 16u);
  dsfs_state_dir_start = dsfs_read_be32(bytes + 20u);
  dsfs_state_data_start = dsfs_read_be32(bytes + 28u);
  expected_bitmap_blocks = total / 4096u + (total % 4096u != 0u);
  if (dsfs_state_bitmap_start != 2u ||
      dsfs_state_bitmap_blocks != expected_bitmap_blocks ||
      dsfs_state_dir_start !=
          dsfs_state_bitmap_start + dsfs_state_bitmap_blocks ||
      dsfs_read_be32(bytes + 24u) != DSFS_DIR_BLOCKS ||
      dsfs_state_data_start != dsfs_state_dir_start + DSFS_DIR_BLOCKS ||
      dsfs_state_data_start > total)
    return DSFS_ERR_LAYOUT;
  dsfs_state_mounted = 1u;
  return DSFS_OK;
}

uint32_t dsfs_dir_load(void *buffer) {
  if (dsfs_state_mounted == 0u || buffer == NULL)
    return DSFS_ERR_ARGS;
  return idc_read(dsfs_state_slot, dsfs_state_dir_start, DSFS_DIR_BLOCKS,
                  buffer) == 0u
             ? DSFS_OK
             : DSFS_ERR_IO;
}

uint32_t dsfs_dir_save_sector(void *dir_buffer, uint32_t index) {
  uint32_t sector;
  if (dsfs_state_mounted == 0u || dir_buffer == NULL || index >= DSFS_ENTRIES)
    return DSFS_ERR_ARGS;
  sector = index >> 4;
  return idc_write(dsfs_state_slot, dsfs_state_dir_start + sector, 1u,
                   (const uint8_t *)dir_buffer + sector * DSFS_SECTOR_SIZE) ==
                 0u
             ? DSFS_OK
             : DSFS_ERR_IO;
}

uint32_t dsfs_bitmap_load(void *buffer) {
  if (dsfs_state_mounted == 0u || buffer == NULL)
    return DSFS_ERR_ARGS;
  dsfs_state_bitmap_buffer = (uint8_t *)buffer;
  return idc_read(dsfs_state_slot, dsfs_state_bitmap_start,
                  dsfs_state_bitmap_blocks, buffer) == 0u
             ? DSFS_OK
             : DSFS_ERR_IO;
}

uint32_t dsfs_bitmap_save(void) {
  if (dsfs_state_mounted == 0u || dsfs_state_bitmap_buffer == NULL)
    return DSFS_ERR_ARGS;
  return idc_write(dsfs_state_slot, dsfs_state_bitmap_start,
                   dsfs_state_bitmap_blocks, dsfs_state_bitmap_buffer) == 0u
             ? DSFS_OK
             : DSFS_ERR_IO;
}

uint32_t dsfs_bm_get(uint32_t block) {
  uint8_t mask;
  if (dsfs_state_bitmap_buffer == NULL || block >= dsfs_state_total)
    return 0u;
  mask = (uint8_t)(0x80u >> (block & 7u));
  return (dsfs_state_bitmap_buffer[block >> 3] & mask) != 0u ? 1u : 0u;
}

void dsfs_bm_set(uint32_t block) {
  if (dsfs_state_bitmap_buffer == NULL || block >= dsfs_state_total)
    return;
  dsfs_state_bitmap_buffer[block >> 3] |= (uint8_t)(0x80u >> (block & 7u));
}

void dsfs_bm_clear(uint32_t block) {
  if (dsfs_state_bitmap_buffer == NULL || block >= dsfs_state_total)
    return;
  dsfs_state_bitmap_buffer[block >> 3] &= (uint8_t)~(0x80u >> (block & 7u));
}

uint8_t *dsfs_entry_addr(void *buffer, uint32_t index) {
  if (buffer == NULL || index >= DSFS_ENTRIES)
    return NULL;
  return (uint8_t *)buffer + index * DSFS_ENTRY_SIZE;
}

uint32_t dsfs_entry_status(const void *entry) {
  return entry == NULL ? 0u : ((const uint8_t *)entry)[0];
}

uint32_t dsfs_entry_start(const void *entry) {
  return entry == NULL ? 0u : dsfs_read_be32((const uint8_t *)entry + 16u);
}

uint32_t dsfs_entry_size(const void *entry) {
  return entry == NULL ? 0u : dsfs_read_be32((const uint8_t *)entry + 20u);
}

uint32_t dsfs_entry_created(const void *entry) {
  return entry == NULL ? 0u : dsfs_read_be32((const uint8_t *)entry + 24u);
}

uint32_t dsfs_entry_modified(const void *entry) {
  return entry == NULL ? 0u : dsfs_read_be32((const uint8_t *)entry + 28u);
}

bool dsfs_name_eq(const void *a, const void *b) {
  const uint8_t *left = (const uint8_t *)a;
  const uint8_t *right = (const uint8_t *)b;
  uint32_t i;
  if (a == NULL || b == NULL)
    return false;
  for (i = 0u; i < DSFS_NAME_BYTES; ++i) {
    if (left[i] != right[i])
      return false;
  }
  return true;
}

int32_t dsfs_find(const void *buffer, const void *name) {
  const uint8_t *entries = (const uint8_t *)buffer;
  uint32_t i;
  if (buffer == NULL || name == NULL)
    return -1;
  for (i = 0u; i < DSFS_ENTRIES; ++i) {
    const uint8_t *entry = entries + i * DSFS_ENTRY_SIZE;
    if (entry[0] == 1u && dsfs_name_eq(entry + 1u, name))
      return (int32_t)i;
  }
  return -1;
}

uint32_t dsfs_count(const void *buffer) {
  const uint8_t *entries = (const uint8_t *)buffer;
  uint32_t count = 0u;
  uint32_t i;
  if (buffer == NULL)
    return 0u;
  for (i = 0u; i < DSFS_ENTRIES; ++i) {
    if (entries[i * DSFS_ENTRY_SIZE] == 1u)
      ++count;
  }
  return count;
}

uint32_t dsfs_read_file(void *dir_buffer, const void *name, void *dest) {
  int32_t index;
  const uint8_t *entry;
  uint32_t size;
  uint32_t sectors;
  if (dir_buffer == NULL || name == NULL)
    return DSFS_ERR_ARGS;
  if (dsfs_dir_load(dir_buffer) != DSFS_OK)
    return DSFS_ERR_IO;
  index = dsfs_find(dir_buffer, name);
  if (index < 0)
    return DSFS_ERR_NOT_FOUND;
  entry = (const uint8_t *)dir_buffer + (uint32_t)index * DSFS_ENTRY_SIZE;
  size = dsfs_read_be32(entry + 20u);
  if (size == 0u)
    return DSFS_OK;
  if (dest == NULL)
    return DSFS_ERR_ARGS;
  sectors = dsfs_sector_count(size);
  return idc_read(dsfs_state_slot, dsfs_read_be32(entry + 16u), sectors,
                  dest) == 0u
             ? DSFS_OK
             : DSFS_ERR_IO;
}

uint32_t dsfs_write_file(void *dir_buffer, void *bitmap_buffer,
                         const void *name, void *src, uint32_t bytes) {
  uint8_t *dir = (uint8_t *)dir_buffer;
  const uint8_t *file_name = (const uint8_t *)name;
  uint32_t sectors;
  uint32_t index;
  uint8_t *entry;
  uint32_t old_start;
  uint32_t old_size;
  uint32_t old_blocks;
  uint32_t old_created;
  uint32_t new_start = 0u;
  uint32_t k;
  bool existed = false;
  bool reused = false;

  if (dsfs_state_mounted == 0u || dir_buffer == NULL || bitmap_buffer == NULL ||
      !dsfs_name_valid(file_name) || (bytes != 0u && src == NULL))
    return DSFS_ERR_ARGS;
  sectors = dsfs_sector_count(bytes);
  if (sectors >= 8388608u)
    return DSFS_ERR_ARGS;
  if (dsfs_dir_load(dir_buffer) != DSFS_OK)
    return DSFS_ERR_IO;

  {
    int32_t found = dsfs_find(dir_buffer, file_name);
    if (found >= 0) {
      index = (uint32_t)found;
      existed = true;
    } else {
      for (index = 0u; index < DSFS_ENTRIES; ++index) {
        if (dir[index * DSFS_ENTRY_SIZE] == 0u)
          break;
      }
      if (index == DSFS_ENTRIES)
        return DSFS_ERR_FULL;
    }
  }
  entry = dir + index * DSFS_ENTRY_SIZE;
  old_start = existed ? dsfs_read_be32(entry + 16u) : 0u;
  old_size = existed ? dsfs_read_be32(entry + 20u) : 0u;
  old_created = existed ? dsfs_read_be32(entry + 24u) : dsfs_state_time;
  old_blocks = dsfs_sector_count(old_size);

  if (sectors == 0u) {
    new_start = 0u;
  } else if (old_blocks != 0u && sectors <= old_blocks) {
    new_start = old_start;
    reused = true;
  } else {
    uint32_t candidate;
    if (sectors > dsfs_state_total - dsfs_state_data_start)
      return DSFS_ERR_FULL;
    if (dsfs_bitmap_load(bitmap_buffer) != DSFS_OK)
      return DSFS_ERR_IO;
    for (candidate = dsfs_state_data_start;
         candidate <= dsfs_state_total - sectors; ++candidate) {
      uint32_t j;
      for (j = 0u; j < sectors; ++j) {
        if (dsfs_bm_get(candidate + j) != 0u)
          break;
      }
      if (j == sectors)
        break;
    }
    if (candidate > dsfs_state_total - sectors)
      return DSFS_ERR_FULL;
    new_start = candidate;
    for (k = 0u; k < sectors; ++k)
      dsfs_bm_set(new_start + k);
    if (dsfs_bitmap_save() != DSFS_OK)
      return DSFS_ERR_IO;
  }

  if (bytes != 0u) {
    uint32_t padded_bytes = sectors * DSFS_SECTOR_SIZE;
    uint8_t *file = (uint8_t *)src;
    for (k = bytes; k < padded_bytes; ++k)
      file[k] = 0u;
    if (idc_write(dsfs_state_slot, new_start, sectors, src) != 0u)
      return DSFS_ERR_IO;
  }

  memset(entry, 0, DSFS_ENTRY_SIZE);
  entry[0] = 1u;
  for (k = 0u; k < DSFS_NAME_BYTES; ++k)
    entry[1u + k] = file_name[k];
  dsfs_write_be32(entry + 16u, new_start);
  dsfs_write_be32(entry + 20u, bytes);
  dsfs_write_be32(entry + 24u, existed ? old_created : dsfs_state_time);
  dsfs_write_be32(entry + 28u, dsfs_state_time);
  if (dsfs_dir_save_sector(dir_buffer, index) != DSFS_OK)
    return DSFS_ERR_IO;

  if (old_blocks != 0u && (!reused || sectors < old_blocks)) {
    if (dsfs_bitmap_load(bitmap_buffer) != DSFS_OK)
      return DSFS_ERR_IO;
    for (k = 0u; k < old_blocks; ++k) {
      uint32_t block = old_start + k;
      bool kept =
          sectors != 0u && block >= new_start && block - new_start < sectors;
      if (!kept)
        dsfs_bm_clear(block);
    }
    if (dsfs_bitmap_save() != DSFS_OK)
      return DSFS_ERR_IO;
  }
  return DSFS_OK;
}

uint32_t dsfs_delete_file(void *dir_buffer, void *bitmap_buffer,
                          const void *name) {
  uint8_t *dir = (uint8_t *)dir_buffer;
  int32_t index;
  uint8_t *entry;
  uint32_t start;
  uint32_t size;
  uint32_t blocks;
  uint32_t k;
  if (dir_buffer == NULL || bitmap_buffer == NULL || name == NULL)
    return DSFS_ERR_ARGS;
  if (dsfs_dir_load(dir_buffer) != DSFS_OK)
    return DSFS_ERR_IO;
  index = dsfs_find(dir_buffer, name);
  if (index < 0)
    return DSFS_ERR_NOT_FOUND;
  entry = dir + (uint32_t)index * DSFS_ENTRY_SIZE;
  start = dsfs_read_be32(entry + 16u);
  size = dsfs_read_be32(entry + 20u);
  blocks = dsfs_sector_count(size);
  memset(entry, 0, DSFS_ENTRY_SIZE);
  if (dsfs_dir_save_sector(dir_buffer, (uint32_t)index) != DSFS_OK)
    return DSFS_ERR_IO;
  if (blocks == 0u)
    return DSFS_OK;
  if (dsfs_bitmap_load(bitmap_buffer) != DSFS_OK)
    return DSFS_ERR_IO;
  for (k = 0u; k < blocks; ++k)
    dsfs_bm_clear(start + k);
  return dsfs_bitmap_save() == DSFS_OK ? DSFS_OK : DSFS_ERR_IO;
}

bool dsfs_is_mounted(void) { return dsfs_state_mounted != 0u; }
uint32_t dsfs_slot(void) { return dsfs_state_slot; }
uint32_t dsfs_total_blocks(void) { return dsfs_state_total; }
uint32_t dsfs_bitmap_start(void) { return dsfs_state_bitmap_start; }
uint32_t dsfs_bitmap_blocks(void) { return dsfs_state_bitmap_blocks; }
uint32_t dsfs_dir_start(void) { return dsfs_state_dir_start; }
uint32_t dsfs_data_start(void) { return dsfs_state_data_start; }
uint32_t dsfs_time(void) { return dsfs_state_time; }
uint8_t *dsfs_bitmap_buffer(void) { return dsfs_state_bitmap_buffer; }
