/* Foundation shell 1.0 (doc/spec_Foundation_shell.md), the part that runs
 * on the current kernel: lexing and syntax (§3, §4), variables (§5.1), the
 * built-in commands that need no file system (§10), messages (§11) and the
 * interactive line editor (§9).
 *
 * Not yet possible, and reported as FND_E_UNSUPPORTED or "command not found"
 * (README.md lists them): the fs service and libfnd file calls (file
 * redirection, wildcards, cd to a path, ls/cat/cp/rm/mv/mkdir/rmdir, scripts,
 * /etc/profile.fsh and PATH lookup), proc_spawn (external programs) and the
 * Ctrl+C break thread (thread_create, CON_BREAK_WAIT). */
#include <stdint.h>

#include <foundation/fnd.h>
#include <foundation/sys.h>

#define SHELL_VERSION "1.0"
#define LINE_MAX 1023u
#define WORDS_MAX 64u
#define VARS_MAX 64u
#define NAME_MAX 31u
#define VALUE_MAX 1023u
#define HISTORY_MAX 16u
#define TOKENS_MAX 1024u
#define SEGS_MAX 1024u
#define POOL_MAX 2048u
#define ARGS_MAX 8192u
#define CMDS_MAX 512u

/* ---- strings ---- */

static uint32_t str_len(const char *s) {
    uint32_t n = 0;
    while (s[n])
        ++n;
    return n;
}

static uint32_t ascii_upper(uint32_t c) { return c >= 'a' && c <= 'z' ? c - 32u : c; }

/* ASCII case-insensitive comparison of [a, a + na) and [b, b + nb). */
static int name_cmp(const char *a, uint32_t na, const char *b, uint32_t nb) {
    for (uint32_t i = 0; i < na && i < nb; ++i) {
        uint32_t x = ascii_upper((uint8_t)a[i]), y = ascii_upper((uint8_t)b[i]);
        if (x != y)
            return x < y ? -1 : 1;
    }
    return na == nb ? 0 : na < nb ? -1 : 1;
}

static int name_is(const char *word, const char *name) {
    return name_cmp(word, str_len(word), name, str_len(name)) == 0;
}

static void copy_bytes(char *dst, const char *src, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i)
        dst[i] = src[i];
}

static uint32_t format_i32(int32_t value, char *out) {
    char digits[11];
    uint32_t n = 0, len = 0, v = value < 0 ? 0u - (uint32_t)value : (uint32_t)value;
    do {
        digits[n++] = (char)('0' + v % 10u);
        v /= 10u;
    } while (v);
    if (value < 0)
        out[len++] = '-';
    while (n)
        out[len++] = digits[--n];
    out[len] = 0;
    return len;
}

/* Decimal i32 (§10 exit); 0 if text is not one. */
static int parse_i32(const char *text, int32_t *out) {
    uint32_t i = 0, negative = 0, value = 0;
    if (text[0] == '-' || text[0] == '+')
        negative = text[i++] == '-';
    if (!text[i])
        return 0;
    for (; text[i]; ++i) {
        uint32_t d = (uint8_t)text[i] - '0';
        if (d > 9u || value > (0x80000000u - d) / 10u)
            return 0;
        value = value * 10u + d;
    }
    if (!negative && value > 0x7FFFFFFFu)
        return 0;
    *out = negative ? (int32_t)(0u - value) : (int32_t)value;
    return 1;
}

/* ---- output ---- */

static void put(fnd_handle h, const char *s, uint32_t n) {
    if (n)
        (void)fnd_write(h, s, n, 0);
}
static void out(const char *s) { put(fnd_std_out(), s, str_len(s)); }
static void out_n(const char *s, uint32_t n) { put(fnd_std_out(), s, n); }
static void err(const char *s) { put(fnd_std_err(), s, str_len(s)); }

static void out_dec(int32_t value, uint32_t width) {
    char text[12];
    uint32_t n = format_i32(value, text);
    while (n < width--)
        out(" ");
    out(text);
}

/* §11.1: "<command>: <target>: <message> (<code>)", target optional. */
static int32_t report(const char *command, const char *target, int32_t code) {
    char text[12];
    err(command);
    err(": ");
    if (target) {
        err(target);
        err(": ");
    }
    err(fnd_strerror(code));
    err(" (");
    format_i32(code, text);
    err(text);
    err(")\n");
    return code;
}

static int32_t usage(const char *text) {
    err("usage: ");
    err(text);
    err("\n");
    return FND_E_BAD_ARG;
}

/* ---- variables (§5.1) ---- */

struct var {
    uint32_t name_len, value_len;
    char name[NAME_MAX + 1u];
    char value[VALUE_MAX + 1u];
};
static struct var vars[VARS_MAX];
static uint32_t var_count;
static int32_t last_status; /* $? */

static struct var *var_find(const char *name, uint32_t len) {
    for (uint32_t i = 0; i < var_count; ++i) {
        if (name_cmp(vars[i].name, vars[i].name_len, name, len) == 0)
            return &vars[i];
    }
    return 0;
}

static int name_start(uint32_t c) { return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_'; }
static int name_char(uint32_t c) { return name_start(c) || (c >= '0' && c <= '9'); }

static int valid_name(const char *name, uint32_t len) {
    if (len == 0 || len > NAME_MAX || !name_start((uint8_t)name[0]))
        return 0;
    for (uint32_t i = 1; i < len; ++i) {
        if (!name_char((uint8_t)name[i]))
            return 0;
    }
    return 1;
}

/* An empty value removes the variable. The list stays sorted by name. */
static int32_t var_set(const char *name, uint32_t len, const char *value, uint32_t value_len) {
    if (value_len > VALUE_MAX)
        return FND_E_LIMIT;
    struct var *v = var_find(name, len);
    if (value_len == 0) {
        if (v) {
            uint32_t i = (uint32_t)(v - vars);
            for (; i + 1u < var_count; ++i)
                vars[i] = vars[i + 1u];
            --var_count;
        }
        return FND_OK;
    }
    if (!v) {
        if (var_count == VARS_MAX)
            return FND_E_LIMIT;
        uint32_t i = var_count++;
        for (; i && name_cmp(vars[i - 1u].name, vars[i - 1u].name_len, name, len) > 0; --i)
            vars[i] = vars[i - 1u];
        v = &vars[i];
        copy_bytes(v->name, name, len);
        v->name[len] = 0;
        v->name_len = len;
    }
    copy_bytes(v->value, value, value_len);
    v->value[value_len] = 0;
    v->value_len = value_len;
    return FND_OK;
}

/* ---- lexing (§3) ---- */

enum { TK_WORD, TK_SEMI, TK_AND, TK_OR, TK_IN, TK_OUT, TK_APPEND, TK_ERR, TK_ERR_APPEND, TK_ERR_DUP };
enum { SEG_TEXT, SEG_VAR, SEG_SPECIAL };

/* A word is a run of segments: text bytes in pool, or a variable whose name
 * is in pool (SEG_SPECIAL: one of ?, #, 0-9). */
struct seg {
    uint16_t kind, start, len;
};
struct token {
    uint16_t kind, first, count, glob; /* glob: an unquoted * or ? */
};

static struct token tokens[TOKENS_MAX];
static struct seg segs[SEGS_MAX];
static char pool[POOL_MAX];
static uint32_t token_count, seg_count, pool_len;
static const char *lex_error; /* target text of a lexing error */

static int add_text(struct token *t, const char *text, uint32_t len) {
    if (pool_len + len > POOL_MAX)
        return 0;
    struct seg *last = t->count ? &segs[seg_count - 1u] : 0;
    if (last && last->kind == SEG_TEXT && last->start + last->len == pool_len) {
        last->len = (uint16_t)(last->len + len);
    } else {
        if (seg_count == SEGS_MAX)
            return 0;
        segs[seg_count++] = (struct seg){SEG_TEXT, (uint16_t)pool_len, (uint16_t)len};
        ++t->count;
    }
    copy_bytes(pool + pool_len, text, len);
    pool_len += len;
    return 1;
}

static int add_var(struct token *t, uint32_t kind, const char *name, uint32_t len) {
    if (seg_count == SEGS_MAX || pool_len + len > POOL_MAX)
        return 0;
    segs[seg_count++] = (struct seg){(uint16_t)kind, (uint16_t)pool_len, (uint16_t)len};
    ++t->count;
    copy_bytes(pool + pool_len, name, len);
    pool_len += len;
    return 1;
}

/* At line[*pos] == '$'. A '$' that starts no reference is a literal. */
static int32_t lex_dollar(const char *line, uint32_t len, uint32_t *pos, struct token *t) {
    uint32_t p = *pos + 1u;
    uint32_t c = p < len ? (uint8_t)line[p] : 0;
    if (c == '?' || c == '#' || (c >= '0' && c <= '9')) {
        *pos = p + 1u;
        return add_var(t, SEG_SPECIAL, line + p, 1) ? FND_OK : FND_E_LIMIT;
    }
    if (c == '{') {
        uint32_t start = ++p;
        while (p < len && line[p] != '}')
            ++p;
        uint32_t n = p - start;
        if (p == len) {
            lex_error = "unterminated ${";
            return FND_E_BAD_ARG;
        }
        *pos = p + 1u;
        if (n == 1u && (line[start] == '?' || line[start] == '#' ||
                        (line[start] >= '0' && line[start] <= '9')))
            return add_var(t, SEG_SPECIAL, line + start, 1) ? FND_OK : FND_E_LIMIT;
        if (!valid_name(line + start, n)) {
            lex_error = "bad variable name";
            return FND_E_BAD_ARG;
        }
        return add_var(t, SEG_VAR, line + start, n) ? FND_OK : FND_E_LIMIT;
    }
    if (name_start(c)) {
        uint32_t start = p;
        while (p < len && name_char((uint8_t)line[p]))
            ++p;
        if (p - start > NAME_MAX) {
            lex_error = "bad variable name";
            return FND_E_BAD_ARG;
        }
        *pos = p;
        return add_var(t, SEG_VAR, line + start, p - start) ? FND_OK : FND_E_LIMIT;
    }
    *pos = p;
    return add_text(t, "$", 1) ? FND_OK : FND_E_LIMIT;
}

static int ends_word(uint32_t c) {
    return c == ' ' || c == '\t' || c == ';' || c == '&' || c == '|' || c == '<' || c == '>' ||
           c == '(' || c == ')' || c == '`';
}

static int32_t lex_word(const char *line, uint32_t len, uint32_t *pos, struct token *t) {
    uint32_t p = *pos;
    int32_t status;
    while (p < len && !ends_word((uint8_t)line[p])) {
        char c = line[p];
        if (c == '\'') {
            uint32_t start = ++p;
            while (p < len && line[p] != '\'')
                ++p;
            if (p == len) {
                lex_error = "unterminated quote";
                return FND_E_BAD_ARG;
            }
            if (!add_text(t, line + start, p - start))
                return FND_E_LIMIT;
            ++p;
        } else if (c == '"') {
            ++p;
            for (;;) {
                if (p == len) {
                    lex_error = "unterminated quote";
                    return FND_E_BAD_ARG;
                }
                char q = line[p];
                if (q == '"') {
                    ++p;
                    break;
                }
                if (q == '\\' && p + 1u < len &&
                    (line[p + 1u] == '"' || line[p + 1u] == '\\' || line[p + 1u] == '$')) {
                    if (!add_text(t, line + p + 1u, 1))
                        return FND_E_LIMIT;
                    p += 2u;
                } else if (q == '$') {
                    if ((status = lex_dollar(line, len, &p, t)) != FND_OK)
                        return status;
                } else {
                    if (!add_text(t, line + p, 1))
                        return FND_E_LIMIT;
                    ++p;
                }
            }
        } else if (c == '\\') {
            if (p + 1u == len) {
                lex_error = "trailing \\";
                return FND_E_BAD_ARG;
            }
            if (!add_text(t, line + p + 1u, 1))
                return FND_E_LIMIT;
            p += 2u;
        } else if (c == '$') {
            if ((status = lex_dollar(line, len, &p, t)) != FND_OK)
                return status;
        } else {
            if (c == '*' || c == '?')
                t->glob = 1;
            if (!add_text(t, line + p, 1))
                return FND_E_LIMIT;
            ++p;
        }
    }
    *pos = p;
    return FND_OK;
}

static int32_t push_token(uint32_t kind) {
    if (token_count == TOKENS_MAX)
        return FND_E_LIMIT;
    tokens[token_count++] = (struct token){(uint16_t)kind, (uint16_t)seg_count, 0, 0};
    return FND_OK;
}

static int32_t lex(const char *line, uint32_t len) {
    token_count = seg_count = pool_len = 0;
    lex_error = 0;
    uint32_t p = 0;
    while (p < len) {
        char c = line[p];
        char n1 = p + 1u < len ? line[p + 1u] : 0, n2 = p + 2u < len ? line[p + 2u] : 0;
        char n3 = p + 3u < len ? line[p + 3u] : 0;
        uint32_t kind, width = 1;
        if (c == ' ' || c == '\t') {
            ++p;
            continue;
        }
        if (c == '#')
            break; /* a comment: '#' at the start of a word */
        if (c == ';') {
            kind = TK_SEMI;
        } else if (c == '&' && n1 == '&') {
            kind = TK_AND, width = 2;
        } else if (c == '|' && n1 == '|') {
            kind = TK_OR, width = 2;
        } else if (c == '&' || c == '|' || c == '(' || c == ')' || c == '`') {
            static char reserved[4] = "'x'";
            reserved[1] = c;
            lex_error = reserved;
            return FND_E_UNSUPPORTED;
        } else if (c == '<') {
            kind = TK_IN;
        } else if (c == '>') {
            kind = n1 == '>' ? TK_APPEND : TK_OUT, width = n1 == '>' ? 2u : 1u;
        } else if (c == '2' && n1 == '>') {
            if (n2 == '>')
                kind = TK_ERR_APPEND, width = 3;
            else if (n2 == '&' && n3 == '1')
                kind = TK_ERR_DUP, width = 4;
            else
                kind = TK_ERR, width = 2;
        } else {
            int32_t status = push_token(TK_WORD);
            if (status == FND_OK)
                status = lex_word(line, len, &p, &tokens[token_count - 1u]);
            if (status != FND_OK)
                return status;
            continue;
        }
        int32_t status = push_token(kind);
        if (status != FND_OK)
            return status;
        p += width;
    }
    return FND_OK;
}

/* ---- syntax (§4) ---- */

enum { LINK_SEQ, LINK_AND, LINK_OR };

struct command {
    uint16_t link;          /* how it follows the previous command */
    uint16_t first, count;  /* its tokens, words and redirections */
};
static struct command cmds[CMDS_MAX];
static uint32_t cmd_count;

static int is_redirect(uint32_t kind) { return kind >= TK_IN; }

/* Checks the whole line before anything runs (a syntax error runs nothing). */
static int32_t parse(void) {
    uint32_t link = LINK_SEQ, words = 0, start = 0;
    cmd_count = 0;
    for (uint32_t i = 0; i <= token_count; ++i) {
        uint32_t kind = i < token_count ? tokens[i].kind : TK_SEMI;
        if (kind == TK_WORD) {
            ++words;
            continue;
        }
        if (is_redirect(kind)) {
            if (kind != TK_ERR_DUP) {
                if (i + 1u == token_count || tokens[i + 1u].kind != TK_WORD) {
                    lex_error = "missing redirection target";
                    return FND_E_BAD_ARG;
                }
                ++i; /* the target is not a command word */
            }
            continue;
        }
        /* ;, &&, || or the end of the line */
        if (words == 0) {
            int at_end = i == token_count;
            if (at_end && start == i && (cmd_count == 0 || link == LINK_SEQ))
                return FND_OK; /* empty line, or a trailing ';' */
            lex_error = "syntax error";
            return FND_E_BAD_ARG;
        }
        if (cmd_count == CMDS_MAX)
            return FND_E_LIMIT;
        cmds[cmd_count++] = (struct command){(uint16_t)link, (uint16_t)start, (uint16_t)(i - start)};
        link = kind == TK_AND ? LINK_AND : kind == TK_OR ? LINK_OR : LINK_SEQ;
        words = 0;
        start = i + 1u;
    }
    return FND_OK;
}

/* ---- expansion (§5) ---- */

static char args[ARGS_MAX];
static uint32_t args_len;

static int append_arg(const char *text, uint32_t len) {
    if (args_len + len >= ARGS_MAX)
        return 0;
    copy_bytes(args + args_len, text, len);
    args_len += len;
    return 1;
}

/* Expands one word into args, NUL-terminated; returns its start or -1. */
static int32_t expand(const struct token *t) {
    uint32_t start = args_len;
    for (uint32_t i = 0; i < t->count; ++i) {
        const struct seg *s = &segs[t->first + i];
        const char *text = pool + s->start;
        int ok = 1;
        if (s->kind == SEG_TEXT) {
            ok = append_arg(text, s->len);
        } else if (s->kind == SEG_VAR) {
            const struct var *v = var_find(text, s->len);
            if (v)
                ok = append_arg(v->value, v->value_len);
        } else if (text[0] == '?') {
            char number[12];
            ok = append_arg(number, format_i32(last_status, number));
        } else if (text[0] == '#') {
            ok = append_arg("0", 1); /* no script is running */
        } else if (text[0] == '0') {
            ok = append_arg("shell", 5);
        } /* $1..$9: empty outside a script */
        if (!ok)
            return -1;
    }
    if (!append_arg("", 1))
        return -1;
    return (int32_t)start;
}

/* ---- built-in commands (§10) ---- */

typedef int32_t (*builtin_fn)(uint32_t argc, char **argv);

struct builtin {
    const char *name;
    builtin_fn fn;
    const char *syntax;
    const char *summary;
};

static const struct builtin *find_builtin(const char *name);
static int exit_requested;
static int32_t exit_code;

/* No fs session yet: the cwd is always /. */
static int32_t cmd_cd(uint32_t argc, char **argv) {
    if (argc > 2u)
        return usage("cd [path]");
    if (argc == 1u)
        return FND_OK; /* to /, as there is no home directory in v1 */
    return report("cd", argv[1], FND_E_UNSUPPORTED);
}

static int32_t cmd_pwd(uint32_t argc, char **argv) {
    (void)argv;
    if (argc != 1u)
        return usage("pwd");
    out("/\n");
    return FND_OK;
}

/* Commands that need the fs service, which does not exist yet. */
static int32_t cmd_needs_fs(uint32_t argc, char **argv) {
    (void)argc;
    const struct builtin *b = find_builtin(argv[0]);
    return report(b->name, 0, FND_E_UNSUPPORTED);
}

static int32_t cmd_clear(uint32_t argc, char **argv) {
    (void)argv;
    if (argc != 1u)
        return usage("clear");
    fnd_status status = fnd_con_clear();
    return status == FND_OK ? FND_OK : report("clear", 0, status);
}

static int32_t cmd_echo(uint32_t argc, char **argv) {
    for (uint32_t i = 1; i < argc; ++i) {
        if (i > 1u)
            out(" ");
        out(argv[i]);
    }
    out("\n");
    return FND_OK;
}

static void show_var(const struct var *v) {
    out_n(v->name, v->name_len);
    out("=");
    out_n(v->value, v->value_len);
    out("\n");
}

static int32_t cmd_set(uint32_t argc, char **argv) {
    if (argc == 1u) {
        for (uint32_t i = 0; i < var_count; ++i)
            show_var(&vars[i]);
        return FND_OK;
    }
    if (argc != 2u)
        return usage("set [NAME[=value]]");
    const char *arg = argv[1];
    uint32_t n = 0;
    while (arg[n] && arg[n] != '=')
        ++n;
    if (!valid_name(arg, n))
        return usage("set [NAME[=value]]");
    if (!arg[n]) {
        const struct var *v = var_find(arg, n);
        if (!v)
            return report("set", arg, FND_E_NOT_FOUND);
        out_n(v->value, v->value_len);
        out("\n");
        return FND_OK;
    }
    char name[NAME_MAX + 1u];
    copy_bytes(name, arg, n);
    name[n] = 0;
    int32_t status = var_set(arg, n, arg + n + 1u, str_len(arg + n + 1u));
    return status == FND_OK ? FND_OK : report("set", name, status);
}

static int32_t read_sysinfo(uint32_t info[8]) {
    int32_t size = fnd_sys_info(FND_INFO_SYSTEM, info, FND_SYSINFO_BYTES);
    return size < 0 ? size : FND_OK;
}

/* uname and the start-up banner (§2.1): the OS, then the shell. */
static int32_t show_versions(void) {
    uint32_t info[8];
    int32_t status = read_sysinfo(info);
    if (status == FND_OK) {
        out("Foundation OS (kernel ABI ");
        out_dec((int32_t)info[0], 0);
        out(".");
        out_dec((int32_t)info[1], 0);
        out(")\n");
    }
    out("Foundation shell " SHELL_VERSION "\n");
    return status;
}

static int32_t cmd_uname(uint32_t argc, char **argv) {
    (void)argv;
    if (argc != 1u)
        return usage("uname");
    int32_t status = show_versions();
    return status == FND_OK ? FND_OK : report("uname", 0, status);
}

static int32_t cmd_free(uint32_t argc, char **argv) {
    (void)argv;
    if (argc != 1u)
        return usage("free");
    uint32_t info[8];
    int32_t status = read_sysinfo(info);
    if (status != FND_OK)
        return report("free", 0, status);
    out_dec((int32_t)(info[3] / 1024u), 8);
    out(" KiB total\n");
    out_dec((int32_t)(info[4] / 1024u), 8);
    out(" KiB free\n");
    status = fnd_sys_info(FND_INFO_MEMORY, info, FND_MEMINFO_BYTES);
    if (status < 0)
        return report("free", 0, status);
    out_dec((int32_t)(info[1] / 1024u), 8);
    out(" KiB VRAM\n");
    out_dec((int32_t)(info[2] / 1024u), 8);
    out(" KiB PCMRAM\n");
    return FND_OK;
}

#define PS_MAX 16u
static uint32_t ps_buf[PS_MAX * FND_PROCINFO_BYTES / 4u];

static void show_process(const uint32_t *p) {
    static const char *const states[3] = {"RUN  ", "BLOCK", "EXIT "};
    char name[17];
    for (uint32_t i = 0; i < 16u; ++i)
        name[i] = (char)(p[8u + i / 4u] >> (24u - 8u * (i % 4u)));
    name[16] = 0;
    out_dec((int32_t)p[0], 5);
    out_dec((int32_t)p[1], 7);
    out(" ");
    out(p[2] < 3u ? states[p[2]] : "?    ");
    out_dec((int32_t)p[3], 5);
    out_dec((int32_t)(p[5] / 1024u), 7);
    out("K ");
    out(name);
    out("\n");
}

static int32_t cmd_ps(uint32_t argc, char **argv) {
    (void)argv;
    if (argc != 1u)
        return usage("ps");
    int32_t size = fnd_sys_info(FND_INFO_PROCESSES, ps_buf, sizeof ps_buf);
    if (size < 0)
        return report("ps", 0, size);
    if ((uint32_t)size > sizeof ps_buf)
        size = (int32_t)sizeof ps_buf;
    out("  PID PARENT STATE PRIO     MEM NAME\n");
    for (uint32_t off = 0; off + FND_PROCINFO_BYTES <= (uint32_t)size; off += FND_PROCINFO_BYTES)
        show_process(ps_buf + off / 4u);
    return FND_OK;
}

/* CPU snapshots: four header words followed by (pid, cycles) pairs.
 * Subtraction is unsigned so clock/counter wrap is harmless within a refresh. */
#define CPU_WORDS (4u + PS_MAX * 2u)
static uint32_t top_before[CPU_WORDS], top_after[CPU_WORDS];

static int32_t read_cpu(uint32_t *buf) {
    int32_t size = fnd_sys_info(FND_INFO_CPU, buf, CPU_WORDS * 4u);
    if (size < 0)
        return size;
    if (size < (int32_t)FND_CPUINFO_HEADER_BYTES || size > (int32_t)(CPU_WORDS * 4u)
        || ((uint32_t)size - FND_CPUINFO_HEADER_BYTES) % FND_CPUINFO_RECORD_BYTES)
        return FND_E_FORMAT;
    return size / 4;
}

static uint32_t cpu_delta(uint32_t pid, uint32_t before_words, uint32_t after_words) {
    uint32_t previous = 0;
    for (uint32_t i = 4; i + 1u < before_words; i += 2u) {
        if (top_before[i] == pid)
            previous = top_before[i + 1u];
    }
    for (uint32_t i = 4; i + 1u < after_words; i += 2u) {
        if (top_after[i] == pid)
            return top_after[i + 1u] - previous;
    }
    return 0;
}

/* One decimal place, without overflowing a 32-bit cycles * 1000 product. */
static uint32_t cpu_percent(uint32_t cycles, uint32_t total) {
    uint32_t unit = total / 1000u;
    uint32_t value = unit ? cycles / unit : 0;
    return value > 1000u ? 1000u : value;
}

static void out_percent(uint32_t value) {
    out_dec((int32_t)(value / 10u), 3);
    out(".");
    out_dec((int32_t)(value % 10u), 0);
    out("%");
}

static int32_t cmd_top(uint32_t argc, char **argv) {
    int32_t count = 0; /* zero means interactive */
    if (argc != 1u && (argc != 3u || !name_is(argv[1], "-n")
        || !parse_i32(argv[2], &count) || count <= 0))
        return usage("top [-n count]");
    int32_t before_words = read_cpu(top_before);
    if (before_words < 0)
        return report("top", 0, before_words);
    uint32_t refresh = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    if (!refresh || refresh > 1000000u)
        return report("top", 0, FND_E_BAD_ARG);
    for (;;) {
        uint32_t begin = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO);
        while ((uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO) - begin < refresh) {
            fnd_key_t key;
            int32_t status = fnd_con_pollkey(&key);
            if (status < 0)
                return report("top", 0, status);
            if (status == FND_OK && key.code == FND_KEY_CHAR
                && (ascii_upper(key.ch) == 'Q'
                    || ((key.mods & FND_MOD_CTRL) && ascii_upper(key.ch) == 'C')))
                return FND_OK;
            fnd_sys_sleep(10u);
        }
        int32_t size = fnd_sys_info(FND_INFO_PROCESSES, ps_buf, sizeof ps_buf);
        if (size < 0 || (uint32_t)size > sizeof ps_buf)
            return report("top", 0, size < 0 ? size : FND_E_LIMIT);
        int32_t after_words = read_cpu(top_after);
        if (after_words < 0)
            return report("top", 0, after_words);
        uint32_t total = top_after[0] - top_before[0];
        uint32_t kernel = top_after[1] - top_before[1];
        uint32_t idle = top_after[2] - top_before[2];
        uint32_t user = total - kernel - idle;
        if (!count)
            fnd_con_clear();
        out("top - CPU (q/Ctrl+C to quit)\nuser ");
        out_percent(cpu_percent(user, total));
        out("  kernel~ ");
        out_percent(cpu_percent(kernel, total));
        out("  idle ");
        out_percent(cpu_percent(idle, total));
        out("\n");
        uint32_t info[8];
        int32_t status = read_sysinfo(info);
        if (status < 0)
            return report("top", 0, status);
        out("Mem: ");
        out_dec((int32_t)(info[3] / 1024u), 0);
        out("K total, ");
        out_dec((int32_t)(info[4] / 1024u), 0);
        out("K free\n");
        out("  CPU%   PID PARENT STATE PRIO     MEM NAME\n");
        uint32_t rows = (uint32_t)size / FND_PROCINFO_BYTES;
        uint32_t order[PS_MAX], cycles[PS_MAX];
        for (uint32_t i = 0; i < rows; ++i) {
            order[i] = i;
            cycles[i] = cpu_delta(ps_buf[i * FND_PROCINFO_BYTES / 4u],
                                  (uint32_t)before_words, (uint32_t)after_words);
        }
        for (uint32_t i = 0; i < rows; ++i) {
            for (uint32_t j = i + 1u; j < rows; ++j) {
                if (cycles[order[j]] > cycles[order[i]]) {
                    uint32_t tmp = order[i];
                    order[i] = order[j];
                    order[j] = tmp;
                }
            }
            out_percent(cpu_percent(cycles[order[i]], total));
            out(" ");
            show_process(ps_buf + order[i] * FND_PROCINFO_BYTES / 4u);
        }
        if (count && --count == 0)
            return FND_OK;
        for (uint32_t i = 0; i < (uint32_t)after_words; ++i)
            top_before[i] = top_after[i];
        before_words = after_words;
    }
}

static int32_t cmd_help(uint32_t argc, char **argv);

static int32_t cmd_exit(uint32_t argc, char **argv) {
    int32_t code = last_status;
    if (argc > 2u || (argc == 2u && !parse_i32(argv[1], &code)))
        return usage("exit [code]");
    exit_requested = 1;
    exit_code = code;
    return code;
}

static const struct builtin builtins[] = {
    {"cd", cmd_cd, "cd [path]", "change the current directory"},
    {"pwd", cmd_pwd, "pwd", "show the current directory"},
    {"ls", cmd_needs_fs, "ls [-l] [path...]", "list a directory"},
    {"cat", cmd_needs_fs, "cat path...", "write files to the output"},
    {"cp", cmd_needs_fs, "cp [-f] src... dst", "copy files"},
    {"rm", cmd_needs_fs, "rm path...", "delete files"},
    {"mv", cmd_needs_fs, "mv old new", "rename or move a file"},
    {"mkdir", cmd_needs_fs, "mkdir path...", "make directories"},
    {"rmdir", cmd_needs_fs, "rmdir path...", "remove empty directories"},
    {"clear", cmd_clear, "clear", "clear the screen"},
    {"echo", cmd_echo, "echo [word...]", "write words"},
    {"set", cmd_set, "set [NAME[=value]]", "show or set variables"},
    {"uname", cmd_uname, "uname", "show the OS and shell versions"},
    {"free", cmd_free, "free", "show memory"},
    {"ps", cmd_ps, "ps", "list processes"},
    {"top", cmd_top, "top [-n count]", "monitor CPU and processes"},
    {"help", cmd_help, "help [command]", "show help"},
    {"exit", cmd_exit, "exit [code]", "leave the shell"},
};
#define BUILTINS (sizeof builtins / sizeof builtins[0])

static const struct builtin *find_builtin(const char *name) {
    for (uint32_t i = 0; i < BUILTINS; ++i) {
        if (name_is(name, builtins[i].name))
            return &builtins[i];
    }
    return 0;
}

static int32_t cmd_help(uint32_t argc, char **argv) {
    if (argc > 2u)
        return usage("help [command]");
    if (argc == 2u) {
        const struct builtin *b = find_builtin(argv[1]);
        if (!b)
            return report("help", argv[1], FND_E_NOT_FOUND);
        out("usage: ");
        out(b->syntax);
        out("\n");
        return FND_OK;
    }
    for (uint32_t i = 0; i < BUILTINS; ++i) {
        uint32_t n = str_len(builtins[i].name);
        out(builtins[i].name);
        while (n++ < 8u)
            out(" ");
        out(builtins[i].summary);
        out("\n");
    }
    return FND_OK;
}

/* ---- execution (§6, §7) ---- */

static int32_t run_command(const struct command *c) {
    char *argv[WORDS_MAX + 1u];
    uint32_t argc = 0;
    int32_t redirect_target = -1;
    args_len = 0;
    for (uint32_t i = 0; i < c->count; ++i) {
        const struct token *t = &tokens[c->first + i];
        if (is_redirect(t->kind)) {
            if (t->kind == TK_ERR_DUP)
                continue; /* STDERR is already the console STDOUT */
            int32_t at = expand(&tokens[c->first + ++i]);
            if (at < 0)
                return report("shell", 0, FND_E_LIMIT);
            if (redirect_target < 0)
                redirect_target = at;
            continue;
        }
        if (argc == WORDS_MAX)
            return report("shell", "too many words", FND_E_LIMIT);
        int32_t at = expand(t);
        if (at < 0)
            return report("shell", 0, FND_E_LIMIT);
        argv[argc++] = args + at;
        if (t->glob) /* §5.2 needs fs directory listings */
            return report("shell", args + at, FND_E_UNSUPPORTED);
    }
    argv[argc] = 0;
    /* §6: opening files needs the fs service. */
    if (redirect_target >= 0)
        return report(argv[0], args + redirect_target, FND_E_UNSUPPORTED);
    int has_slash = 0;
    for (const char *p = argv[0]; *p; ++p)
        has_slash |= *p == '/';
    const struct builtin *b = has_slash ? 0 : find_builtin(argv[0]);
    if (!b) {
        /* §7.1 steps 1 and 3 need the fs service: nothing can be found. */
        err(argv[0]);
        err(": command not found (");
        char code[12];
        format_i32(FND_E_NOT_FOUND, code);
        err(code);
        err(")\n");
        return FND_E_NOT_FOUND;
    }
    return b->fn(argc, argv);
}

static int valid_utf8(const char *s, uint32_t len) {
    for (uint32_t i = 0; i < len;) {
        uint32_t b = (uint8_t)s[i], need, min;
        if (b < 0x80u) {
            ++i;
            continue;
        }
        if ((b & 0xE0u) == 0xC0u)
            need = 1, min = 0x80u, b &= 0x1Fu;
        else if ((b & 0xF0u) == 0xE0u)
            need = 2, min = 0x800u, b &= 0x0Fu;
        else if ((b & 0xF8u) == 0xF0u)
            need = 3, min = 0x10000u, b &= 0x07u;
        else
            return 0;
        for (uint32_t k = 1; k <= need; ++k) {
            if (i + k >= len || ((uint8_t)s[i + k] & 0xC0u) != 0x80u)
                return 0;
            b = b << 6 | ((uint8_t)s[i + k] & 0x3Fu);
        }
        if (b < min || b > 0x10FFFFu || (b >= 0xD800u && b <= 0xDFFFu))
            return 0;
        i += need + 1u;
    }
    return 1;
}

static void run_line(const char *line, uint32_t len) {
    for (uint32_t i = 0; i < len; ++i) {
        if (!line[i]) {
            last_status = report("shell", "NUL in line", FND_E_BAD_ARG);
            return;
        }
    }
    if (len > LINE_MAX) {
        last_status = report("shell", "line too long", FND_E_LIMIT);
        return;
    }
    if (!valid_utf8(line, len)) {
        last_status = report("shell", "bad UTF-8", FND_E_BAD_ARG);
        return;
    }
    int32_t status = lex(line, len);
    if (status == FND_OK)
        status = parse();
    if (status != FND_OK) {
        last_status = report("shell", lex_error, status);
        return;
    }
    for (uint32_t i = 0; i < cmd_count && !exit_requested; ++i) {
        if ((cmds[i].link == LINK_AND && last_status != 0) || (cmds[i].link == LINK_OR && last_status == 0))
            continue; /* not run: $? stays */
        last_status = run_command(&cmds[i]);
    }
}

/* ---- interactive input (§9) ---- */

static char line[LINE_MAX + 1u];
static uint32_t line_len, cursor;
static char history[HISTORY_MAX][LINE_MAX + 1u];
static uint32_t history_len[HISTORY_MAX], history_count;

static int continuation(char c) { return ((uint8_t)c & 0xC0u) == 0x80u; }

/* Screen cells of line[from, to): one per scalar value. */
static uint32_t cells(uint32_t from, uint32_t to) {
    uint32_t n = 0;
    for (uint32_t i = from; i < to; ++i)
        n += !continuation(line[i]);
    return n;
}

static void back(uint32_t count) {
    while (count--)
        out("\b");
}

static uint32_t prev_scalar(uint32_t at) {
    do
        --at;
    while (at && continuation(line[at]));
    return at;
}

static uint32_t next_scalar(uint32_t at) {
    do
        ++at;
    while (at < line_len && continuation(line[at]));
    return at;
}

/* Rewrites the screen from the cursor after line[cursor..] changed: the new
 * tail, blanks over erased cells, then back to the cursor. */
static void redraw_tail(uint32_t erased) {
    out_n(line + cursor, line_len - cursor);
    for (uint32_t i = 0; i < erased; ++i)
        out(" ");
    back(cells(cursor, line_len) + erased);
}

static void replace_line(const char *text, uint32_t len) {
    uint32_t old_cells = cells(0, line_len);
    back(cells(0, cursor));
    copy_bytes(line, text, len);
    line_len = len;
    cursor = 0;
    uint32_t new_cells = cells(0, line_len);
    redraw_tail(old_cells > new_cells ? old_cells - new_cells : 0);
    out_n(line, line_len);
    cursor = line_len;
}

static void encode(uint32_t cp, char *buf, uint32_t *n) {
    if (cp < 0x80u) {
        buf[0] = (char)cp, *n = 1;
    } else if (cp < 0x800u) {
        buf[0] = (char)(0xC0u | cp >> 6), buf[1] = (char)(0x80u | (cp & 0x3Fu)), *n = 2;
    } else if (cp < 0x10000u) {
        buf[0] = (char)(0xE0u | cp >> 12), buf[1] = (char)(0x80u | (cp >> 6 & 0x3Fu));
        buf[2] = (char)(0x80u | (cp & 0x3Fu)), *n = 3;
    } else {
        buf[0] = (char)(0xF0u | cp >> 18), buf[1] = (char)(0x80u | (cp >> 12 & 0x3Fu));
        buf[2] = (char)(0x80u | (cp >> 6 & 0x3Fu)), buf[3] = (char)(0x80u | (cp & 0x3Fu)), *n = 4;
    }
}

static void remember(void) {
    if (line_len == 0)
        return;
    if (history_count) {
        uint32_t last = history_count - 1u;
        uint32_t same = history_len[last] == line_len;
        for (uint32_t i = 0; same && i < line_len; ++i)
            same = history[last][i] == line[i];
        if (same)
            return; /* §9.2: not the same line twice in a row */
    }
    if (history_count == HISTORY_MAX) {
        for (uint32_t i = 1; i < HISTORY_MAX; ++i) {
            copy_bytes(history[i - 1u], history[i], history_len[i]);
            history_len[i - 1u] = history_len[i];
        }
        --history_count;
    }
    copy_bytes(history[history_count], line, line_len);
    history_len[history_count++] = line_len;
}

/* Edits one line; 0 when Enter confirmed it, 1 when Ctrl+C dropped it. */
static int read_line(void) {
    static char draft[LINE_MAX + 1u];
    uint32_t draft_len = 0, browse = history_count;
    line_len = cursor = 0;
    for (;;) {
        fnd_key_t key;
        if (fnd_con_getkey(&key) != FND_OK)
            fnd_exit(last_status);
        switch (key.code) {
        case FND_KEY_ENTER:
            out_n(line + cursor, line_len - cursor);
            out("\n");
            return 0;
        case FND_KEY_LEFT:
            if (cursor) {
                cursor = prev_scalar(cursor);
                back(1);
            }
            break;
        case FND_KEY_RIGHT:
            if (cursor < line_len) {
                uint32_t next = next_scalar(cursor);
                out_n(line + cursor, next - cursor);
                cursor = next;
            }
            break;
        case FND_KEY_HOME:
            back(cells(0, cursor));
            cursor = 0;
            break;
        case FND_KEY_END:
            out_n(line + cursor, line_len - cursor);
            cursor = line_len;
            break;
        case FND_KEY_BACKSPACE:
            if (cursor) {
                uint32_t at = prev_scalar(cursor), n = cursor - at;
                copy_bytes(line + at, line + cursor, line_len - cursor);
                line_len -= n;
                cursor = at;
                back(1);
                redraw_tail(1);
            }
            break;
        case FND_KEY_DELETE:
            if (cursor < line_len) {
                uint32_t n = next_scalar(cursor) - cursor;
                copy_bytes(line + cursor, line + cursor + n, line_len - cursor - n);
                line_len -= n;
                redraw_tail(1);
            }
            break;
        case FND_KEY_UP:
            if (browse) {
                if (browse == history_count) {
                    copy_bytes(draft, line, line_len);
                    draft_len = line_len;
                }
                --browse;
                replace_line(history[browse], history_len[browse]);
            }
            break;
        case FND_KEY_DOWN:
            if (browse < history_count) {
                ++browse;
                if (browse == history_count)
                    replace_line(draft, draft_len);
                else
                    replace_line(history[browse], history_len[browse]);
            }
            break;
        case FND_KEY_CHAR:
            if (key.mods & FND_MOD_CTRL) {
                if (key.ch == 'c' || key.ch == 'C') {
                    out_n(line + cursor, line_len - cursor);
                    out("^C\n");
                    return 1;
                }
                break;
            } else {
                char bytes[4];
                uint32_t n;
                encode(key.ch, bytes, &n);
                if (line_len + n > LINE_MAX)
                    break; /* §9.2: the limit is not exceeded */
                for (uint32_t i = line_len; i > cursor; --i)
                    line[i - 1u + n] = line[i - 1u];
                copy_bytes(line + cursor, bytes, n);
                line_len += n;
                out_n(line + cursor, n);
                cursor += n;
                redraw_tail(0);
            }
            break;
        default:
            break;
        }
    }
}

int main(void) {
    if (fnd_init() != FND_OK || !fnd_std_in() || !fnd_std_out() || !fnd_console())
        fnd_exit(FND_E_BAD_HANDLE);
    var_set("PATH", 4, "/bin", 4);
    /* §2.1: no startup block yet, so always interactive; /etc/profile.fsh
     * would need the fs service. */
    show_versions();
    for (;;) {
        out("/> "); /* §9.1: the cwd, which is / until there is an fs */
        if (read_line() != 0)
            continue; /* Ctrl+C: $? is unchanged */
        remember();
        run_line(line, line_len);
        if (exit_requested)
            fnd_exit(exit_code);
    }
}
