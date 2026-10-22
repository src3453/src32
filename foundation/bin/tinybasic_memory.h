/* Private process storage; no fixed physical guest addresses or MMIO grants. */
struct tb_storage {
  char lines[MAX_LINE + 1][LINE_STRIDE];
  Token token_data[TOK_MAX + 1];
  char input_data[256], word_data[256], string_data[4096];
  DirectoryEntry entries[DIR_MAX];
  int32_t variables[26], array_data[ARRAY_SIZE], gosub_data[512], for_data[256];
  uint32_t ast_data[0x40000 / 4];
  char pool[0x40000];
  uint8_t disk_data[0x20000];
};
static struct tb_storage *tb_memory;
#define tokens (tb_memory->token_data)
#define input (tb_memory->input_data)
#define wordbuf (tb_memory->word_data)
#define strbuf (tb_memory->string_data)
#define directory (tb_memory->entries)
#define vars (tb_memory->variables)
#define arrays (tb_memory->array_data)
#define gosub (tb_memory->gosub_data)
#define for_mem (tb_memory->for_data)
#undef LINE_BASE
#undef AST_BASE
#undef AST_LIMIT
#undef STR_POOL_BASE
#undef STR_POOL_LIMIT
#undef DISK_BASE
#define LINE_BASE ((uint32_t)(uintptr_t)tb_memory->lines)
#define AST_BASE ((uint32_t)(uintptr_t)tb_memory->ast_data)
#define AST_LIMIT (AST_BASE + sizeof tb_memory->ast_data)
#define STR_POOL_BASE ((uint32_t)(uintptr_t)tb_memory->pool)
#define STR_POOL_LIMIT (STR_POOL_BASE + sizeof tb_memory->pool)
#define DISK_BASE ((uint32_t)(uintptr_t)tb_memory->disk_data)
static uint32_t ast_bump, str_bump;
