#include <cpt32/cpu.h>
#include <cpt32/irqc.h>
#include <cpt32/mmio.h>
#include <cpt32/pcg.h>
#include <cpt32/pec_rng.h>
#include <stdint.h>

/* Freestanding translation of tinybasic.sol. Large arrays stay at the guest
 * addresses and sizes used by that program; there is no host runtime. */
#define PEC_DATA 0x80040030u
#define PEC_STATUS 0x80040034u
#define PEC_POP 0x80040038u
#define IDC 0x80041000u
#define DMA 0x80050000u
#define DMA_CH0 0x80050100u
#define LINE_BASE 0x00200000u
#define LINE_STRIDE 128u
#define ARRAY_BASE 0x00180000u
#define VAR_BASE 0x00170000u
#define GOSUB_BASE 0x00190000u
#define FOR_BASE 0x00191000u
#define INPUT_BASE 0x001B0000u
#define WORD_BASE 0x001B0100u
#define TOK_BUF 0x001B1000u
#define STR_BUF 0x001B2000u
#define DIR_BASE 0x001C0000u
#define DIR_MAX 1024
#define AST_BASE 0x00500000u
#define AST_LIMIT 0x00540000u
#define STR_POOL_BASE 0x00540000u
#define STR_POOL_LIMIT 0x00580000u
#define DISK_BASE 0x00400000u
#define MAX_LINE 9999
#define MAX_TEXT 120
#define ARRAY_SIZE 1000
#define TOK_MAX 96

enum {
  T_EOF = 0,
  T_NUM = 1,
  T_STR = 2,
  T_IDENT = 3,
  T_AND = 256,
  T_OR,
  T_NOT,
  T_MOD,
  T_ABS,
  T_ASC,
  T_RND,
  T_SGN,
  T_LE,
  T_GE,
  T_NE,
  T_LET,
  T_PRINT,
  T_INPUT,
  T_IF,
  T_THEN,
  T_GOTO,
  T_GOSUB,
  T_RETURN,
  T_FOR,
  T_TO,
  T_NEXT,
  T_RUN,
  T_LIST,
  T_NEW,
  T_CLEAR,
  T_CLS,
  T_END,
  T_STOP,
  T_REM,
  T_HELP,
  T_QUIT,
  T_BYE,
  T_LOAD,
  T_SAVE,
  T_TRON,
  T_TROFF
};
enum { N_NUM = 1, N_STR, N_VAR, N_ARR, N_BINOP, N_UNOP, N_CALL };
enum {
  S_PRINT = 11,
  PI_STR = 12,
  PI_EXPR = 13,
  PI_SEP = 14,
  S_LET = 16,
  S_ARRLET,
  S_INPUT,
  S_GOTO,
  S_GOSUB,
  S_RETURN,
  S_IF,
  S_FOR,
  S_NEXT,
  S_SYS,
  PI_WIDTH
};
enum {
  OP_ADD = 1,
  OP_SUB,
  OP_MUL,
  OP_DIV,
  OP_IDIV,
  OP_MOD,
  OP_POW,
  OP_EQ,
  OP_LT,
  OP_GT,
  OP_LE,
  OP_GE,
  OP_NE,
  OP_AND,
  OP_OR
};
enum { U_NEG = 1, U_NOT };
enum { F_ABS = 1, F_SGN, F_ASC, F_RND };
enum {
  SYS_RUN = 1,
  SYS_LIST,
  SYS_NEW,
  SYS_CLEAR,
  SYS_CLS,
  SYS_HELP,
  SYS_LOAD,
  SYS_SAVE,
  SYS_TRON,
  SYS_TROFF,
  SYS_END,
  SYS_BYE
};
typedef struct {
  int32_t type, value;
} Token;
typedef struct Node {
  int32_t kind, value;
  struct Node *a, *b, *c;
} Node;
typedef struct {
  int32_t line;
  Node *root;
} DirectoryEntry;
static Token *const tokens = (Token *)(uintptr_t)TOK_BUF;
static char *const input = (char *)(uintptr_t)INPUT_BASE;
static char *const wordbuf = (char *)(uintptr_t)WORD_BASE;
static char *const strbuf = (char *)(uintptr_t)STR_BUF;
static DirectoryEntry *const directory = (DirectoryEntry *)(uintptr_t)DIR_BASE;
static int32_t *const vars = (int32_t *)(uintptr_t)VAR_BASE;
static int32_t *const arrays = (int32_t *)(uintptr_t)ARRAY_BASE;
static int32_t *const gosub = (int32_t *)(uintptr_t)GOSUB_BASE;
static uint32_t ast_bump = AST_BASE, str_bump = STR_POOL_BASE;
static uint32_t token_count, str_used, parser_pos, disk_pos;
static int32_t disk_limit;
static int32_t line_count, pc, line_no, error, stop, jump, trace;
static int32_t gosub_sp, for_sp, shift_down, caps_lock;
static int32_t *const for_mem = (int32_t *)(uintptr_t)FOR_BASE;
static Node *current;
static volatile uint32_t event_ring[512];
static volatile uint32_t event_head, event_tail;
static inline uint32_t u32(int32_t x) { return (uint32_t)x; }
static inline int32_t si(uint32_t x) { return (int32_t)x; }
static inline int32_t add_i(int32_t a, int32_t b) {
  return si(u32(a) + u32(b));
}
static inline int32_t sub_i(int32_t a, int32_t b) {
  return si(u32(a) - u32(b));
}
static inline int32_t mul_i(int32_t a, int32_t b) {
  return si(u32(a) * u32(b));
}
static inline uint8_t *bytes_at(uint32_t p) { return (uint8_t *)(uintptr_t)p; }
static inline char *line_ptr(int32_t n) {
  return (char *)(uintptr_t)(LINE_BASE + (uint32_t)n * LINE_STRIDE);
}
static void print_string(const char *s);
static void print_number(int32_t n);
static void newline(void);
static void putc_tb(int32_t ch) {
  if (ch == 13)
    return;
  if (ch == 10) {
    pcg_putcscr(10);
    return;
  }
  if (ch == 8) {
    if (pcg_cur_x) {
      pcg_locate(pcg_cur_x - 1, pcg_cur_y);
      pcg_putcscr(32);
      pcg_locate(pcg_cur_x - 1, pcg_cur_y);
    }
    return;
  }
  pcg_putcscr((uint32_t)ch);
}

static void print_string(const char *s) {
  while (*s)
    putc_tb((uint8_t)*s++);
}

static void newline(void) { pcg_prnlnscr(""); }

static void print_number(int32_t number) {
  uint32_t value;
  if (number < 0) {
    putc_tb('-');
    value = 0u - u32(number);
  } else {
    value = u32(number);
  }
  if (value >= 10)
    print_number((int32_t)(value / 10));
  putc_tb((int32_t)(value % 10) + 48);
}

static int32_t digits(int32_t number) {
  uint32_t value;
  int32_t count = 0;
  if (number < 0) {
    value = 0u - u32(number);
    count = 1;
  } else {
    value = u32(number);
  }
  if (!value)
    return 1;
  while (value) {
    ++count;
    value /= 10;
  }
  return count;
}

static void print_padded(int32_t number, int32_t width) {
  int32_t padding = width - digits(number);
  while (padding-- > 0)
    putc_tb(' ');
  print_number(number);
}

static void help(void) {
  print_string("TinyBASIC: LET, PRINT, INPUT, IF/THEN, GOTO, GOSUB/RETURN,");
  newline();
  print_string("FOR/NEXT, RUN, LIST, NEW, CLEAR, LOAD, SAVE, REM, END");
  newline();
  print_string("Expressions: + - * / ");
  putc_tb('\\');
  print_string(" MOD ^, comparisons, AND/OR/NOT,");
  newline();
  print_string("ABS(), ASC(), RND(), SGN(), integer variables A-Z, @(n).");
  newline();
  print_string("PRINT: numbers use width 8; #N sets width for the rest of PRINT.");
  newline();
  print_string("LOAD/SAVE use slot 0: LBA and sector count; LOAD clamps "
               "sectors to image size.");
  newline();
}

static void report_error(void) {
  if (!line_no) {
    print_string("? ERROR");
  } else {
    print_string("? ERROR IN LINE ");
    print_number(line_no);
  }
  newline();
}

static void copy_line(const char *src, char *dst) {
  uint32_t i = 0;
  while (src[i] && i < MAX_TEXT) {
    dst[i] = src[i];
    ++i;
  }
  dst[i] = 0;
}

static void clear_program(void) {
  for (int32_t i = 1; i <= MAX_LINE; ++i)
    *line_ptr(i) = 0;
  line_count = 0;
}

static void list_program(void) {
  for (int32_t i = 1; i <= MAX_LINE; ++i) {
    if (*line_ptr(i)) {
      print_number(i);
      putc_tb(' ');
      print_string(line_ptr(i));
      newline();
    }
  }
}

static void clear_variables(void) {
  for (uint32_t i = 0; i < 26; ++i)
    vars[i] = 0;
  for (uint32_t i = 0; i < ARRAY_SIZE; ++i)
    arrays[i] = 0;
}

static void store_input_line(void) {
  uint32_t pos = 0;
  int32_t number = 0;
  while (input[pos] == ' ' || input[pos] == 9)
    ++pos;
  if (input[pos] < '0' || input[pos] > '9')
    return;
  while (input[pos] >= '0' && input[pos] <= '9')
    number = number * 10 + input[pos++] - '0';
  if (number < 1 || number > MAX_LINE) {
    print_string("LINE RANGE");
    newline();
    return;
  }
  while (input[pos] == ' ' || input[pos] == 9)
    ++pos;
  if (!input[pos])
    *line_ptr(number) = 0;
  else
    copy_line(input + pos, line_ptr(number));
}

static int32_t key_ascii(uint32_t usage, int32_t shifted, int32_t caps) {
  int32_t ch;
  if (usage >= 4 && usage <= 29) {
    ch = (int32_t)usage + 93;
    if (shifted ^ caps)
      ch -= 32;
    return ch;
  }
  if (usage >= 30 && usage <= 39) {
    static const char normal_digits[] = "1234567890";
    static const char shifted_digits[] = "!@#$%^&*()";
    uint32_t index = usage - 30u;
    return shifted ? shifted_digits[index] : normal_digits[index];
  }
  if (usage == 40)
    return 13;
  if (usage == 42)
    return 8;
  if (usage == 44)
    return ' ';
  if (usage == 45)
    return shifted ? '_' : '-';
  if (usage == 46)
    return shifted ? '+' : '=';
  if (usage == 47)
    return shifted ? '{' : '[';
  if (usage == 48)
    return shifted ? '}' : ']';
  if (usage == 49)
    return shifted ? '|' : '\\';
  if (usage == 51)
    return shifted ? ':' : ';';
  if (usage == 52)
    return shifted ? '"' : '\'';
  if (usage == 54)
    return shifted ? '<' : ',';
  if (usage == 55)
    return shifted ? '>' : '.';
  if (usage == 56)
    return shifted ? '?' : '/';
  if (usage == 53)
    return shifted ? '~' : '`';
  return 0;
}
static void irq3(void);
static int32_t read_line(char *dest, int32_t cap) {
  int32_t count = 0;
  shift_down = 0;
  for (;;) {
    uint32_t event, usage;
    int32_t value, ch;
    while (event_tail == event_head) {
      if (CPT32_MMIO32(PEC_STATUS) & 1u) {
        irq3();
        continue;
      }
      cpt32_halt();
      irq3();
    }
    event = event_ring[event_tail++ & 511u];
    if ((event >> 28) != 0 || ((event >> 24) & 15u) != 0)
      continue;
    usage = (event >> 16) & 255u;
    value = event & 1u;
    if (usage == 225 || usage == 229) {
      shift_down = value;
    } else if (usage == 57 && value) {
      caps_lock = !caps_lock;
    } else if (value) {
      ch = key_ascii(usage, shift_down, caps_lock);
      if (ch == 13) {
        dest[count] = 0;
        newline();
        return count;
      }
      if (ch == 8) {
        if (count > 0) {
          dest[--count] = 0;
          putc_tb(8);
        }
      } else if (ch != 0 && cap > 0 && count < cap - 1) {
        dest[count++] = (char)ch;
        putc_tb(ch);
      }
    }
  }
}
static void irq3(void) {
  while (CPT32_MMIO32(PEC_STATUS) & 1u) {
    uint32_t event = CPT32_MMIO32(PEC_DATA);
    if ((uint32_t)(event_head - event_tail) < 512u) {
      event_ring[event_head & 511u] = event;
      ++event_head;
    }
    CPT32_MMIO32(PEC_POP) = 1;
  }
}
static int word_eq(const char *text) {
  uint32_t i = 0;
  while (wordbuf[i] == (uint8_t)text[i]) {
    if (!wordbuf[i])
      return 1;
    ++i;
  }
  return 0;
}

static int32_t keyword(void) {
#define KW(word, token)                                                        \
  if (word_eq(word))                                                           \
  return token
  KW("and", T_AND);
  KW("or", T_OR);
  KW("not", T_NOT);
  KW("mod", T_MOD);
  KW("abs", T_ABS);
  KW("asc", T_ASC);
  KW("rnd", T_RND);
  KW("irnd", T_RND);
  KW("sgn", T_SGN);
  KW("let", T_LET);
  KW("print", T_PRINT);
  KW("input", T_INPUT);
  KW("if", T_IF);
  KW("then", T_THEN);
  KW("goto", T_GOTO);
  KW("gosub", T_GOSUB);
  KW("return", T_RETURN);
  KW("for", T_FOR);
  KW("to", T_TO);
  KW("next", T_NEXT);
  KW("run", T_RUN);
  KW("list", T_LIST);
  KW("new", T_NEW);
  KW("clear", T_CLEAR);
  KW("cls", T_CLS);
  KW("end", T_END);
  KW("stop", T_STOP);
  KW("rem", T_REM);
  KW("help", T_HELP);
  KW("quit", T_QUIT);
  KW("bye", T_BYE);
  KW("load", T_LOAD);
  KW("save", T_SAVE);
  KW("tron", T_TRON);
  KW("troff", T_TROFF);
#undef KW
  return T_IDENT;
}

static void emit_token(int32_t type, int32_t value) {
  if (token_count < TOK_MAX) {
    tokens[token_count].type = type;
    tokens[token_count++].value = value;
  } else {
    error = 1;
  }
}

static int32_t lex_step(const char *src, uint32_t pos) {
  int32_t ch = (uint8_t)src[pos];
  int32_t value = 0;
  if (ch == ' ' || ch == 9)
    return (int32_t)pos + 1;
  if (!ch || ch == 39) {
    emit_token(T_EOF, 0);
    return -1;
  }
  if (ch == 34) {
    uint32_t start = str_used;
    ++pos;
    while (src[pos] && src[pos] != 34) {
      if (str_used < 4000)
        strbuf[str_used++] = src[pos];
      ++pos;
    }
    strbuf[str_used] = 0;
    ++str_used;
    emit_token(T_STR, (int32_t)start);
    if (error)
      return -1;
    if (src[pos] == 34)
      ++pos;
    return (int32_t)pos;
  }
  if (ch >= '0' && ch <= '9') {
    while (src[pos] >= '0' && src[pos] <= '9')
      value = value * 10 + (uint8_t)src[pos++] - '0';
    emit_token(T_NUM, value);
    return error ? -1 : (int32_t)pos;
  }
  if ((ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z')) {
    uint32_t length = 0;
    while ((src[pos] >= 'A' && src[pos] <= 'Z') ||
           (src[pos] >= 'a' && src[pos] <= 'z') ||
           (src[pos] >= '0' && src[pos] <= '9')) {
      int32_t letter = (uint8_t)src[pos++];
      if (letter >= 'A' && letter <= 'Z')
        letter += 32;
      if (length < 120)
        wordbuf[length++] = (char)letter;
    }
    wordbuf[length] = 0;
    int32_t type = keyword();
    emit_token(type, type == T_IDENT ? (uint8_t)wordbuf[0] - 'a' : 0);
    return error ? -1 : (int32_t)pos;
  }
  ++pos;
  if (ch == '<') {
    if (src[pos] == '=') {
      emit_token(T_LE, 0);
      return (int32_t)pos + 1;
    }
    if (src[pos] == '>') {
      emit_token(T_NE, 0);
      return (int32_t)pos + 1;
    }
  }
  if (ch == '>' && src[pos] == '=') {
    emit_token(T_GE, 0);
    return (int32_t)pos + 1;
  }
  emit_token(ch, 0);
  return error ? -1 : (int32_t)pos;
}

static void lex_line(const char *src) {
  uint32_t pos = 0;
  token_count = 0;
  str_used = 0;
  error = 0;
  while (!error) {
    int32_t next = lex_step(src, pos);
    if (next < 0)
      break;
    pos = (uint32_t)next;
  }
}

static Node *alloc_node(int32_t kind, int32_t value, Node *a, Node *b,
                        Node *c) {
  if (ast_bump + 20 > AST_LIMIT) {
    error = 1;
    return 0;
  }
  Node *node = (Node *)(uintptr_t)ast_bump;
  ast_bump += 20;
  node->kind = kind;
  node->value = value;
  node->a = a;
  node->b = b;
  node->c = c;
  return node;
}

static char *pool_string(const char *src) {
  uint32_t length = 0;
  while (src[length])
    ++length;
  if (str_bump + length + 1 > STR_POOL_LIMIT) {
    error = 1;
    return 0;
  }
  char *dest = (char *)(uintptr_t)str_bump;
  for (uint32_t i = 0; i <= length; ++i)
    dest[i] = src[i];
  str_bump += length + 1;
  return dest;
}

static int32_t peek(void) { return tokens[parser_pos].type; }
static int32_t peek_val(void) { return tokens[parser_pos].value; }

static int match(int32_t type) {
  if (peek() == type) {
    ++parser_pos;
    return 1;
  }
  return 0;
}

static int expect(int32_t type) {
  if (match(type))
    return 1;
  error = 1;
  return 0;
}

static int prec(int32_t token) {
  switch (token) {
  case T_OR:
    return 1;
  case T_AND:
    return 2;
  case '=':
  case '<':
  case '>':
  case T_LE:
  case T_GE:
  case T_NE:
    return 4;
  case '+':
  case '-':
    return 5;
  case '*':
  case '/':
  case 92:
  case T_MOD:
    return 6;
  case '^':
    return 8;
  default:
    return 0;
  }
}

static int op_code(int32_t token) {
  switch (token) {
  case '+':
    return OP_ADD;
  case '-':
    return OP_SUB;
  case '*':
    return OP_MUL;
  case '/':
    return OP_DIV;
  case 92:
    return OP_IDIV;
  case T_MOD:
    return OP_MOD;
  case '^':
    return OP_POW;
  case '=':
    return OP_EQ;
  case '<':
    return OP_LT;
  case '>':
    return OP_GT;
  case T_LE:
    return OP_LE;
  case T_GE:
    return OP_GE;
  case T_NE:
    return OP_NE;
  case T_AND:
    return OP_AND;
  case T_OR:
    return OP_OR;
  default:
    return 0;
  }
}
static Node *parse_expr(int32_t min);
static Node *parse_stmt_list(void);
static Node *parse_builtin(int32_t t) {
  int32_t f = t == T_ABS   ? F_ABS
              : t == T_SGN ? F_SGN
              : t == T_ASC ? F_ASC
                           : F_RND;
  Node *a;
  ++parser_pos;
  if (!expect('('))
    return 0;
  if (t == T_ASC && peek() == T_STR) {
    char *s = pool_string(strbuf + (uint32_t)peek_val());
    if (error)
      return 0;
    ++parser_pos;
    a = alloc_node(N_STR, 0, (Node *)(uintptr_t)s, 0, 0);
  } else
    a = parse_expr(0);
  if (error)
    return 0;
  if (!expect(')'))
    return 0;
  return alloc_node(N_CALL, f, a, 0, 0);
}
static Node *parse_primary(void) {
  Node *n;
  if (peek() == T_NUM) {
    int32_t v = peek_val();
    ++parser_pos;
    return alloc_node(N_NUM, v, 0, 0, 0);
  }
  if (peek() == T_STR) {
    char *s = pool_string(strbuf + (uint32_t)peek_val());
    if (error)
      return 0;
    ++parser_pos;
    return alloc_node(N_STR, 0, (Node *)(uintptr_t)s, 0, 0);
  }
  if (peek() == '-') {
    ++parser_pos;
    n = parse_expr(7);
    return error ? 0 : alloc_node(N_UNOP, U_NEG, n, 0, 0);
  }
  if (peek() == '+') {
    ++parser_pos;
    return parse_expr(7);
  }
  if (peek() == T_NOT) {
    ++parser_pos;
    n = parse_expr(3);
    return error ? 0 : alloc_node(N_UNOP, U_NOT, n, 0, 0);
  }
  if (peek() == '(') {
    ++parser_pos;
    n = parse_expr(0);
    if (error || !expect(')'))
      return 0;
    return n;
  }
  if (peek() == '@') {
    ++parser_pos;
    if (!expect('('))
      return 0;
    n = parse_expr(0);
    if (error || !expect(')'))
      return 0;
    return alloc_node(N_ARR, 0, n, 0, 0);
  }
  error = 1;
  if (peek() != T_EOF)
    ++parser_pos;
  return 0;
}
static Node *parse_expr(int32_t min) {
  Node *l, *r;
  int32_t op, p;
  if (peek() == T_ABS || peek() == T_SGN || peek() == T_ASC || peek() == T_RND)
    l = parse_builtin(peek());
  else if (peek() == T_IDENT) {
    int32_t v = peek_val();
    ++parser_pos;
    l = alloc_node(N_VAR, v, 0, 0, 0);
  } else
    l = parse_primary();
  if (error || !l)
    return 0;
  while ((p = prec(peek())) && p >= min) {
    op = peek();
    ++parser_pos;
    r = parse_expr(p == 8 ? p : p + 1);
    if (error)
      return 0;
    l = alloc_node(N_BINOP, op_code(op), l, r, 0);
    if (error)
      return 0;
  }
  return l;
}
static Node *append(Node *h, Node **tail, Node *n) {
  if (!n)
    return h;
  if (!h)
    h = n;
  else
    (*tail)->c = n;
  *tail = n;
  return h;
}
static Node *parse_print(void) {
  Node *h = 0, *t = 0, *n, *e;
  ++parser_pos;
  while (peek() != T_EOF && peek() != ':') {
    if (peek() == T_STR) {
      char *s = pool_string(strbuf + (uint32_t)peek_val());
      if (error)
        return 0;
      ++parser_pos;
      n = alloc_node(PI_STR, 0, (Node *)(uintptr_t)s, 0, 0);
    } else if (peek() == ',' || peek() == ';') {
      int32_t sep = peek();
      ++parser_pos;
      n = alloc_node(PI_SEP, sep, 0, 0, 0);
    } else if (peek() == '#') {
      int32_t w;
      ++parser_pos;
      if (peek() != T_NUM) {
        error = 1;
        return 0;
      }
      w = peek_val();
      ++parser_pos;
      if (peek() == ',' || peek() == ';')
        ++parser_pos;
      n = alloc_node(PI_WIDTH, w, 0, 0, 0);
      if (error)
        return 0;
      h = append(h, &t, n);
      continue;
    } else {
      e = parse_expr(0);
      if (error)
        return 0;
      n = alloc_node(PI_EXPR, 0, e, 0, 0);
    }
    if (error)
      return 0;
    h = append(h, &t, n);
  }
  return h;
}
static Node *parse_sys(int32_t code) {
  Node *l = 0, *s = 0;
  if (code == SYS_LOAD || code == SYS_SAVE) {
    ++parser_pos;
    if (peek() == T_EOF || peek() == ':')
      return alloc_node(S_SYS, code, 0, 0, 0);
    l = parse_expr(0);
    if (error)
      return 0;
    if (peek() == ',' || peek() == ';')
      ++parser_pos;
    if (peek() != T_EOF && peek() != ':')
      s = parse_expr(0);
    if (error)
      return 0;
    return alloc_node(S_SYS, code, l, s, 0);
  }
  ++parser_pos;
  return alloc_node(S_SYS, code, 0, 0, 0);
}
static Node *parse_statement(void) {
  Node *n, *e, *cond, *branch, *limit;
  int32_t idx, prompt;
  if (peek() == T_REM) {
    ++parser_pos;
    while (peek() != T_EOF)
      ++parser_pos;
    return 0;
  }
  if (peek() == T_PRINT || peek() == '?') {
    n = parse_print();
    return error ? 0 : alloc_node(S_PRINT, 0, n, 0, 0);
  }
  if (peek() == T_INPUT) {
    ++parser_pos;
    prompt = 0;
    if (peek() == T_STR) {
      prompt = (int32_t)(uintptr_t)pool_string(strbuf + (uint32_t)peek_val());
      if (error)
        return 0;
      ++parser_pos;
      if (peek() == ',')
        ++parser_pos;
    }
    if (peek() != T_IDENT) {
      error = 1;
      return 0;
    }
    idx = peek_val();
    ++parser_pos;
    return alloc_node(S_INPUT, idx, (Node *)(uintptr_t)prompt, 0, 0);
  }
  if (peek() == T_LET) {
    ++parser_pos;
    if (peek() != T_IDENT) {
      error = 1;
      return 0;
    }
    idx = peek_val();
    ++parser_pos;
    if (!expect('='))
      return 0;
    e = parse_expr(0);
    return error ? 0 : alloc_node(S_LET, idx, e, 0, 0);
  }
  if (peek() == T_IDENT) {
    idx = peek_val();
    ++parser_pos;
    if (!expect('='))
      return 0;
    e = parse_expr(0);
    return error ? 0 : alloc_node(S_LET, idx, e, 0, 0);
  }
  if (peek() == '@') {
    ++parser_pos;
    if (!expect('('))
      return 0;
    cond = parse_expr(0);
    if (error || !expect(')') || !expect('='))
      return 0;
    e = parse_expr(0);
    return error ? 0 : alloc_node(S_ARRLET, 0, cond, e, 0);
  }
  if (peek() == T_GOTO || peek() == T_GOSUB) {
    int32_t k = peek() == T_GOTO ? S_GOTO : S_GOSUB;
    ++parser_pos;
    e = parse_expr(0);
    return error ? 0 : alloc_node(k, 0, e, 0, 0);
  }
  if (peek() == T_RETURN) {
    ++parser_pos;
    return alloc_node(S_RETURN, 0, 0, 0, 0);
  }
  if (peek() == T_IF) {
    ++parser_pos;
    cond = parse_expr(0);
    if (error)
      return 0;
    if (peek() == T_THEN)
      ++parser_pos;
    if (peek() == T_NUM) {
      idx = peek_val();
      ++parser_pos;
      return alloc_node(S_IF, idx, cond, 0, 0);
    }
    branch = parse_stmt_list();
    return error ? 0 : alloc_node(S_IF, 0, cond, branch, 0);
  }
  if (peek() == T_FOR) {
    ++parser_pos;
    if (peek() != T_IDENT) {
      error = 1;
      return 0;
    }
    idx = peek_val();
    ++parser_pos;
    if (!expect('='))
      return 0;
    e = parse_expr(0);
    if (error || !expect(T_TO))
      return 0;
    limit = parse_expr(0);
    return error ? 0 : alloc_node(S_FOR, idx, e, limit, 0);
  }
  if (peek() == T_NEXT) {
    ++parser_pos;
    idx = -1;
    if (peek() == T_IDENT) {
      idx = peek_val();
      ++parser_pos;
    }
    return alloc_node(S_NEXT, idx, 0, 0, 0);
  }
  if (peek() == T_END || peek() == T_STOP) {
    ++parser_pos;
    return alloc_node(S_SYS, SYS_END, 0, 0, 0);
  }
#define SYS(t, c)                                                              \
  if (peek() == t)                                                             \
  return parse_sys(c)
  SYS(T_RUN, SYS_RUN);
  SYS(T_LIST, SYS_LIST);
  SYS(T_NEW, SYS_NEW);
  SYS(T_CLEAR, SYS_CLEAR);
  SYS(T_CLS, SYS_CLS);
  SYS(T_HELP, SYS_HELP);
  SYS(T_LOAD, SYS_LOAD);
  SYS(T_SAVE, SYS_SAVE);
  SYS(T_TRON, SYS_TRON);
  SYS(T_TROFF, SYS_TROFF);
#undef SYS
  if (peek() == T_QUIT || peek() == T_BYE)
    return parse_sys(SYS_BYE);
  if (peek() == T_EOF || peek() == ':')
    return 0;
  error = 1;
  return 0;
}
static Node *parse_stmt_list(void) {
  Node *h = 0, *t = 0, *n;
  for (;;) {
    if (peek() == ':') {
      ++parser_pos;
      continue;
    }
    if (peek() == T_EOF)
      break;
    n = parse_statement();
    if (error)
      return 0;
    h = append(h, &t, n);
  }
  return h;
}
static Node *parse_line(void) {
  parser_pos = 0;
  Node *h = parse_stmt_list();
  if (error || !expect(T_EOF))
    return 0;
  return h;
}
static int32_t power_i(int32_t a, int32_t e) {
  int32_t r = 1, f = a;
  if (e < 0)
    return 0;
  while (e > 0) {
    if (e & 1)
      r = mul_i(r, f);
    f = mul_i(f, f);
    e = (int32_t)(u32(e) >> 1);
  }
  return r;
}
static int32_t eval(Node *n);
static int32_t eval_op(int32_t o, int32_t a, int32_t b) {
  switch (o) {
  case OP_ADD:
    return add_i(a, b);
  case OP_SUB:
    return sub_i(a, b);
  case OP_MUL:
    return mul_i(a, b);
  case OP_DIV:
  case OP_IDIV:
    if (!b) {
      error = 1;
      return 0;
    }
    if (a == INT32_MIN && b == -1)
      return INT32_MIN;
    return a / b;
  case OP_MOD:
    if (!b) {
      error = 1;
      return 0;
    }
    if (a == INT32_MIN && b == -1)
      return 0;
    return a % b;
  case OP_POW:
    return power_i(a, b);
  case OP_EQ:
    return a == b;
  case OP_LT:
    return a < b;
  case OP_GT:
    return a > b;
  case OP_LE:
    return a <= b;
  case OP_GE:
    return a >= b;
  case OP_NE:
    return a != b;
  case OP_AND:
    return a & b;
  case OP_OR:
    return a | b;
  default:
    error = 1;
    return 0;
  }
}
static int32_t eval(Node *n) {
  int32_t a, b;
  if (!n) {
    error = 1;
    return 0;
  }
  switch (n->kind) {
  case N_NUM:
    return n->value;
  case N_STR:
    return *(uint8_t *)(uintptr_t)n->a;
  case N_VAR:
    return vars[n->value];
  case N_ARR:
    a = eval(n->a);
    if (error)
      return 0;
    if (a < 0 || a >= ARRAY_SIZE) {
      error = 1;
      return 0;
    }
    return arrays[a];
  case N_BINOP:
    a = eval(n->a);
    if (error)
      return 0;
    b = eval(n->b);
    if (error)
      return 0;
    return eval_op(n->value, a, b);
  case N_UNOP:
    a = eval(n->a);
    if (error)
      return 0;
    return n->value == U_NEG ? si(0u - u32(a)) : !a;
  case N_CALL:
    if (n->value == F_ASC && n->a->kind == N_STR)
      return *(uint8_t *)(uintptr_t)n->a->a;
    a = eval(n->a);
    if (error)
      return 0;
    if (n->value == F_ABS)
      return a < 0 ? si(0u - u32(a)) : a;
    if (n->value == F_SGN)
      return a > 0 ? 1 : (a < 0 ? -1 : 0);
    if (a <= 0)
      return 0;
    return (int32_t)((pec_rng_rand() & 0x7fffffffu) % (uint32_t)a) + 1;
  default:
    error = 1;
    return 0;
  }
}
static int32_t find_line(int32_t n) {
  for (int32_t i = 0; i < line_count; i++)
    if (directory[i].line == n)
      return i;
  return -1;
}
static void goto_line(int32_t n) {
  int32_t i = find_line(n);
  if (i < 0) {
    error = 1;
    return;
  }
  line_no = n;
  pc = i;
  current = directory[i].root;
  jump = 1;
}
static void build_program(void) {
  line_count = 0;
  ast_bump = AST_BASE;
  str_bump = STR_POOL_BASE;
  error = 0;
  for (int32_t ln = 1; ln <= MAX_LINE && !error; ln++) {
    char *a = line_ptr(ln);
    if (*a) {
      error = 0;
      lex_line(a);
      Node *r = error ? 0 : parse_line();
      if (error) {
        line_no = ln;
        break;
      }
      if (line_count < DIR_MAX) {
        directory[line_count].line = ln;
        directory[line_count++].root = r;
      } else {
        error = 1;
        line_no = ln;
      }
    }
  }
}
static int32_t input_value(void) {
  uint32_t p = 0;
  int32_t sign = 1, v = 0;
  if (input[0] == '-') {
    sign = -1;
    p = 1;
  } else if (input[0] == '+')
    p = 1;
  while (input[p] >= '0' && input[p] <= '9')
    v = v * 10 + input[p++] - '0';
  if (!p)
    v = (uint8_t)input[0];
  return mul_i(v, sign);
}
static void disk_transfer(int32_t lba, int32_t sectors, int32_t cmd,
                          int32_t *out) {
  uint32_t bytes, dc, status;
  if (sectors < 1 || sectors > 256) {
    *out = 1;
    return;
  }
  bytes = (uint32_t)sectors * 512;
  while (CPT32_MMIO32(IDC + 8) & 1u) {
  }
  while (CPT32_MMIO32(DMA_CH0 + 16) & 1u) {
  }
  CPT32_MMIO32(IDC + 8) = 6;
  CPT32_MMIO32(DMA_CH0 + 16) = 6;
  CPT32_MMIO32(DMA + 4) = 1;
  CPT32_MMIO32(DMA + 8) = 1;
  CPT32_MMIO32(IDC + 20) = 0;
  CPT32_MMIO32(IDC + 28) = u32(lba);
  CPT32_MMIO32(IDC + 32) = u32(sectors);
  if (cmd == 1) {
    CPT32_MMIO32(DMA_CH0) = IDC + 36;
    CPT32_MMIO32(DMA_CH0 + 4) = DISK_BASE;
    dc = 0x104;
  } else {
    CPT32_MMIO32(DMA_CH0) = DISK_BASE;
    CPT32_MMIO32(DMA_CH0 + 4) = IDC + 36;
    dc = 0x110;
  }
  CPT32_MMIO32(DMA_CH0 + 8) = bytes;
  CPT32_MMIO32(IDC + 24) = u32(cmd);
  CPT32_MMIO32(DMA_CH0 + 12) = dc;
  while (CPT32_MMIO32(DMA_CH0 + 16) & 1u) {
  }
  while (CPT32_MMIO32(IDC + 8) & 1u) {
  }
  status = CPT32_MMIO32(DMA_CH0 + 16);
  *out = ((status & 4u) || (CPT32_MMIO32(IDC + 8) & 4u)) ? 1 : 0;
}
static uint32_t disk_capacity(void) {
  uint32_t s;
  while (CPT32_MMIO32(IDC + 8) & 1u) {
  }
  CPT32_MMIO32(IDC + 8) = 6;
  CPT32_MMIO32(IDC + 20) = 0;
  CPT32_MMIO32(IDC + 24) = 3;
  while (CPT32_MMIO32(IDC + 8) & 1u) {
  }
  s = CPT32_MMIO32(IDC + 8);
  return (s & 4u) ? 0 : CPT32_MMIO32(IDC + 48);
}
static void buffer_byte(int32_t c) {
  if ((int32_t)disk_pos < disk_limit)
    bytes_at(DISK_BASE)[disk_pos++] = (uint8_t)c;
  else
    error = 1;
}
static void save_serialize(void) {
  disk_pos = 0;
  error = 0;
  for (int32_t ln = 1; ln <= MAX_LINE && !error; ln++) {
    char *a = line_ptr(ln);
    if (*a) {
      uint8_t tmp[4];
      int32_t v = ln, n = 0;
      do {
        tmp[n++] = (uint8_t)(v % 10 + '0');
        v /= 10;
      } while (v);
      while (n)
        buffer_byte(tmp[--n]);
      buffer_byte(' ');
      for (uint32_t p = 0; a[p] && (int32_t)disk_pos < disk_limit; p++)
        buffer_byte(a[p]);
      buffer_byte(13);
      buffer_byte(10);
    }
  }
  while ((int32_t)disk_pos < disk_limit)
    buffer_byte(0);
}
static void finish_load_line(int32_t st, int32_t ln, uint32_t addr,
                             uint32_t idx) {
  if (st == 1) {
    if (ln < 1 || ln > MAX_LINE) {
      print_string("LINE RANGE");
      newline();
    } else
      *line_ptr(ln) = 0;
  }
  if (st == 2)
    *bytes_at(addr) = 0;
  if (st == 3)
    *bytes_at(addr + idx) = 0;
}
static void load_disk_lines(uint32_t size) {
  uint32_t pos = 0, addr = 0, idx = 0, count = 0;
  int32_t c, ln = 0, st = 0;
  clear_program();
  while (pos < size) {
    c = bytes_at(DISK_BASE)[pos++];
    if (!c)
      continue;
    if (c == 10 || c == 13) {
      if (st != 0 && st != 4)
        finish_load_line(st, ln, addr, idx);
      st = 0;
      ln = 0;
      idx = 0;
      count = 0;
      continue;
    }
    if (count < MAX_TEXT) {
      if (st == 3) {
        *bytes_at(addr + idx++) = (uint8_t)c;
      } else if (st == 2) {
        if (c != ' ' && c != '\t') {
          *bytes_at(addr) = (uint8_t)c;
          idx = 1;
          st = 3;
        }
      } else if (st == 1) {
        if (c >= '0' && c <= '9')
          ln = ln * 10 + c - '0';
        else if (ln < 1 || ln > MAX_LINE) {
          print_string("LINE RANGE");
          newline();
          st = 4;
        } else {
          addr = (uint32_t)(uintptr_t)line_ptr(ln);
          if (c == ' ' || c == '\t')
            st = 2;
          else {
            *bytes_at(addr) = (uint8_t)c;
            idx = 1;
            st = 3;
          }
        }
      } else if (st == 0) {
        if (c >= '0' && c <= '9') {
          ln = c - '0';
          st = 1;
        } else if (c != ' ' && c != '\t')
          st = 4;
      }
      ++count;
    }
  }
  finish_load_line(st, ln, addr, idx);
}
static void exec_one(Node *n);
static void exec_list(Node *n) {
  while (n && !stop && !error && !jump) {
    Node *next = n->c;
    exec_one(n);
    n = next;
  }
}
static void cls(void) {
  pcg_scroll_y = 0;
  pcg_vram_row = 0;
  CPT32_MMIO8(0x8000f00e) = 0;
  CPT32_MMIO8(0x8000f00f) = 0;
  for (uint32_t i = 0; i < 1200; i++) {
    bytes_at(0x10000000 + i)[0] = 32;
    bytes_at(0x10001000 + i)[0] = 63;
    bytes_at(0x10002000 + i)[0] = 0;
  }
  pcg_locate(0, 0);
}
static void exec_sys(int32_t code) {
  if (code == SYS_RUN) {
    clear_variables();
    gosub_sp = for_sp = 0;
    stop = error = 0;
    build_program();
    if (error)
      return;
    pc = 0;
    if (line_count) {
      line_no = directory[0].line;
      current = directory[0].root;
    } else {
      current = 0;
      line_no = 0;
    }
    jump = 1;
    return;
  }
  if (code == SYS_LIST) {
    list_program();
    return;
  }
  if (code == SYS_NEW) {
    clear_program();
    clear_variables();
    if (pc >= 0)
      stop = 1;
    return;
  }
  if (code == SYS_CLEAR) {
    clear_variables();
    return;
  }
  if (code == SYS_CLS) {
    cls();
    return;
  }
  if (code == SYS_HELP) {
    help();
    return;
  }
  if (code == SYS_TRON) {
    trace = 1;
    return;
  }
  if (code == SYS_TROFF) {
    trace = 0;
    return;
  }
  if (code == SYS_END) {
    stop = 1;
    return;
  }
  if (code == SYS_BYE) {
    for (;;)
      cpt32_halt();
  }
  error = 1;
}
static void exec_disk(int32_t cmd, Node *lban, Node *secn) {
  int32_t lba = 0, sectors = 0, res;
  uint32_t capacity, available;
  if (lban) {
    lba = eval(lban);
    if (error)
      return;
  }
  if (secn) {
    sectors = eval(secn);
    if (error)
      return;
  } else
    sectors = 128;
  if (cmd == SYS_LOAD) {
    capacity = disk_capacity();
    if (lba < (int32_t)capacity) {
      available = u32(sub_i((int32_t)capacity, lba));
      if (sectors > (int32_t)available)
        sectors = (int32_t)available;
      disk_transfer(lba, sectors, 1, &res);
      if (!res) {
        load_disk_lines((uint32_t)sectors * 512u);
        print_string("LOADED");
        newline();
      } else {
        print_string("DISK READ ERROR");
        newline();
      }
    } else {
      print_string("DISK READ ERROR");
      newline();
    }
  } else if (!lban || !secn) {
    print_string("SAVE NEEDS LBA AND SECTORS");
    newline();
  } else {
    disk_limit = mul_i(sectors, 512);
    save_serialize();
    if (!error) {
      disk_transfer(lba, sectors, 2, &res);
      if (!res) {
        print_string("SAVED");
        newline();
      } else {
        print_string("DISK WRITE ERROR");
        newline();
      }
    } else {
      print_string("SAVE BUFFER FULL");
      newline();
    }
  }
  if (pc >= 0 && cmd == SYS_LOAD)
    stop = 1;
}
static void exec_print(Node *items) {
  int32_t nl = 1, v, width = 8;
  for (Node *p = items; p; p = p->c) {
    if (p->kind == PI_STR) {
      print_string((char *)(uintptr_t)p->a);
      nl = 1;
    } else if (p->kind == PI_WIDTH) {
      width = p->value;
    } else if (p->kind == PI_EXPR) {
      v = eval(p->a);
      if (error)
        return;
      print_padded(v, width);
      nl = 1;
    } else
      nl = 0;
  }
  if (nl)
    newline();
}
static void exec_one(Node *node) {
  int32_t value, index, limit;
  switch (node->kind) {
  case S_PRINT:
    exec_print(node->a);
    return;
  case S_LET:
    value = eval(node->a);
    if (!error)
      vars[node->value] = value;
    return;
  case S_ARRLET:
    index = eval(node->a);
    if (error)
      return;
    value = eval(node->b);
    if (error)
      return;
    if (index < 0 || index >= ARRAY_SIZE)
      error = 1;
    else
      arrays[index] = value;
    return;
  case S_INPUT:
    if (node->a)
      print_string((char *)(uintptr_t)node->a);
    else
      print_string("? ");
    read_line(input, 120);
    vars[node->value] = input_value();
    return;
  case S_GOTO:
    value = eval(node->a);
    if (!error)
      goto_line(value);
    return;
  case S_GOSUB:
    if (gosub_sp >= 64) {
      error = 1;
      return;
    }
    gosub[gosub_sp * 2] = pc;
    gosub[gosub_sp * 2 + 1] = (int32_t)(uintptr_t)current;
    ++gosub_sp;
    value = eval(node->a);
    if (error) {
      --gosub_sp;
      return;
    }
    goto_line(value);
    return;
  case S_RETURN:
    if (gosub_sp <= 0) {
      error = 1;
      return;
    }
    --gosub_sp;
    pc = gosub[gosub_sp * 2];
    current = (Node *)(uintptr_t)gosub[gosub_sp * 2 + 1];
    line_no = pc >= 0 ? directory[pc].line : 0;
    jump = 1;
    return;
  case S_IF:
    value = eval(node->a);
    if (error || !value)
      return;
    if (node->value)
      goto_line(node->value);
    else
      exec_list(node->b);
    return;
  case S_FOR:
    value = eval(node->a);
    if (error)
      return;
    limit = eval((Node *)(uintptr_t)node->b);
    if (error)
      return;
    vars[node->value] = value;
    if (for_sp >= 32) {
      error = 1;
      return;
    }
    for_mem[for_sp * 4] = node->value;
    for_mem[for_sp * 4 + 1] = limit;
    for_mem[for_sp * 4 + 2] = pc;
    for_mem[for_sp * 4 + 3] = (int32_t)(uintptr_t)current;
    ++for_sp;
    return;
  case S_NEXT:
    if (for_sp <= 0) {
      error = 1;
      return;
    }
    index = for_mem[(for_sp - 1) * 4];
    if (node->value != -1 && node->value != index) {
      error = 1;
      return;
    }
    value = add_i(vars[index], 1);
    vars[index] = value;
    if (value <= for_mem[(for_sp - 1) * 4 + 1]) {
      pc = for_mem[(for_sp - 1) * 4 + 2];
      current = (Node *)(uintptr_t)for_mem[(for_sp - 1) * 4 + 3];
      line_no = pc >= 0 ? directory[pc].line : 0;
      jump = 1;
    } else {
      --for_sp;
    }
    return;
  case S_SYS:
    if (node->value == SYS_LOAD || node->value == SYS_SAVE)
      exec_disk(node->value, node->a, node->b);
    else
      exec_sys(node->value);
    return;
  default:
    error = 1;
    return;
  }
}
static void run_loop(void) {
  for (;;) {
    if (stop || error)
      break;
    if (!current) {
      if (pc < 0)
        break;
      ++pc;
      if (pc >= line_count) {
        stop = 1;
        break;
      }
      line_no = directory[pc].line;
      current = directory[pc].root;
      if (trace) {
        print_number(line_no);
        putc_tb(' ');
        print_string(line_ptr(line_no));
        newline();
      }
      continue;
    }
    Node *node = current;
    current = current->c;
    jump = 0;
    exec_one(node);
  }
  if (error) {
    report_error();
    error = 0;
  }
}
static void direct_line(void) {
  Node *head;
  lex_line(input);
  if (token_count && tokens[0].type == T_NUM) {
    error = 0;
    store_input_line();
    return;
  }
  if (error) {
    print_string("? ERROR");
    newline();
    error = 0;
    return;
  }
  head = parse_line();
  if (error) {
    print_string("? ERROR");
    newline();
    error = 0;
    return;
  }
  pc = -1;
  current = head;
  line_no = 0;
  stop = 0;
  run_loop();
}
static void repl(void) {
  cls();
  help();
  for (;;) {
    ast_bump = AST_BASE;
    str_bump = STR_POOL_BASE;
    print_string("OK");
    newline();
    print_string("> ");
    read_line(input, 120);
    direct_line();
  }
}
int main(void) {
  CPT32_MMIO8(0x80000000u) = 1;
  pcg_initPCG();
  pec_rng_initPRNG();
  pcg_col80();
  irqc_init();
  irqc_register(3, irq3);
  irqc_enable(8);
  repl();
  return 0;
}
