#include <cpt32/cpu.h>
#include <cpt32/dsfs.h>
#include <cpt32/idc.h>
#include <cpt32/irqc.h>
#include <cpt32/mmio.h>
#include <cpt32/pcg.h>
#include <cpt32/wax.h>
#include <stdint.h>

#define PEC_DATA 0x80040030u
#define PEC_STATUS 0x80040034u
#define PEC_POP 0x80040038u
#define RTC_SECONDS 0x80040208u
#define VDP_ENABLE 0x80000000u
#define DIR_BUF ((uint8_t *)0x00300000u)
#define BM_BUF ((uint8_t *)0x00302000u)
#define FILE_BUF ((uint8_t *)0x00310000u)
#define EDIT_IDX ((uint32_t *)0x00320000u)
#define EDIT_TMP ((uint8_t *)0x00330000u)
#define CMD_BUF ((char *)0x00200000u)
#define WORD_BUF ((char *)0x00200100u)
#define NAME_A ((uint8_t *)0x00200200u)
#define NAME_B ((uint8_t *)0x00200210u)
#define HEX_RAW ((uint8_t *)0x00200220u)
#define HEX_TXT ((char *)0x00200230u)
#define EDIT_RAW ((uint8_t *)0x00200240u)
#define EDIT_TXT ((char *)0x00200250u)
#define TMP_TXT ((char *)0x00200260u)
#define MSG_BUF ((char *)0x00200280u)
#define CMD_CAP 120u
#define WORD_CAP 32u
#define MAX_WORDS 5u
#define MAX_FILE 65536u
#define EDIT_MAX 60000u
#define MAX_LINES 1024u
#define TEXT_ROWS 2400u
#define EVENT_QUEUE ((volatile uint32_t *)0x00340000u)
static volatile uint32_t event_head, event_tail;

static uint32_t drive, caps, shift_state, specified_drive, quit, key_code;
static uint32_t text_len, line_count, hex_len, hex_pos, hex_dirty,
    message_pending;

static int event_next(uint32_t *event);
static uint32_t waitkey(void);
static void irq_pec(void);

static void newline(void) {
  uint32_t y = pcg_cur_y + 1u;
  pcg_cur_x = 0;
  if (++pcg_vram_row >= pcg_height)
    pcg_vram_row = 0;
  if (y >= pcg_height) {
    pcg_scroll_y = (pcg_scroll_y + 8u) % (pcg_height * 8u);
    CPT32_MMIO8(0x8000f00eu) = (uint8_t)pcg_scroll_y;
    y = pcg_height - 1u;
    uint32_t row = pcg_vram_base + pcg_vram_row * pcg_width;
    for (uint32_t i = 0; i < pcg_width; ++i) {
      CPT32_MMIO8(row + i) = 0;
      CPT32_MMIO8(row + 0x1000u + i) = 0x3f;
      CPT32_MMIO8(row + 0x2000u + i) = 0;
    }
  }
  pcg_cur_y = y;
  pcg_locate(0, y);
}

static void putc_d(uint32_t ch) {
  if (ch == 13u)
    return;
  if (ch == 10u) {
    newline();
    return;
  }
  if (ch == 8u) {
    if (pcg_cur_x) {
      pcg_locate(pcg_cur_x - 1u, pcg_cur_y);
      pcg_putcscr(32u);
      pcg_locate(pcg_cur_x - 1u, pcg_cur_y);
    }
    return;
  }
  pcg_putcscr(ch);
}

static void puts_d(const char *s) {
  while (*s)
    putc_d((uint8_t)*s++);
}

static void putsln(const char *s) {
  puts_d(s);
  newline();
}

static void pad(uint32_t count) {
  while (count--)
    putc_d(32);
}

static uint32_t strlen_d(const char *s) {
  uint32_t length = 0;
  while (s[length])
    ++length;
  return length;
}

static uint32_t strcpy_d(const char *src, char *dest) {
  uint32_t length = 0;
  do {
    dest[length] = src[length];
  } while (src[length++]);
  return length - 1u;
}

static int streq(const char *a, const char *b) {
  while (*a && *a == *b) {
    ++a;
    ++b;
  }
  return *a == *b;
}

static void memcpy_d(const void *src, void *dest, uint32_t length) {
  const uint8_t *source = src;
  uint8_t *target = dest;
  while (length--)
    *target++ = *source++;
}

static void memzero(void *dest, uint32_t length) {
  uint8_t *bytes = dest;
  while (length--)
    *bytes++ = 0;
}

static void memfill(void *dest, uint32_t length, uint8_t value) {
  uint8_t *bytes = dest;
  while (length--)
    *bytes++ = value;
}

static uint32_t upper(uint32_t ch) {
  return ch >= 97u && ch <= 122u ? ch - 32u : ch;
}

static void putnum(uint32_t value) {
  if (value >= 10u)
    putnum(value / 10u);
  putc_d(48u + value % 10u);
}

static uint32_t num_text(uint32_t value, char *dest) {
  if (value >= 10u) {
    uint32_t length = num_text(value / 10u, dest);
    dest[length] = (char)('0' + value % 10u);
    return length + 1u;
  }
  dest[0] = (char)('0' + value);
  return 1u;
}

static uint32_t hexdigit(uint32_t value) {
  return value < 10u ? 48u + value : 55u + value;
}

static void puthex(uint32_t value, uint32_t digits) {
  while (digits) {
    --digits;
    putc_d(hexdigit((value >> (digits * 4u)) & 15u));
  }
}

static void cls(void) {
  pcg_scroll_y = 0;
  pcg_vram_row = 0;
  CPT32_MMIO8(0x8000f00eu) = 0;
  CPT32_MMIO8(0x8000f00fu) = 0;
  for (uint32_t i = 0; i < TEXT_ROWS; ++i) {
    CPT32_MMIO8(0x10000000u + i) = 32;
    CPT32_MMIO8(0x10001000u + i) = 0x3f;
    CPT32_MMIO8(0x10002000u + i) = 0;
  }
  pcg_locate(0, 0);
}

static void err(const char *text) {
  puts_d("ERROR: ");
  putsln(text);
}

static void more(void) {
  puts_d("  -- MORE --");
  waitkey();
  newline();
}

static uint32_t key_ascii(uint32_t usage, uint32_t shifted,
                          uint32_t caps_lock) {
  uint32_t ch = 0;
  if (usage >= 4u && usage <= 29u) {
    ch = usage + 93u;
    return (shifted ^ caps_lock) ? ch - 32u : ch;
  }
  if (usage >= 30u && usage <= 39u) {
    static const char normal[] = "1234567890";
    static const char shifted_digits[] = "!@#$%^&*()";
    return (uint8_t)(shifted ? shifted_digits[usage - 30u]
                             : normal[usage - 30u]);
  }
  switch (usage) {
  case 40:
    return 13;
  case 42:
    return 8;
  case 44:
    return 32;
  case 45:
    return shifted ? '_' : '-';
  case 46:
    return shifted ? '+' : '=';
  case 47:
    return shifted ? '{' : '[';
  case 48:
    return shifted ? '}' : ']';
  case 49:
    return shifted ? '|' : '\\';
  case 51:
    return shifted ? ':' : ';';
  case 52:
    return shifted ? '"' : '\'';
  case 53:
    return shifted ? '~' : '`';
  case 54:
    return shifted ? '<' : ',';
  case 55:
    return shifted ? '>' : '.';
  case 56:
    return shifted ? '?' : '/';
  default:
    return ch;
  }
}

static uint32_t read_line(char *dest, uint32_t capacity) {
  uint32_t count = 0;
  for (;;) {
    uint32_t event;
    if (!event_next(&event)) {
      if (CPT32_MMIO32(PEC_STATUS) & 1u) {
        irq_pec();
        continue;
      }
      cpt32_halt();
      irq_pec();
      continue;
    }
    if ((event >> 28) != 0 || ((event >> 24) & 15u) != 0)
      continue;
    uint32_t usage = (event >> 16) & 255u;
    uint32_t value = event & 1u;
    if (usage == 225u || usage == 229u) {
      shift_state = value;
      continue;
    }
    if (usage == 57u) {
      if (value)
        caps = !caps;
      continue;
    }
    if (!value)
      continue;
    uint32_t ch = key_ascii(usage, shift_state, caps);
    if (ch == 13u) {
      dest[count] = 0;
      newline();
      return count;
    }
    if (ch == 8u) {
      if (count) {
        dest[--count] = 0;
        putc_d(8);
      }
      continue;
    }
    if (ch && count < capacity - 1u) {
      dest[count++] = (char)ch;
      putc_d(ch);
    }
  }
}

static uint32_t waitkey(void) {
  uint32_t event;
  key_code = 0;
  for (;;) {
    if (!event_next(&event)) {
      if (CPT32_MMIO32(PEC_STATUS) & 1u) {
        irq_pec();
        continue;
      }
      cpt32_halt();
      irq_pec();
      continue;
    }
    if ((event >> 28) != 0 || ((event >> 24) & 15u) != 0)
      continue;
    uint32_t usage = (event >> 16) & 255u;
    uint32_t value = event & 1u;
    if (value || usage) {
      key_code = usage;
      return key_code;
    }
  }
}

static void irq_pec(void) {
  while (CPT32_MMIO32(PEC_STATUS) & 1u) {
    uint32_t event = CPT32_MMIO32(PEC_DATA);
    if (event_head - event_tail < 512u) {
      EVENT_QUEUE[event_head & 511u] = event;
      ++event_head;
    }
    CPT32_MMIO32(PEC_POP) = 1;
  }
}

static int event_next(uint32_t *event) {
  if (event_tail == event_head)
    return 0;
  *event = EVENT_QUEUE[event_tail & 511u];
  ++event_tail;
  return 1;
}

static int name_char_ok(uint32_t ch) {
  return !((ch >= 65u && ch <= 90u) || (ch >= 48u && ch <= 57u) || ch == 95u ||
           ch == 45u);
}

static int name_build(const char *src, uint32_t length, uint8_t *dest) {
  uint32_t dot = 0xffffffffu;
  uint32_t base_len = length;
  uint32_t ext_len = 0;
  if (!length)
    return 1;
  for (uint32_t i = 0; i < length; ++i) {
    if (src[i] == '.')
      dot = i;
  }
  if (dot != 0xffffffffu) {
    base_len = dot;
    ext_len = length - dot - 1u;
  }
  if (!base_len || base_len > 8u || ext_len > 3u)
    return 1;
  memfill(dest, 11, 32);
  for (uint32_t i = 0; i < base_len; ++i) {
    uint32_t ch = upper((uint8_t)src[i]);
    if (name_char_ok(ch))
      return 1;
    dest[i] = (uint8_t)ch;
  }
  if (dot != 0xffffffffu) {
    for (uint32_t i = 0; i < ext_len; ++i) {
      uint32_t ch = upper((uint8_t)src[dot + 1u + i]);
      if (name_char_ok(ch))
        return 1;
      dest[8u + i] = (uint8_t)ch;
    }
  }
  return 0;
}

static uint32_t words(const char *src, uint32_t length) {
  uint32_t i = 0;
  uint32_t word = 0;
  uint32_t chars = 0;
  uint32_t started = 0;
  memzero(WORD_BUF, MAX_WORDS * WORD_CAP);
  while (i < length) {
    uint32_t ch = (uint8_t)src[i++];
    if (ch == 32u || ch == 9u) {
      if (started) {
        WORD_BUF[word * WORD_CAP + chars] = 0;
        ++word;
        chars = 0;
        started = 0;
      }
    } else if (word < MAX_WORDS && chars < 31u) {
      WORD_BUF[word * WORD_CAP + chars++] = (char)upper(ch);
      started = 1;
    }
  }
  if (started)
    WORD_BUF[word++ * WORD_CAP + chars] = 0;
  return word;
}

static int name_from_word(const char *word, uint8_t *dest) {
  return name_build(word, strlen_d(word), dest);
}
static void name_text(const uint8_t *name, char *dest) {
  uint32_t out = 0;
  uint32_t base_len = 8;
  uint32_t ext_len = 3;
  while (base_len && name[base_len - 1u] == 32u)
    --base_len;
  while (ext_len && name[8u + ext_len - 1u] == 32u)
    --ext_len;
  for (uint32_t i = 0; i < base_len; ++i)
    dest[out++] = (char)name[i];
  if (ext_len) {
    dest[out++] = '.';
    for (uint32_t i = 0; i < ext_len; ++i)
      dest[out++] = (char)name[8u + i];
  }
  dest[out] = 0;
}

static int parse_spec(const char *src, uint32_t length) {
  uint32_t offset = 0;
  specified_drive = drive;
  if (length >= 2u && src[1] == ':') {
    if (src[0] < '0' || src[0] > '3')
      return -1;
    specified_drive = (uint32_t)(src[0] - '0');
    offset = 2;
  }
  if (name_build(src + offset, length - offset, NAME_A))
    return -1;
  return (int)(length - offset);
}

static char *tail(const char *src, uint32_t count) {
  uint32_t i = 0;
  uint32_t word = 0;
  while (word < count) {
    while (src[i] == ' ')
      ++i;
    while (src[i] && src[i] != ' ' && src[i] != '\t')
      ++i;
    ++word;
  }
  while (src[i] == ' ')
    ++i;
  return (char *)(src + i);
}

static int32_t parse_dec(const char *src) {
  uint32_t i = 0;
  uint32_t value = 0;
  if (!*src)
    return -1;
  while (src[i]) {
    if (src[i] < '0' || src[i] > '9')
      return -1;
    value = value * 10u + (uint32_t)(src[i++] - '0');
  }
  return (int32_t)value;
}

static int32_t hexval(uint32_t ch) {
  ch = upper(ch);
  if (ch >= '0' && ch <= '9')
    return (int32_t)(ch - '0');
  if (ch >= 'A' && ch <= 'F')
    return (int32_t)(ch - 'A' + 10u);
  return -1;
}

static int32_t parse_hex(const char *src) {
  uint32_t value = 0;
  uint32_t digits = 0;
  while (*src) {
    int32_t digit = hexval((uint8_t)*src++);
    if (digit < 0)
      return -1;
    value = (value << 4) + (uint32_t)digit;
    ++digits;
  }
  return digits ? (int32_t)value : -1;
}

static uint32_t now(void) { return CPT32_MMIO32(RTC_SECONDS); }

static int mount_drive(uint32_t slot) {
  return dsfs_mount(slot, DIR_BUF) == DSFS_OK ? 0 : 1;
}

static int ensure(void) {
  if (!mount_drive(drive))
    return 0;
  err("DRIVE NOT MOUNTED - FORMAT IT FIRST");
  return 1;
}

static int32_t file_size(const uint8_t *name) {
  if (dsfs_dir_load(DIR_BUF))
    return -1;
  int32_t index = dsfs_find(DIR_BUF, name);
  if (index < 0)
    return -1;
  return (int32_t)dsfs_entry_size(dsfs_entry_addr(DIR_BUF, (uint32_t)index));
}

static void cmd_dir(void) {
  if (ensure())
    return;
  if (dsfs_dir_load(DIR_BUF)) {
    err("DIRECTORY READ ERROR");
    return;
  }
  uint32_t used = dsfs_count(DIR_BUF);
  uint32_t rows = 1;
  puts_d("DRIVE ");
  putnum(drive);
  puts_d("  ");
  putnum(dsfs_total_blocks());
  puts_d(" BLOCKS  ");
  putnum(used);
  putsln(" FILE(S)");
  putsln("  #   NAME             SIZE    BLOCKS");
  for (uint32_t i = 0; i < DSFS_ENTRIES; ++i) {
    uint8_t *entry = dsfs_entry_addr(DIR_BUF, i);
    if (dsfs_entry_status(entry) != 1u)
      continue;
    puts_d("  ");
    puthex(i, 2);
    puts_d("  ");
    name_text(entry + 1, TMP_TXT);
    puts_d(TMP_TXT);
    uint32_t name_len = strlen_d(TMP_TXT);
    if (name_len < 17u)
      pad(17u - name_len);
    uint32_t size = dsfs_entry_size(entry);
    uint32_t digits = num_text(size, TMP_TXT);
    TMP_TXT[digits] = 0;
    puts_d(TMP_TXT);
    if (digits < 8u)
      pad(8u - digits);
    putnum(size ? ((size + 511u) >> 9) : 0);
    newline();
    if (++rows >= 26u) {
      rows = 0;
      more();
    }
  }
}

static void cmd_type(const uint8_t *name) {
  if (ensure())
    return;
  int32_t size = file_size(name);
  if (size < 0) {
    err("NOT FOUND");
    return;
  }
  if ((uint32_t)size > MAX_FILE) {
    err("FILE TOO LARGE");
    return;
  }
  if (dsfs_read_file(DIR_BUF, name, FILE_BUF)) {
    err("READ ERROR");
    return;
  }
  uint32_t rows = 0;
  for (uint32_t i = 0; i < (uint32_t)size; ++i) {
    uint32_t ch = FILE_BUF[i];
    if (ch == 10u) {
      newline();
      if (++rows >= 24u) {
        rows = 0;
        more();
      }
    } else {
      putc_d(ch);
    }
  }
  newline();
}

static void cmd_run(const uint8_t *name) {
  static const char *const messages[] = {
      "WAX LOAD ERROR",        "INVALID WAX HEADER",   "INVALID WAX LAYOUT",
      "INVALID WAX RANGE",     "OVERLAPPING WAX DATA", "INVALID WAX ENTRY",
      "WAX USES RESERVED RAM", "INVALID WAX BUFFER"};
  if (ensure())
    return;
  int32_t size = file_size(name);
  if (size < 0) {
    err("NOT FOUND");
    return;
  }
  if ((uint32_t)size > MAX_FILE) {
    err("FILE TOO LARGE");
    return;
  }
  if (dsfs_read_file(DIR_BUF, name, FILE_BUF)) {
    err("READ ERROR");
    return;
  }
  puts_d("STARTING ");
  name_text(name, TMP_TXT);
  putsln(TMP_TXT);
  uint32_t result = wax_load(FILE_BUF, (uint32_t)size);
  err(result < 8u ? messages[result] : messages[0]);
}

static void cmd_new(const uint8_t *name) {
  if (ensure())
    return;
  if (file_size(name) >= 0) {
    err("ALREADY EXISTS");
    return;
  }
  dsfs_set_time(now());
  if (dsfs_write_file(DIR_BUF, BM_BUF, name, FILE_BUF, 0)) {
    err("CREATE FAILED");
    return;
  }
  puts_d("CREATED ");
  name_text(name, TMP_TXT);
  putsln(TMP_TXT);
}

static void cmd_delete(const uint8_t *name) {
  if (ensure())
    return;
  if (file_size(name) < 0) {
    err("NOT FOUND");
    return;
  }
  if (dsfs_delete_file(DIR_BUF, BM_BUF, name)) {
    err("DELETE FAILED");
    return;
  }
  puts_d("DELETED ");
  name_text(name, TMP_TXT);
  putsln(TMP_TXT);
}

static void cmd_rename(const uint8_t *old_name, const uint8_t *new_name) {
  if (ensure())
    return;
  if (dsfs_dir_load(DIR_BUF)) {
    err("DIRECTORY READ ERROR");
    return;
  }
  int32_t index = dsfs_find(DIR_BUF, old_name);
  if (index < 0) {
    err("NOT FOUND");
    return;
  }
  if (dsfs_find(DIR_BUF, new_name) >= 0) {
    err("NAME ALREADY IN USE");
    return;
  }
  uint8_t *entry = dsfs_entry_addr(DIR_BUF, (uint32_t)index);
  memcpy_d(new_name, entry + 1, 11);
  if (dsfs_dir_save_sector(DIR_BUF, (uint32_t)index)) {
    err("WRITE ERROR");
    return;
  }
  puts_d("RENAMED ");
  name_text(old_name, TMP_TXT);
  puts_d(TMP_TXT);
  puts_d(" -> ");
  name_text(new_name, TMP_TXT);
  putsln(TMP_TXT);
}

static void cmd_copy(char *source_word, char *dest_word) {
  int32_t source_len = parse_spec(source_word, strlen_d(source_word));
  if (source_len < 0) {
    err("BAD SOURCE NAME");
    return;
  }
  uint32_t source_drive = specified_drive;
  memcpy_d(NAME_A, NAME_B, 11);
  int32_t dest_len = parse_spec(dest_word, strlen_d(dest_word));
  if (dest_len < 0) {
    err("BAD DEST NAME");
    return;
  }
  uint32_t dest_drive = specified_drive;
  if (mount_drive(source_drive)) {
    err("SOURCE DRIVE ERROR");
    return;
  }
  if (dsfs_dir_load(DIR_BUF)) {
    err("SOURCE DIRECTORY ERROR");
    return;
  }
  int32_t index = dsfs_find(DIR_BUF, NAME_B);
  if (index < 0) {
    err("SOURCE NOT FOUND");
    return;
  }
  uint32_t size = dsfs_entry_size(dsfs_entry_addr(DIR_BUF, (uint32_t)index));
  if (size > MAX_FILE) {
    err("FILE TOO LARGE");
    return;
  }
  if (dsfs_read_file(DIR_BUF, NAME_B, FILE_BUF)) {
    err("READ ERROR");
    return;
  }
  uint32_t tail_bytes = ((size + 511u) >> 9) * 512u - size;
  memzero(FILE_BUF + size, tail_bytes);
  if (mount_drive(dest_drive)) {
    err("DEST DRIVE ERROR");
    return;
  }
  dsfs_set_time(now());
  if (dsfs_write_file(DIR_BUF, BM_BUF, NAME_A, FILE_BUF, size)) {
    err("WRITE ERROR");
    return;
  }
  puts_d("COPIED ");
  putnum(size);
  putsln(" BYTES");
}

static void cmd_format(uint32_t slot) {
  int32_t identified = idc_identify(slot);
  if (CPT32_MMIO32(IDC_STATUS) & IDC_BIT_ERROR) {
    err("NO MEDIA IN DRIVE");
    return;
  }
  /* The SDK returns a signed value; read the raw capacity when bit 31 is set.
   */
  uint32_t total =
      identified < 0 ? CPT32_MMIO32(IDC_CAPACITY) : (uint32_t)identified;
  if (total < 19u) {
    err("DEVICE TOO SMALL FOR DSFS");
    return;
  }
  if (total > 32768u) {
    err("VOLUME TOO LARGE (16MB MAX)");
    return;
  }
  uint32_t bitmap_blocks = (total + 4095u) >> 12;
  if (bitmap_blocks != 1u) {
    err("VOLUME NEEDS A LARGER BITMAP");
    return;
  }
  uint32_t data_start = bitmap_blocks + 18u;
  memzero(DIR_BUF, 512);
  DIR_BUF[510] = 0x55;
  DIR_BUF[511] = 0xaa;
  if (idc_write(slot, 0, 1, DIR_BUF)) {
    err("WRITE ERROR (BOOT)");
    return;
  }
  memzero(DIR_BUF, 512);
  CPT32_MMIO32((uintptr_t)DIR_BUF) = 0x44534653u;
  CPT32_MMIO16((uintptr_t)DIR_BUF + 4u) = 1;
  CPT32_MMIO16((uintptr_t)DIR_BUF + 6u) = 0;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 8u) = total;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 12u) = 2;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 16u) = bitmap_blocks;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 20u) = bitmap_blocks + 2u;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 24u) = 16;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 28u) = data_start;
  CPT32_MMIO32((uintptr_t)DIR_BUF + 32u) = now();
  if (idc_write(slot, 1, 1, DIR_BUF)) {
    err("WRITE ERROR (SUPER)");
    return;
  }
  memzero(BM_BUF, 512);
  for (uint32_t i = 0; i < data_start; ++i)
    BM_BUF[i >> 3] |= (uint8_t)(0x80u >> (i & 7u));
  if (idc_write(slot, 2, bitmap_blocks, BM_BUF)) {
    err("WRITE ERROR (BITMAP)");
    return;
  }
  memzero(DIR_BUF, 8192);
  if (idc_write(slot, bitmap_blocks + 2u, 16, DIR_BUF)) {
    err("WRITE ERROR (DIR)");
    return;
  }
  puts_d("FORMATTED ");
  putnum(total);
  puts_d(" BLOCKS, DATA AT ");
  putnum(data_start);
  puts_d(" FREE ");
  putnum(total - data_start);
  putsln(" BLOCKS");
}

static void cmd_free(void) {
  if (ensure())
    return;
  if (dsfs_bitmap_load(BM_BUF)) {
    err("BITMAP READ ERROR");
    return;
  }
  uint32_t used = 0;
  for (uint32_t block = dsfs_data_start(); block < dsfs_total_blocks();
       ++block) {
    if (dsfs_bm_get(block))
      ++used;
  }
  (void)dsfs_dir_load(DIR_BUF);
  uint32_t files = dsfs_count(DIR_BUF);
  puts_d("TOTAL BLOCKS  ");
  putnum(dsfs_total_blocks());
  newline();
  puts_d("DATA BLOCKS   ");
  putnum(dsfs_total_blocks() - dsfs_data_start());
  newline();
  puts_d("USED BLOCKS   ");
  putnum(used);
  newline();
  puts_d("FREE BLOCKS   ");
  putnum(dsfs_total_blocks() - dsfs_data_start() - used);
  newline();
  puts_d("FILES         ");
  putnum(files);
  newline();
}

static void help(void) {
  putsln("DIR              list files");
  putsln("TYPE NAME        print a text file");
  putsln("RUN NAME         load and execute a WAX v2 program");
  putsln("NEW NAME         create an empty file");
  putsln("REN OLD NEW      rename a file");
  putsln("DEL NAME         delete a file");
  putsln("COPY SRC DST     copy between drives (0:NAME.TXT)");
  putsln("EDIT NAME        line editor");
  putsln("HEX NAME         hex editor");
  putsln("FORMAT [N]       format drive N as DSFS");
  putsln("DRV N            select the current drive");
  putsln("MODE 40 / MODE 80     set text width");
  putsln("FREE             block and file statistics");
  putsln("CLS / HELP / EXIT");
}

static void msg_err(const char *text) {
  strcpy_d("ERROR: ", MSG_BUF);
  strcpy_d(text, MSG_BUF + 7);
  message_pending = 1;
}
static void edit_scan(void) {
  uint32_t count = 0;
  uint32_t start = 0;
  for (uint32_t i = 0; i < text_len; ++i) {
    if (FILE_BUF[i] == 10u) {
      FILE_BUF[i] = 0;
      if (count < MAX_LINES)
        EDIT_IDX[count++] = start;
      start = i + 1u;
    }
  }
  if (start < text_len && count < MAX_LINES)
    EDIT_IDX[count++] = start;
  line_count = count;
}

static uint32_t edit_render(void) {
  uint32_t output = 0;
  for (uint32_t i = 0; i < line_count; ++i) {
    uint32_t offset = EDIT_IDX[i];
    while (FILE_BUF[offset])
      EDIT_TMP[output++] = FILE_BUF[offset++];
    EDIT_TMP[output++] = 10;
  }
  return output;
}

static void edit_splice(uint32_t index, uint32_t mode, const char *text) {
  uint32_t output = 0;
  for (uint32_t i = 0; i <= line_count; ++i) {
    if (i == index && mode != 2u) {
      uint32_t length = strlen_d(text);
      if (output + length + 1u > EDIT_MAX) {
        msg_err("EDITOR BUFFER FULL");
        return;
      }
      memcpy_d(text, EDIT_TMP + output, length);
      output += length;
      EDIT_TMP[output++] = 10;
    }
    if (i < line_count && (i != index || mode == 1u)) {
      uint32_t offset = EDIT_IDX[i];
      uint32_t length = strlen_d((char *)FILE_BUF + offset);
      if (output + length + 1u > EDIT_MAX) {
        msg_err("EDITOR BUFFER FULL");
        return;
      }
      memcpy_d(FILE_BUF + offset, EDIT_TMP + output, length);
      output += length;
      EDIT_TMP[output++] = 10;
    }
  }
  memcpy_d(EDIT_TMP, FILE_BUF, output);
  text_len = output;
  edit_scan();
}

static int edit_load(void) {
  if (dsfs_dir_load(DIR_BUF))
    return 1;
  int32_t index = dsfs_find(DIR_BUF, EDIT_RAW);
  if (index < 0) {
    text_len = 0;
    edit_scan();
    return 0;
  }
  uint32_t size = dsfs_entry_size(dsfs_entry_addr(DIR_BUF, (uint32_t)index));
  if (size > EDIT_MAX)
    return 2;
  if (dsfs_read_file(DIR_BUF, EDIT_RAW, FILE_BUF))
    return 3;
  text_len = size;
  edit_scan();
  return 0;
}

static void edit_show(void) {
  cls();
  puts_d("LINE EDITOR  ");
  puts_d(EDIT_TXT);
  puts_d("  ");
  putnum(line_count);
  putsln(" LINE(S)");
  putsln("N text=replace  I N text=insert  D N=delete  A text=append");
  putsln("L=list  W=save  Q=quit");
  if (message_pending) {
    putsln(MSG_BUF);
    message_pending = 0;
  }
  uint32_t rows = 0;
  for (uint32_t i = 0; i < line_count; ++i) {
    if (rows >= 21u) {
      rows = 0;
      more();
    }
    putnum(i + 1u);
    puts_d("  ");
    putsln((char *)FILE_BUF + EDIT_IDX[i]);
    ++rows;
  }
}

static void edit_save(void) {
  uint32_t size = edit_render();
  uint32_t sectors = (size + 511u) >> 9;
  uint32_t tail_bytes = sectors * 512u - size;
  memzero(EDIT_TMP + size, tail_bytes);
  dsfs_set_time(now());
  if (dsfs_write_file(DIR_BUF, BM_BUF, EDIT_RAW, EDIT_TMP, size)) {
    msg_err("SAVE FAILED");
    return;
  }
  uint32_t offset = strcpy_d("SAVED ", MSG_BUF);
  offset += num_text(size, MSG_BUF + offset);
  strcpy_d(" BYTES", MSG_BUF + offset);
  message_pending = 1;
}

static void edit_list(void) {
  uint32_t rows = 0;
  for (uint32_t i = 0; i < line_count; ++i) {
    if (rows >= 22u) {
      rows = 0;
      more();
    }
    putnum(i + 1u);
    puts_d("  ");
    putsln((char *)FILE_BUF + EDIT_IDX[i]);
    ++rows;
  }
}

static void edit_dispatch(uint32_t count) {
  char *command = WORD_BUF;
  char *argument = WORD_BUF + WORD_CAP;
  if (!count)
    return;
  if (streq(command, "L") || streq(command, "LIST")) {
    edit_list();
    return;
  }
  if (streq(command, "W") || streq(command, "SAVE")) {
    edit_save();
    return;
  }
  if (streq(command, "Q") || streq(command, "QUIT")) {
    quit = 1;
    return;
  }
  if (streq(command, "A") || streq(command, "APPEND")) {
    edit_splice(line_count, 1, tail(CMD_BUF, 1));
    return;
  }
  if (streq(command, "I") || streq(command, "INSERT")) {
    if (count < 2u) {
      msg_err(streq(command, "I") ? "USAGE: I N TEXT" : "USAGE: INSERT N TEXT");
      return;
    }
    int32_t line = parse_dec(argument);
    if (line < 1) {
      msg_err("BAD LINE NUMBER");
      return;
    }
    if ((uint32_t)line > line_count + 1u) {
      msg_err("LINE NUMBER TOO LARGE");
      return;
    }
    edit_splice((uint32_t)line - 1u, 1, tail(CMD_BUF, 2));
    return;
  }
  if (streq(command, "D") || streq(command, "DELETE")) {
    if (count < 2u) {
      msg_err(streq(command, "D") ? "USAGE: D N" : "USAGE: DELETE N");
      return;
    }
    int32_t line = parse_dec(argument);
    if (line < 1) {
      msg_err("BAD LINE NUMBER");
      return;
    }
    if ((uint32_t)line > line_count) {
      msg_err("LINE NUMBER TOO LARGE");
      return;
    }
    edit_splice((uint32_t)line - 1u, 2, "");
    return;
  }
  int32_t line = parse_dec(command);
  if (line >= 0) {
    if (line < 1) {
      msg_err("BAD LINE NUMBER");
      return;
    }
    if ((uint32_t)line > line_count) {
      msg_err("LINE NUMBER TOO LARGE");
      return;
    }
    edit_splice((uint32_t)line - 1u, 0, tail(CMD_BUF, 1));
    return;
  }
  msg_err("UNKNOWN EDITOR COMMAND");
}

static void cmd_edit(const uint8_t *name) {
  memcpy_d(name, EDIT_RAW, 11);
  name_text(EDIT_RAW, EDIT_TXT);
  if (ensure())
    return;
  if (edit_load()) {
    err("CANNOT LOAD FILE FOR EDITING");
    return;
  }
  quit = 0;
  while (!quit) {
    edit_show();
    puts_d(":");
    uint32_t length = read_line(CMD_BUF, CMD_CAP);
    edit_dispatch(words(CMD_BUF, length));
  }
  putsln("EDITOR CLOSED");
}

static void hex_show(void) {
  cls();
  puts_d("HEX EDITOR  ");
  puts_d(HEX_TXT);
  puts_d("  ");
  putnum(hex_len);
  puts_d(" BYTES  OFFSET ");
  puthex(hex_pos, 4);
  newline();
  putsln("N=next P=prev G addr E addr v.. W=save Q=quit");
  if (message_pending) {
    putsln(MSG_BUF);
    message_pending = 0;
  }
  puts_d("      ");
  putsln("00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F");
  for (uint32_t row = 0; row < 16u; ++row) {
    uint32_t address = hex_pos + row * 16u;
    puthex(address, 4);
    puts_d(": ");
    for (uint32_t column = 0; column < 16u; ++column) {
      uint32_t offset = address + column;
      if (offset < hex_len)
        puthex(FILE_BUF[offset], 2);
      else
        puts_d("  ");
      putc_d(32);
    }
    putc_d('|');
    for (uint32_t column = 0; column < 16u; ++column) {
      uint32_t offset = address + column;
      if (offset < hex_len) {
        uint32_t byte = FILE_BUF[offset];
        putc_d(byte >= 32u && byte <= 126u ? byte : '.');
      } else {
        putc_d(32);
      }
    }
    putsln("|");
  }
}

static void hex_save(void) {
  uint32_t sectors = (hex_len + 511u) >> 9;
  uint32_t tail_bytes = sectors * 512u - hex_len;
  memzero(FILE_BUF + hex_len, tail_bytes);
  dsfs_set_time(now());
  if (dsfs_write_file(DIR_BUF, BM_BUF, HEX_RAW, FILE_BUF, hex_len)) {
    msg_err("SAVE FAILED");
    return;
  }
  uint32_t offset = strcpy_d("SAVED ", MSG_BUF);
  offset += num_text(hex_len, MSG_BUF + offset);
  strcpy_d(" BYTES", MSG_BUF + offset);
  message_pending = 1;
  hex_dirty = 0;
}

static void hex_dispatch(uint32_t count) {
  char *command = WORD_BUF;
  char *argument = WORD_BUF + WORD_CAP;
  int32_t address;
  int32_t value;
  if (!count) {
    hex_pos += 256u;
    return;
  }
  if (streq(command, "N") || streq(command, "NEXT")) {
    hex_pos += 256u;
    return;
  }
  if (streq(command, "P") || streq(command, "PREV")) {
    hex_pos = hex_pos < 256u ? 0 : hex_pos - 256u;
    return;
  }
  if (streq(command, "Q") || streq(command, "QUIT")) {
    quit = 1;
    return;
  }
  if (streq(command, "W") || streq(command, "SAVE")) {
    hex_save();
    return;
  }
  if (streq(command, "G") || streq(command, "GOTO")) {
    if (count < 2u) {
      msg_err(streq(command, "G") ? "USAGE: G ADDRESS" : "USAGE: GOTO ADDRESS");
      return;
    }
    address = parse_hex(argument);
    if (address < 0) {
      msg_err("BAD ADDRESS");
      return;
    }
    if ((uint32_t)address > hex_len) {
      msg_err("ADDRESS OUT OF RANGE");
      return;
    }
    hex_pos = (uint32_t)address & ~255u;
    return;
  }
  if (streq(command, "E") || streq(command, "SET")) {
    if (count < 3u) {
      msg_err(streq(command, "E") ? "USAGE: E ADDRESS V1 [V2 ...]"
                                  : "USAGE: SET ADDRESS V1 [V2 ...]");
      return;
    }
    address = parse_hex(argument);
    if (address < 0) {
      msg_err("BAD ADDRESS");
      return;
    }
    for (uint32_t i = 2; i < count; ++i) {
      value = parse_hex(WORD_BUF + i * WORD_CAP);
      if (value < 0) {
        msg_err("BAD VALUE");
        return;
      }
      if (value > 255) {
        msg_err("VALUE MUST BE A BYTE");
        return;
      }
      if ((uint32_t)address >= hex_len) {
        msg_err("WRITE PAST END OF FILE");
        return;
      }
      FILE_BUF[address++] = (uint8_t)value;
      hex_dirty = 1;
    }
    return;
  }
  msg_err("UNKNOWN HEX COMMAND");
}

static void cmd_hex(const uint8_t *name) {
  memcpy_d(name, HEX_RAW, 11);
  name_text(HEX_RAW, HEX_TXT);
  if (ensure())
    return;
  int32_t size = file_size(name);
  if (size < 0) {
    err("NOT FOUND");
    return;
  }
  if (size == 0) {
    err("EMPTY FILE HAS NOTHING TO EDIT");
    return;
  }
  if ((uint32_t)size > MAX_FILE) {
    err("FILE TOO LARGE");
    return;
  }
  if (dsfs_read_file(DIR_BUF, name, FILE_BUF)) {
    err("READ ERROR");
    return;
  }
  hex_len = (uint32_t)size;
  hex_pos = 0;
  hex_dirty = 0;
  quit = 0;
  while (!quit) {
    hex_show();
    puts_d(":");
    uint32_t length = read_line(CMD_BUF, CMD_CAP);
    hex_dispatch(words(CMD_BUF, length));
  }
  if (hex_dirty)
    putsln("UNSAVED CHANGES DISCARDED (W TO SAVE)");
}

static void dispatch(uint32_t count) {
  char *command = WORD_BUF;
  char *arg1 = WORD_BUF + WORD_CAP;
  char *arg2 = WORD_BUF + 2u * WORD_CAP;
  int32_t number;
  if (!count)
    return;
  if (streq(command, "DIR") || streq(command, "LS") ||
      streq(command, "FILES")) {
    cmd_dir();
    return;
  }
  if (streq(command, "TYPE") || streq(command, "CAT")) {
    if (count < 2u) {
      err(streq(command, "TYPE") ? "USAGE: TYPE NAME" : "USAGE: CAT NAME");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD FILE NAME");
      return;
    }
    cmd_type(NAME_A);
    return;
  }
  if (streq(command, "RUN")) {
    if (count < 2u) {
      err("USAGE: RUN NAME");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD FILE NAME");
      return;
    }
    cmd_run(NAME_A);
    return;
  }
  if (streq(command, "NEW") || streq(command, "CREATE")) {
    if (count < 2u) {
      err(streq(command, "NEW") ? "USAGE: NEW NAME" : "USAGE: CREATE NAME");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD FILE NAME");
      return;
    }
    cmd_new(NAME_A);
    return;
  }
  if (streq(command, "DEL") || streq(command, "RM")) {
    if (count < 2u) {
      err(streq(command, "DEL") ? "USAGE: DEL NAME" : "USAGE: RM NAME");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD FILE NAME");
      return;
    }
    cmd_delete(NAME_A);
    return;
  }
  if (streq(command, "REN") || streq(command, "RENAME")) {
    if (count < 3u) {
      err(streq(command, "REN") ? "USAGE: REN OLD NEW"
                                : "USAGE: RENAME OLD NEW");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD OLD NAME");
      return;
    }
    memcpy_d(NAME_A, NAME_B, 11);
    if (name_from_word(arg2, NAME_A)) {
      err("BAD NEW NAME");
      return;
    }
    cmd_rename(NAME_B, NAME_A);
    return;
  }
  if (streq(command, "COPY") || streq(command, "CP")) {
    if (count < 3u) {
      err(streq(command, "COPY") ? "USAGE: COPY SRC DST" : "USAGE: CP SRC DST");
      return;
    }
    cmd_copy(arg1, arg2);
    return;
  }
  if (streq(command, "EDIT")) {
    if (count < 2u) {
      err("USAGE: EDIT NAME");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD FILE NAME");
      return;
    }
    cmd_edit(NAME_A);
    return;
  }
  if (streq(command, "HEX")) {
    if (count < 2u) {
      err("USAGE: HEX NAME");
      return;
    }
    if (name_from_word(arg1, NAME_A)) {
      err("BAD FILE NAME");
      return;
    }
    cmd_hex(NAME_A);
    return;
  }
  if (streq(command, "FORMAT")) {
    number = (int32_t)drive;
    if (count >= 2u)
      number = parse_dec(arg1);
    if (number < 0 || number >= 4) {
      err("BAD DRIVE NUMBER");
      return;
    }
    cmd_format((uint32_t)number);
    drive = (uint32_t)number;
    if (mount_drive(drive))
      err("FORMATTED BUT MOUNT FAILED");
    return;
  }
  if (streq(command, "DRV")) {
    if (count < 2u) {
      err("USAGE: DRV N");
      return;
    }
    number = parse_dec(arg1);
    if (number < 0 || number >= 4) {
      err("BAD DRIVE NUMBER");
      return;
    }
    drive = (uint32_t)number;
    if (mount_drive(drive)) {
      puts_d("DRIVE ");
      putnum(drive);
      putsln(" IS NOT MOUNTED");
    } else {
      puts_d("DRIVE ");
      putnum(drive);
      putsln(" READY");
    }
    return;
  }
  if (streq(command, "FREE") || streq(command, "STAT")) {
    cmd_free();
    return;
  }
  if (streq(command, "MODE")) {
    if (count < 2u) {
      err("USAGE: MODE 40|80");
      return;
    }
    number = parse_dec(arg1);
    if (number == 40) {
      pcg_col40();
      cls();
      putsln("40-COLUMN MODE");
      return;
    }
    if (number == 80) {
      pcg_col80();
      cls();
      putsln("80-COLUMN MODE");
      return;
    }
    err("USAGE: MODE 40|80");
    return;
  }
  if (streq(command, "HELP")) {
    help();
    return;
  }
  if (streq(command, "CLS")) {
    cls();
    return;
  }
  if (streq(command, "EXIT") || streq(command, "QUIT") ||
      streq(command, "BYE")) {
    putsln("BYE");
    for (;;)
      cpt32_halt();
  }
  err("UNKNOWN COMMAND - TYPE HELP");
}

static void banner(void) {
  putsln("CPT32 DOS v1.0 - DSFS / FDD SHELL");
  putsln("DIR TYPE RUN NEW REN DEL COPY EDIT HEX FORMAT DRV MODE FREE CLS HELP "
         "EXIT");
  newline();
}

int main(void) {
  CPT32_MMIO8(VDP_ENABLE) = 1;
  pcg_initPCG();
  pcg_col80();
  event_head = event_tail = 0;
  irqc_init();
  irqc_register(3, irq_pec);
  irqc_enable((uint16_t)(1u << 3));
  cls();
  banner();
  if (mount_drive(drive)) {
    puts_d("WARNING: DRIVE ");
    putnum(drive);
    putsln(" IS NOT MOUNTED - USE FORMAT");
  }
  for (;;) {
    putnum(drive);
    puts_d("> ");
    uint32_t length = read_line(CMD_BUF, CMD_CAP);
    dispatch(words(CMD_BUF, length));
  }
  return 0;
}
