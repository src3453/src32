# TinyBASIC for CPT32, implemented in sol.
#
# Structured frontend: LEXER (source text -> token array) + PARSER
# (token array -> AST nodes) + INTERPRETER (tree-walk over the AST).
# The AST layer is also the extension point for a future compiler
# backend: tb_emit_program / tb_emit_stmt / tb_emit_expr walk the same
# nodes a code generator would consume (see CODEGEN HOOK section).
#
# Interactive PCG console; keyboard input comes from the PeC event FIFO.
# Program lines are kept as plain source text (LIST / LOAD / SAVE work on
# text), and RUN first builds a line directory of AST roots, then executes
# the AST. Direct (line-number-less) input is lexed/parsed/executed the
# same way, one line at a time.
#
# BASIC truth values are 1 (true) / 0 (false). Do not confuse them with
# the sol convention (comparison true = 0, `if`/`while` run on 0).
#
# Edge semantics kept from the previous interpreter: FOR executes its
# body at least once (the exit test lives in NEXT); LOAD/SAVE default to
# 128 sectors with LBA 0; LOAD clamps to the disk capacity; LOAD or NEW
# typed inside a running program stops it (the AST under execution would
# otherwise go stale); BYE/QUIT halt the CPU.
!include "sol/cpt32/pcg.sol"
!include "sol/cpt32/pec_rng.sol"
!include "sol/cpt32/irqc.sol"

!const TB_PEC_DATA 0x80040030
!const TB_PEC_STATUS 0x80040034
!const TB_PEC_POP 0x80040038
!const TB_IDC 0x80041000
!const TB_DMA 0x80050000
!const TB_DMA_CH0 0x80050100
!const TB_DISK_DATA 0x80041024
!const TB_LINE_BASE 0x00200000
!const TB_LINE_STRIDE 128
!const TB_ARRAY_BASE 0x00180000
!const TB_VAR_BASE 0x00170000
!const TB_GOSUB_BASE 0x00190000
!const TB_FOR_BASE 0x00191000
!const TB_INPUT_BASE 0x001B0000
!const TB_WORD_BASE 0x001B0100
!const TB_TEMP_BASE 0x001B0200
# Lexer output: token array, 8 bytes per entry (type, value).
!const TB_TOK_BUF 0x001B1000
!const TB_TOK_MAX 96
# Lexer scratch for the NUL-terminated bytes of one line's strings.
!const TB_STR_BUF 0x001B2000
# Line directory: 8 bytes per entry (lineNo, astRoot).
!const TB_LINE_DIR 0x001C0000
!const TB_DIR_MAX 1024
# AST heap: 20-byte nodes (kind, value, a, b, c).
!const TB_AST_BASE 0x00500000
!const TB_AST_LIMIT 0x00540000
# Persistent string pool for AST string literals (lexer scratch is per line).
!const TB_STR_POOL 0x00540000
!const TB_STR_POOL_LIMIT 0x00580000
!const TB_DISK_BASE 0x00400000
!const TB_MAX_LINE 9999
!const TB_MAX_TEXT 120
!const TB_ARRAY_SIZE 1000
# Token types. Single-character operators keep their ASCII codes.
!const TB_TOKEN_EOF 0
!const TB_TOKEN_NUMBER 1
!const TB_TOKEN_STRING 2
!const TB_TOKEN_IDENT 3
!const TB_TOKEN_AND 256
!const TB_TOKEN_OR 257
!const TB_TOKEN_NOT 258
!const TB_TOKEN_MOD 259
!const TB_TOKEN_ABS 260
!const TB_TOKEN_ASC 261
!const TB_TOKEN_RND 262
!const TB_TOKEN_SGN 263
!const TB_TOKEN_LE 264
!const TB_TOKEN_GE 265
!const TB_TOKEN_NE 266
!const TB_TOKEN_LET 267
!const TB_TOKEN_PRINT 268
!const TB_TOKEN_INPUT 269
!const TB_TOKEN_IF 270
!const TB_TOKEN_THEN 271
!const TB_TOKEN_GOTO 272
!const TB_TOKEN_GOSUB 273
!const TB_TOKEN_RETURN 274
!const TB_TOKEN_FOR 275
!const TB_TOKEN_TO 276
!const TB_TOKEN_NEXT 277
!const TB_TOKEN_RUN 278
!const TB_TOKEN_LIST 279
!const TB_TOKEN_NEW 280
!const TB_TOKEN_CLEAR 281
!const TB_TOKEN_CLS 282
!const TB_TOKEN_END 283
!const TB_TOKEN_STOP 284
!const TB_TOKEN_REM 285
!const TB_TOKEN_HELP 286
!const TB_TOKEN_QUIT 287
!const TB_TOKEN_BYE 288
!const TB_TOKEN_LOAD 289
!const TB_TOKEN_SAVE 290
!const TB_TOKEN_TRON 291
!const TB_TOKEN_TROFF 292
# AST node kinds (expressions).
!const TB_N_NUM 1
!const TB_N_STR 2
!const TB_N_VAR 3
!const TB_N_ARR 4
!const TB_N_BINOP 5
!const TB_N_UNOP 6
!const TB_N_CALL 7
# AST node kinds (statements and print items; c links same-line siblings).
!const TB_S_PRINT 11
!const TB_PI_STR 12
!const TB_PI_EXPR 13
!const TB_PI_SEP 14
!const TB_S_LET 16
!const TB_S_ARRLET 17
!const TB_S_INPUT 18
!const TB_S_GOTO 19
!const TB_S_GOSUB 20
!const TB_S_RETURN 21
!const TB_S_IF 22
!const TB_S_FOR 23
!const TB_S_NEXT 24
!const TB_S_SYS 25
# Binary operator codes.
!const TB_OP_ADD 1
!const TB_OP_SUB 2
!const TB_OP_MUL 3
!const TB_OP_DIV 4
!const TB_OP_IDIV 5
!const TB_OP_MOD 6
!const TB_OP_POW 7
!const TB_OP_EQ 8
!const TB_OP_LT 9
!const TB_OP_GT 10
!const TB_OP_LE 11
!const TB_OP_GE 12
!const TB_OP_NE 13
!const TB_OP_AND 14
!const TB_OP_OR 15
# Unary operator codes and builtin function ids.
!const TB_U_NEG 1
!const TB_U_NOT 2
!const TB_F_ABS 1
!const TB_F_SGN 2
!const TB_F_ASC 3
!const TB_F_RND 4
# System statement codes (RUN/LIST/.../BYE).
!const TB_SYS_RUN 1
!const TB_SYS_LIST 2
!const TB_SYS_NEW 3
!const TB_SYS_CLEAR 4
!const TB_SYS_CLS 5
!const TB_SYS_HELP 6
!const TB_SYS_LOAD 7
!const TB_SYS_SAVE 8
!const TB_SYS_TRON 9
!const TB_SYS_TROFF 10
!const TB_SYS_END 11
!const TB_SYS_BYE 12

# Lexer state.
!var tb_ntok 0
!var tb_str_used 0
# Parser state (token cursor).
!var ps_pos 0
# AST allocators.
!var tb_ast_bump 0
!var tb_str_pool 0
# Program: line directory (#entries), execution cursor, control flags.
!var tb_nlines 0
!var tb_pc 0
!var tb_cur 0
!var tb_stop 0
!var tb_jump 0
!var tb_line 0
!var tb_error 0
!var tb_trace 0
!var tb_gsp 0
!var tb_fsp 0
!var tb_shift 0
!var tb_caps 0
!var tb_disk_pos 0
!var tb_disk_limit 0

# ---------------- line text storage ----------------

fn tb_line_addr (line) :
    TB_LINE_BASE line TB_LINE_STRIDE mul add ret
;

fn tb_copy_line (source destination) :
    local i 0
    local ch 0
    while
        source i add ldb >ch
        ch 0 neq i TB_MAX_TEXT lt or
        if
            ch destination i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    0 destination i add stb
;

# Parse the leading line number of TB_INPUT_BASE and store/delete the
# remainder as program text. No lexer state is used here.
fn tb_store_input_line () :
    local line 0
    local pos 0
    local ch 0
    while
        TB_INPUT_BASE pos add ldb >ch
        ch 32 eq ch 9 eq and
        if pos 1 add >pos 0 else 1 end
    end
    ch 48 lt ch 57 gt and if retn end
    while
        TB_INPUT_BASE pos add ldb >ch
        ch 48 ge ch 57 le or
        if
            line 10 mul ch 48 sub add >line
            pos 1 add >pos
            0
        else
            1
        end
    end
    line 1 lt line TB_MAX_LINE gt and if
        "LINE RANGE" tb_print_string tb_newline
        retn
    end
    while
        TB_INPUT_BASE pos add ldb >ch
        ch 32 eq ch 9 eq and
        if pos 1 add >pos 0 else 1 end
    end
    ch 0 eq if
        0 line tb_line_addr stb
    else
        TB_INPUT_BASE pos add line tb_line_addr tb_copy_line
    end
;

fn tb_clear_program () :
    local line 1
    local addr 0
    while
        line TB_MAX_LINE le
        if
            line tb_line_addr >addr
            addr 0 stb
            line 1 add >line
            0
        else
            1
        end
    end
    0 >tb_nlines
;

fn tb_list_program () :
    local line 1
    local addr 0
    while
        line TB_MAX_LINE le
        if
            line tb_line_addr >addr
            addr ldb 0 neq
            if
                line tb_print_number
                32 tb_putc
                addr tb_print_string
                tb_newline
            end
            line 1 add >line
            0
        else
            1
        end
    end
;

fn tb_help () :
    "TinyBASIC: LET, PRINT, INPUT, IF/THEN, GOTO, GOSUB/RETURN," tb_print_string tb_newline
    "FOR/NEXT, RUN, LIST, NEW, CLEAR, LOAD, SAVE, REM, END" tb_print_string tb_newline
    "Expressions: + - * / \\ MOD ^, comparisons, AND/OR/NOT," tb_print_string tb_newline
    "ABS(), ASC(), RND(), SGN(), integer variables A-Z, @(n)." tb_print_string tb_newline
    "PRINT: , tab, ; no newline, #N right-justifies next number." tb_print_string tb_newline
    "LOAD/SAVE use slot 0: LBA and sector count; LOAD clamps sectors to image size." tb_print_string tb_newline
;

# ---------------- variables ----------------

fn tb_get_var (index) :
    TB_VAR_BASE index 2 shl add ld ret
;

fn tb_set_var (index value) :
    value TB_VAR_BASE index 2 shl add st
;

fn tb_array_store (index value) :
    index 0 lt index TB_ARRAY_SIZE ge and if 1 >tb_error retn end
    value TB_ARRAY_BASE index 2 shl add st
;

fn tb_clear_variables () :
    local i 0
    while
        i 26 lt
        if
            0 TB_VAR_BASE i 2 shl add st
            i 1 add >i
            0
        else
            1
        end
    end
    0 >i
    while
        i TB_ARRAY_SIZE lt
        if
            0 TB_ARRAY_BASE i 2 shl add st
            i 1 add >i
            0
        else
            1
        end
    end
;

fn tb_input_value () :
    local pos 0
    local sign 1
    local value 0
    local ch 0
    TB_INPUT_BASE ldb >ch
    ch 45 eq if
        -1 >sign
        1 >pos
    else
        ch 43 eq if 1 >pos end
    end
    while
        TB_INPUT_BASE pos add ldb >ch
        ch 48 ge ch 57 le or
        if
            value 10 mul ch 48 sub add >value
            pos 1 add >pos
            0
        else
            1
        end
    end
    pos 0 eq if
        TB_INPUT_BASE ldb >value
    end
    value sign mul ret
;

# ---------------- console output ----------------

fn tb_putc (char) :
    char 13 eq if retn end
    char 10 eq if pcg._pcg_ln retn end
    char 8 eq if
        pcg.pcg_cur_x 0 gt if
            pcg.pcg_cur_x 1 sub pcg.pcg_cur_y pcg.locate
            32 pcg.putcscr
            pcg.pcg_cur_x 1 sub pcg.pcg_cur_y pcg.locate
        end
        retn
    end
    char pcg.putcscr
;

fn tb_print_string (ptr) :
    local i 0
    local ch 0
    while
        ptr i add ldb >ch
        ch 0 neq
        if
            ch tb_putc
            i 1 add >i
            0
        else
            1
        end
    end
;

fn tb_print_number (number) :
    local value 0
    number >value
    value 0 lt if
        45 tb_putc
        value neg >value
    end
    value 10 ge if
        value 10 div tb_print_number
    end
    value 10 mod 48 add tb_putc
;

# Decimal digit count of a number, including a leading minus sign.
fn tb_number_width (number) :
    local value 0
    local digits 0
    number >value
    value 0 lt if
        value neg >value
        1 >digits
    end
    value 0 eq if 1 ret end
    while
        value 0 gt
        if
            digits 1 add >digits
            value 10 div >value
            0
        else
            1
        end
    end
    digits ret
;

# Print a number right-justified in a fixed-width field.
fn tb_print_padded (number width) :
    local pad 0
    local digits 0
    number tb_number_width >digits
    width digits sub >pad
    while
        pad 0 gt
        if
            32 tb_putc
            pad 1 sub >pad
            0
        else
            1
        end
    end
    number tb_print_number
;

fn tb_newline () :
    10 tb_putc
;

fn tb_report_error () :
    tb_line 0 eq if
        "? ERROR" tb_print_string tb_newline
    else
        "? ERROR IN LINE " tb_print_string
        tb_line tb_print_number
        tb_newline
    end
;

# ---------------- keyboard input ----------------

# IRQ3 is the PeC input event FIFO. Acknowledge the latched IRQC
# request so the HALT in tb_read_line wakes on the next event. The
# REPL drains the FIFO after IRET, which drops the source level;
# clearing IRQC pending here keeps the line low so the next event
# latches a fresh rising edge at the CPU input.
fn irq3 () :
    8 irqc.IRQC_PENDING sth
    retn
;

# Consume PeC keyboard events until Enter. HID shift/caps are tracked locally.
fn tb_read_line (dest capacity) :
    local count 0
    local event 0
    local usage 0
    local value 0
    local ch 0
    0 >tb_shift
    while
        TB_PEC_STATUS ld 1 and 0 gt
        if
            TB_PEC_DATA ld >event
            event 28 shr 15 and 0 eq if
                event 24 shr 15 and 0 eq if
                    event 16 shr 255 and >usage
                    event 1 and >value
                    usage 225 eq usage 229 eq and if
                        value >tb_shift
                    else
                        usage 57 eq value 1 eq or if
                            tb_caps 0 eq if 1 >tb_caps else 0 >tb_caps end
                        else
                            value 1 eq if
                                usage tb_shift tb_caps tb_key_ascii >ch
                                ch 13 eq if
                                    1 TB_PEC_POP st
                                    0 dest count add stb
                                    tb_newline
                                    count ret
                                end
                                ch 8 eq if
                                    count 0 gt if
                                        count 1 sub >count
                                        0 dest count add stb
                                        8 tb_putc
                                    end
                                else
                                    ch 0 neq count capacity lt or if
                                        ch dest count add stb
                                        count 1 add >count
                                        ch tb_putc
                                    end
                                end
                            end
                        end
                    end
                end
            end
            1 TB_PEC_POP st
        else
            # No event yet: stop the CPU until the PeC input
            # IRQ (IRQC source 3) wakes it. irq3 acknowledges
            # IRQC; draining the FIFO above drops the source
            # level so the next key latches a fresh edge.
            halt
        end
        0
    end
    count ret
;

# Return ASCII for supported US keyboard HID usage IDs (0 means ignored).
fn tb_key_ascii (usage shifted caps) :
    local ch 0
    local upper 0
    usage 4 ge usage 29 le or if
        usage 97 add 4 sub >ch
        shifted caps xor >upper
        upper 0 neq if ch 32 sub >ch end
        ch ret
    end
    usage 30 ge usage 39 le or if
        usage 30 eq if 49 >ch end
        usage 31 eq if 50 >ch end
        usage 32 eq if 51 >ch end
        usage 33 eq if 52 >ch end
        usage 34 eq if 53 >ch end
        usage 35 eq if 54 >ch end
        usage 36 eq if 55 >ch end
        usage 37 eq if 56 >ch end
        usage 38 eq if 57 >ch end
        usage 39 eq if 48 >ch end
        shifted 0 neq if
            ch 49 eq if 33 >ch end
            ch 50 eq if 64 >ch end
            ch 51 eq if 35 >ch end
            ch 52 eq if 36 >ch end
            ch 53 eq if 37 >ch end
            ch 54 eq if 94 >ch end
            ch 55 eq if 38 >ch end
            ch 56 eq if 42 >ch end
            ch 57 eq if 40 >ch end
            ch 48 eq if 41 >ch end
        end
        ch ret
    end
    usage 40 eq if 13 ret end
    usage 42 eq if 8 ret end
    usage 44 eq if 32 ret end
    usage 45 eq if shifted 0 eq if 45 ret else 95 ret end end
    usage 46 eq if shifted 0 eq if 61 ret else 43 ret end end
    usage 47 eq if shifted 0 eq if 91 ret else 123 ret end end
    usage 48 eq if shifted 0 eq if 93 ret else 125 ret end end
    usage 49 eq if shifted 0 eq if 92 ret else 124 ret end end
    usage 51 eq if shifted 0 eq if 59 ret else 58 ret end end
    usage 52 eq if shifted 0 eq if 39 ret else 34 ret end end
    usage 54 eq if shifted 0 eq if 44 ret else 60 ret end end
    usage 55 eq if shifted 0 eq if 46 ret else 62 ret end end
    usage 56 eq if shifted 0 eq if 47 ret else 63 ret end end
    usage 53 eq if shifted 0 eq if 96 ret else 126 ret end end
    0 ret
;

# ---------------- LEXER: text -> token array ----------------
# Tokens live in TB_TOK_BUF as (type, value) pairs; tb_ntok counts them
# including the trailing EOF token. String bytes of the current line go
# to TB_STR_BUF; the parser copies them into the persistent pool.

fn tb_lex_emit (type value) :
    local addr 0
    tb_ntok TB_TOK_MAX lt if
        TB_TOK_BUF tb_ntok 8 mul add >addr
        type addr st
        value addr 4 add st
        tb_ntok 1 add >tb_ntok
    else
        1 >tb_error
    end
;

# Return 0 when the current lowercase identifier equals the supplied string.
fn tb_word_is (text) :
    local i 0
    local a 0
    local b 0
    while
        TB_WORD_BASE i add ldb >a
        text i add ldb >b
        a b eq
        if
            a 0 eq
            if
                0 ret
            end
            i 1 add >i
            0
        else
            1
        end
    end
    1 ret
;

# Map the lowercase word in TB_WORD_BASE to a keyword token, or to an
# IDENT token whose value is the first-letter variable index.
fn tb_lex_word_token (ignored1 ignored2) :
    local code 0
    local index 0
    3 >code
    "and" tb_word_is 0 eq if 256 >code end
    "or" tb_word_is 0 eq if 257 >code end
    "not" tb_word_is 0 eq if 258 >code end
    "mod" tb_word_is 0 eq if 259 >code end
    "abs" tb_word_is 0 eq if 260 >code end
    "asc" tb_word_is 0 eq if 261 >code end
    "rnd" tb_word_is 0 eq if 262 >code end
    "irnd" tb_word_is 0 eq if 262 >code end
    "sgn" tb_word_is 0 eq if 263 >code end
    "let" tb_word_is 0 eq if 267 >code end
    "print" tb_word_is 0 eq if 268 >code end
    "input" tb_word_is 0 eq if 269 >code end
    "if" tb_word_is 0 eq if 270 >code end
    "then" tb_word_is 0 eq if 271 >code end
    "goto" tb_word_is 0 eq if 272 >code end
    "gosub" tb_word_is 0 eq if 273 >code end
    "return" tb_word_is 0 eq if 274 >code end
    "for" tb_word_is 0 eq if 275 >code end
    "to" tb_word_is 0 eq if 276 >code end
    "next" tb_word_is 0 eq if 277 >code end
    "run" tb_word_is 0 eq if 278 >code end
    "list" tb_word_is 0 eq if 279 >code end
    "new" tb_word_is 0 eq if 280 >code end
    "clear" tb_word_is 0 eq if 281 >code end
    "cls" tb_word_is 0 eq if 282 >code end
    "end" tb_word_is 0 eq if 283 >code end
    "stop" tb_word_is 0 eq if 284 >code end
    "rem" tb_word_is 0 eq if 285 >code end
    "help" tb_word_is 0 eq if 286 >code end
    "quit" tb_word_is 0 eq if 287 >code end
    "bye" tb_word_is 0 eq if 288 >code end
    "load" tb_word_is 0 eq if 289 >code end
    "save" tb_word_is 0 eq if 290 >code end
    "tron" tb_word_is 0 eq if 291 >code end
    "troff" tb_word_is 0 eq if 292 >code end
    code 3 eq if
        TB_WORD_BASE ldb 97 sub >index
        code index tb_lex_emit
    else
        code 0 tb_lex_emit
    end
;

# Lex one token at src+pos. Returns the position after the token, or -1
# when the line ends (NUL or '). Emits into TB_TOK_BUF.
fn tb_lex_step (src pos) :
    local ch 0
    local value 0
    local digit 0
    local start 0
    src pos add ldb >ch
    ch 32 eq ch 9 eq and if
        pos 1 add ret
    end
    ch 0 eq ch 39 eq and if
        0 0 tb_lex_emit
        -1 ret
    end
    ch 34 eq if
        TB_STR_BUF tb_str_used add >start
        pos 1 add >pos
        while
            src pos add ldb >ch
            ch 0 neq
            if
                ch 34 neq
                if
                    tb_str_used 4000 lt if
                        ch TB_STR_BUF tb_str_used add stb
                        tb_str_used 1 add >tb_str_used
                    end
                    pos 1 add >pos
                    0
                else
                    1
                end
            else
                1
            end
        end
        0 TB_STR_BUF tb_str_used add stb
        tb_str_used 1 add >tb_str_used
        2 start tb_lex_emit
        tb_error 0 neq if -1 ret end
        ch 34 eq if pos 1 add >pos end
        pos ret
    end
    ch 48 ge ch 57 le or if
        0 >value
        while
            src pos add ldb >ch
            ch 48 ge ch 57 le or
            if
                ch 48 sub >digit
                value 10 mul digit add >value
                pos 1 add >pos
                0
            else
                1
            end
        end
        1 value tb_lex_emit
        tb_error 0 neq if -1 ret end
        pos ret
    end
    ch 65 ge ch 90 le or
    ch 97 ge ch 122 le or and if
        0 >value
        while
            src pos add ldb >ch
            ch 65 ge ch 90 le or
            ch 97 ge ch 122 le or and
            ch 48 ge ch 57 le or and
            if
                ch 65 ge ch 90 le or if
                    ch 32 add >ch
                end
                value 120 lt if
                    ch TB_WORD_BASE value add stb
                    value 1 add >value
                end
                pos 1 add >pos
                0
            else
                1
            end
        end
        0 TB_WORD_BASE value add stb
        3 0 tb_lex_word_token
        tb_error 0 neq if -1 ret end
        pos ret
    end
    pos 1 add >pos
    ch 60 eq if
        src pos add ldb >value
        value 61 eq if
            264 0 tb_lex_emit
            pos 1 add ret
        end
        value 62 eq if
            266 0 tb_lex_emit
            pos 1 add ret
        end
        60 0 tb_lex_emit
        tb_error 0 neq if -1 ret end
        pos ret
    end
    ch 62 eq if
        src pos add ldb >value
        value 61 eq if
            265 0 tb_lex_emit
            pos 1 add ret
        end
        62 0 tb_lex_emit
        tb_error 0 neq if -1 ret end
        pos ret
    end
    ch 0 tb_lex_emit
    tb_error 0 neq if -1 ret end
    pos ret
;

fn tb_lex_line (src) :
    local pos 0
    0 >tb_ntok
    0 >tb_str_used
    0 >tb_error
    while
        src pos tb_lex_step >pos
        pos 0 ge tb_error 0 eq or
    end
;

# ---------------- AST heap ----------------
# Nodes are 20 bytes: +0 kind, +4 value, +8 a, +12 b, +16 c.

fn tb_ast_alloc (kind value a b c) :
    local ptr 0
    tb_ast_bump 20 add TB_AST_LIMIT gt if 1 >tb_error 0 ret end
    tb_ast_bump >ptr
    kind ptr st
    value ptr 4 add st
    a ptr 8 add st
    b ptr 12 add st
    c ptr 16 add st
    ptr 20 add >tb_ast_bump
    ptr ret
;

fn tb_ast_kind (node) :
    node ld ret
;

fn tb_ast_val (node) :
    node 4 add ld ret
;

fn tb_ast_a (node) :
    node 8 add ld ret
;

fn tb_ast_b (node) :
    node 12 add ld ret
;

fn tb_ast_c (node) :
    node 16 add ld ret
;

fn tb_ast_set_c (node next) :
    next node 16 add st
;

# Copy a NUL-terminated string into the persistent pool.
fn tb_pool_str (src) :
    local len 0
    local dest 0
    local i 0
    while
        src len add ldb 0 neq
        if
            len 1 add >len
            0
        else
            1
        end
    end
    tb_str_pool len add 1 add TB_STR_POOL_LIMIT gt if 1 >tb_error 0 ret end
    tb_str_pool >dest
    0 >i
    while
        i len le
        if
            src i add ldb dest i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    dest len add 1 add >tb_str_pool
    dest ret
;

# ---------------- PARSER: tokens -> AST ----------------

fn tb_tok_type (index) :
    TB_TOK_BUF index 8 mul add ld ret
;

fn tb_tok_val (index) :
    TB_TOK_BUF index 8 mul add 4 add ld ret
;

fn ps_peek () :
    ps_pos tb_tok_type ret
;

fn ps_peek_val () :
    ps_pos tb_tok_val ret
;

fn ps_advance () :
    ps_pos 1 add >ps_pos
;

# 0 when the current token matches (and is consumed), 1 otherwise.
fn ps_match (token) :
    ps_peek token eq if
        ps_advance
        0 ret
    end
    1 ret
;

fn ps_expect (token) :
    token ps_match 0 eq if 0 ret end
    1 >tb_error
    1 ret
;

fn tb_precedence (token) :
    token TB_TOKEN_OR eq if 1 ret end
    token TB_TOKEN_AND eq if 2 ret end
    token 61 eq if 4 ret end
    token 60 eq if 4 ret end
    token 62 eq if 4 ret end
    token 264 eq if 4 ret end
    token 265 eq if 4 ret end
    token 266 eq if 4 ret end
    token 43 eq if 5 ret end
    token 45 eq if 5 ret end
    token 42 eq if 6 ret end
    token 47 eq if 6 ret end
    token 92 eq if 6 ret end
    token TB_TOKEN_MOD eq if 6 ret end
    token 94 eq if 8 ret end
    0 ret
;

# Map an operator token to a TB_OP_* code (0 = not a binary operator).
fn tb_op_code (op) :
    op 43 eq if 1 ret end
    op 45 eq if 2 ret end
    op 42 eq if 3 ret end
    op 47 eq if 4 ret end
    op 92 eq if 5 ret end
    op TB_TOKEN_MOD eq if 6 ret end
    op 94 eq if 7 ret end
    op 61 eq if 8 ret end
    op 60 eq if 9 ret end
    op 62 eq if 10 ret end
    op 264 eq if 11 ret end
    op 265 eq if 12 ret end
    op 266 eq if 13 ret end
    op TB_TOKEN_AND eq if 14 ret end
    op TB_TOKEN_OR eq if 15 ret end
    0 ret
;

fn tb_make_binop (op lhs rhs) :
    local code 0
    op tb_op_code >code
    code 0 eq if 1 >tb_error 0 ret end
    lhs 0 eq rhs 0 eq and if 1 >tb_error 0 ret end
    5 code lhs rhs 0 tb_ast_alloc ret
;

# Parse one builtin call; kind is one of TB_TOKEN_ABS/SGN/ASC/RND.
# The keyword token is current on entry. Flat sequential ifs with early
# returns (no else): this shape is used instead of nested if/else.
fn ps_builtin (kind) :
    local arg 0
    kind TB_TOKEN_ABS eq if
        ps_advance
        40 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >arg
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        7 1 arg 0 0 tb_ast_alloc ret
    end
    kind TB_TOKEN_SGN eq if
        ps_advance
        40 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >arg
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        7 2 arg 0 0 tb_ast_alloc ret
    end
    kind TB_TOKEN_ASC eq if
        ps_advance
        40 ps_expect drop
        tb_error 0 neq if 0 ret end
        ps_peek 2 eq if
            ps_peek_val tb_pool_str >arg
            tb_error 0 neq if 0 ret end
            ps_advance
            2 arg 0 0 0 tb_ast_alloc >arg
            tb_error 0 neq if 0 ret end
            41 ps_expect drop
            tb_error 0 neq if 0 ret end
            7 3 arg 0 0 tb_ast_alloc ret
        end
        0 ps_expr >arg
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        7 3 arg 0 0 tb_ast_alloc ret
    end
    kind TB_TOKEN_RND eq if
        ps_advance
        40 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >arg
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        7 4 arg 0 0 tb_ast_alloc ret
    end
    1 >tb_error
    0 ret
;

# Primary forms other than builtins and plain variables (those stay in
# ps_expr so the builtin lookahead lives in one place).
fn ps_primary () :
    local value 0
    local index 0
    ps_peek 1 eq if
        ps_peek_val >value
        ps_advance
        1 value 0 0 0 tb_ast_alloc ret
    end
    ps_peek 2 eq if
        ps_peek_val tb_pool_str >value
        tb_error 0 neq if 0 ret end
        ps_advance
        2 value 0 0 0 tb_ast_alloc ret
    end
    ps_peek 45 eq if
        ps_advance
        7 ps_expr >value
        tb_error 0 neq if 0 ret end
        6 1 value 0 0 tb_ast_alloc ret
    end
    ps_peek 43 eq if
        ps_advance
        7 ps_expr ret
    end
    ps_peek TB_TOKEN_NOT eq if
        ps_advance
        3 ps_expr >value
        tb_error 0 neq if 0 ret end
        6 2 value 0 0 tb_ast_alloc ret
    end
    ps_peek 40 eq if
        ps_advance
        0 ps_expr >value
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        value ret
    end
    ps_peek 64 eq if
        ps_advance
        40 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >index
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        4 0 index 0 0 tb_ast_alloc ret
    end
    1 >tb_error
    ps_advance
    0 ret
;

# Precedence-climbing expression parser. BASIC comparisons yield 1/0.
fn ps_expr (minimum) :
    local lhs 0
    local rhs 0
    local op 0
    local prec 0
    ps_peek TB_TOKEN_ABS eq ps_peek TB_TOKEN_SGN eq and
    ps_peek TB_TOKEN_ASC eq and ps_peek TB_TOKEN_RND eq and if
        ps_peek >op
        op ps_builtin >lhs
    else
        ps_peek 3 eq if
            ps_peek_val >lhs
            ps_advance
            3 lhs 0 0 0 tb_ast_alloc >lhs
        else
            ps_primary >lhs
        end
    end
    tb_error 0 neq if 0 ret end
    lhs 0 eq if 1 >tb_error 0 ret end
    while
        ps_peek tb_precedence >prec
        prec 0 gt prec minimum ge or if
            ps_peek >op
            ps_advance
            prec 8 eq if
                prec ps_expr >rhs
            else
                prec 1 add ps_expr >rhs
            end
            tb_error 0 neq if
                1
            else
                op lhs rhs tb_make_binop >lhs
                0
            end
        else
            1
        end
    end
    lhs ret
;

# Skip tokens up to (not including) EOF; used for REM.
fn ps_skip_rest () :
    while
        ps_peek 0 eq if 1 else ps_advance 0 end
    end
;

# PRINT item list (after PRINT/?); returns the item head (0 = bare PRINT).
fn ps_print_items () :
    local head 0
    local tail 0
    local node 0
    local expr 0
    local width 0
    ps_advance
    while
        ps_peek 0 eq ps_peek 58 eq and if
            1
        else
            ps_peek 2 eq if
                ps_peek_val tb_pool_str >node
                tb_error 0 neq if 0 ret end
                ps_advance
                12 node 0 0 0 tb_ast_alloc >node
                tb_error 0 neq if 0 ret end
                head 0 eq if node >head node >tail else tail node tb_ast_set_c node >tail end
                0
            else
                ps_peek 44 eq ps_peek 59 eq and if
                    ps_peek >node
                    ps_advance
                    14 node 0 0 0 tb_ast_alloc >node
                    tb_error 0 neq if 0 ret end
                    head 0 eq if node >head node >tail else tail node tb_ast_set_c node >tail end
                    0
                else
                    ps_peek 35 eq if
                        ps_advance
                        ps_peek 1 neq if 1 >tb_error 0 ret end
                        ps_peek_val >width
                        ps_advance
                        ps_peek 44 eq ps_peek 59 eq and if ps_advance end
                        0 ps_expr >expr
                        tb_error 0 neq if 0 ret end
                        13 width expr 0 0 tb_ast_alloc >node
                        tb_error 0 neq if 0 ret end
                        head 0 eq if node >head node >tail else tail node tb_ast_set_c node >tail end
                        0
                    else
                        0 ps_expr >expr
                        tb_error 0 neq if 0 ret end
                        13 0 expr 0 0 tb_ast_alloc >node
                        tb_error 0 neq if 0 ret end
                        head 0 eq if node >head node >tail else tail node tb_ast_set_c node >tail end
                        0
                    end
                end
            end
        end
    end
    head ret
;

# LOAD/SAVE with optional (lba, sectors) expressions; other SYS words take
# no arguments. The SYS keyword is current on entry.
fn ps_parse_sys (code) :
    local lba 0
    local secs 0
    code TB_SYS_LOAD eq code TB_SYS_SAVE eq and if
        ps_advance
        ps_peek 0 eq ps_peek 58 eq and if
            25 code 0 0 0 tb_ast_alloc ret
        end
        0 ps_expr >lba
        tb_error 0 neq if 0 ret end
        ps_peek 44 eq ps_peek 59 eq and if ps_advance end
        ps_peek 0 eq ps_peek 58 eq and if
            25 code lba 0 0 tb_ast_alloc ret
        end
        0 ps_expr >secs
        tb_error 0 neq if 0 ret end
        25 code lba secs 0 tb_ast_alloc ret
    end
    ps_advance
    25 code 0 0 0 tb_ast_alloc ret
;

# Parse one statement; returns the stmt node (0 for empty/REM).
fn ps_statement () :
    local index 0
    local expr 0
    local limit 0
    local cond 0
    local line 0
    local then 0
    local prompt 0
    ps_peek 285 eq if
        ps_advance
        ps_skip_rest
        0 ret
    end
    ps_peek 268 eq ps_peek 63 eq and if
        ps_print_items >expr
        tb_error 0 neq if 0 ret end
        11 0 expr 0 0 tb_ast_alloc ret
    end
    ps_peek 269 eq if
        ps_advance
        0 >prompt
        ps_peek 2 eq if
            ps_peek_val tb_pool_str >prompt
            tb_error 0 neq if 0 ret end
            ps_advance
            ps_peek 44 eq if ps_advance end
        end
        ps_peek 3 eq if
            ps_peek_val >index
            ps_advance
            18 index prompt 0 0 tb_ast_alloc ret
        else
            1 >tb_error
            0 ret
        end
    end
    ps_peek 267 eq if
        ps_advance
        ps_peek 3 eq if
            ps_peek_val >index
            ps_advance
            61 ps_expect drop
            tb_error 0 neq if 0 ret end
            0 ps_expr >expr
            tb_error 0 neq if 0 ret end
            16 index expr 0 0 tb_ast_alloc ret
        else
            1 >tb_error
            0 ret
        end
    end
    ps_peek 3 eq if
        ps_peek_val >index
        ps_advance
        61 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >expr
        tb_error 0 neq if 0 ret end
        16 index expr 0 0 tb_ast_alloc ret
    end
    ps_peek 64 eq if
        ps_advance
        40 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >index
        tb_error 0 neq if 0 ret end
        41 ps_expect drop
        tb_error 0 neq if 0 ret end
        61 ps_expect drop
        tb_error 0 neq if 0 ret end
        0 ps_expr >expr
        tb_error 0 neq if 0 ret end
        17 0 index expr 0 tb_ast_alloc ret
    end
    ps_peek 272 eq if
        ps_advance
        0 ps_expr >expr
        tb_error 0 neq if 0 ret end
        19 0 expr 0 0 tb_ast_alloc ret
    end
    ps_peek 273 eq if
        ps_advance
        0 ps_expr >expr
        tb_error 0 neq if 0 ret end
        20 0 expr 0 0 tb_ast_alloc ret
    end
    ps_peek 274 eq if
        ps_advance
        21 0 0 0 0 tb_ast_alloc ret
    end
    ps_peek 270 eq if
        ps_advance
        0 ps_expr >cond
        tb_error 0 neq if 0 ret end
        ps_peek 271 eq if ps_advance end
        ps_peek 1 eq if
            ps_peek_val >line
            ps_advance
            22 line cond 0 0 tb_ast_alloc ret
        else
            ps_stmt_list >then
            tb_error 0 neq if 0 ret end
            22 0 cond then 0 tb_ast_alloc ret
        end
    end
    ps_peek 275 eq if
        ps_advance
        ps_peek 3 eq if
            ps_peek_val >index
            ps_advance
            61 ps_expect drop
            tb_error 0 neq if 0 ret end
            0 ps_expr >expr
            tb_error 0 neq if 0 ret end
            276 ps_expect drop
            tb_error 0 neq if 0 ret end
            0 ps_expr >limit
            tb_error 0 neq if 0 ret end
            23 index expr limit 0 tb_ast_alloc ret
        else
            1 >tb_error
            0 ret
        end
    end
    ps_peek 277 eq if
        ps_advance
        -1 >index
        ps_peek 3 eq if
            ps_peek_val >index
            ps_advance
        end
        24 index 0 0 0 tb_ast_alloc ret
    end
    ps_peek 283 eq ps_peek 284 eq and if
        ps_advance
        25 11 0 0 0 tb_ast_alloc ret
    end
    ps_peek 278 eq if 1 ps_parse_sys ret end
    ps_peek 279 eq if 2 ps_parse_sys ret end
    ps_peek 280 eq if 3 ps_parse_sys ret end
    ps_peek 281 eq if 4 ps_parse_sys ret end
    ps_peek 282 eq if 5 ps_parse_sys ret end
    ps_peek 286 eq if 6 ps_parse_sys ret end
    ps_peek 289 eq if 7 ps_parse_sys ret end
    ps_peek 290 eq if 8 ps_parse_sys ret end
    ps_peek 291 eq if 9 ps_parse_sys ret end
    ps_peek 292 eq if 10 ps_parse_sys ret end
    ps_peek 287 eq ps_peek 288 eq and if
        12 ps_parse_sys ret
    end
    ps_peek 0 eq ps_peek 58 eq and if
        0 ret
    end
    1 >tb_error
    0 ret
;

# Parse a ':'-separated statement list; returns the head node (0 = empty).
fn ps_stmt_list () :
    local head 0
    local tail 0
    local st 0
    while
        ps_peek 58 eq if
            ps_advance
            0
        else
            ps_peek 0 eq if
                1
            else
                ps_statement >st
                tb_error 0 neq if 0 ret end
                st 0 neq if
                    head 0 eq if st >head st >tail else tail st tb_ast_set_c st >tail end
                end
                0
            end
        end
    end
    head ret
;

# Parse the lexed current line; returns the statement-list head.
fn tb_parse_line () :
    local head 0
    0 >ps_pos
    ps_stmt_list >head
    tb_error 0 neq if 0 ret end
    0 ps_expect drop
    tb_error 0 neq if 0 ret end
    head ret
;

# ---------------- PROGRAM: text lines -> AST directory ----------------

fn tb_dir_line (index) :
    TB_LINE_DIR index 8 mul add ld ret
;

fn tb_dir_root (index) :
    TB_LINE_DIR index 8 mul add 4 add ld ret
;

fn tb_find_dir (line) :
    local i 0
    while
        i tb_nlines lt
        if
            i tb_dir_line line eq if i ret end
            i 1 add >i
            0
        else
            1
        end
    end
    -1 ret
;

# Build the AST directory from the stored text lines. On failure tb_error
# is set and tb_line holds the offending line number.
fn tb_build_program () :
    local line 1
    local addr 0
    local head 0
    0 >tb_nlines
    TB_AST_BASE >tb_ast_bump
    TB_STR_POOL >tb_str_pool
    0 >tb_error
    while
        line TB_MAX_LINE le tb_error 0 eq or if
            line tb_line_addr >addr
            addr ldb 0 neq if
                0 >tb_error
                addr tb_lex_line
                tb_error 0 eq if
                    tb_parse_line >head
                end
                tb_error 0 neq if
                    line >tb_line
                else
                    tb_nlines TB_DIR_MAX lt if
                        line TB_LINE_DIR tb_nlines 8 mul add st
                        head TB_LINE_DIR tb_nlines 8 mul add 4 add st
                        tb_nlines 1 add >tb_nlines
                    else
                        1 >tb_error
                        line >tb_line
                    end
                end
            end
            line 1 add >line
            0
        else
            1
        end
    end
;

fn tb_goto_line (line) :
    local index 0
    line tb_find_dir >index
    index 0 lt if
        1 >tb_error
        retn
    end
    line >tb_line
    index >tb_pc
    index tb_dir_root >tb_cur
    1 >tb_jump
;

# ---------------- EVALUATOR: AST -> integer ----------------
# BASIC comparisons yield 1 (true) / 0 (false).

fn tb_power (base exponent) :
    local result 1
    local factor 0
    local power 0
    base >factor
    exponent >power
    power 0 lt if 0 ret end
    while
        power 0 gt
        if
            power 1 and 0 neq if result factor mul >result end
            factor factor mul >factor
            power 1 shr >power
            0
        else
            1
        end
    end
    result ret
;

fn tb_eval_binop (code lhs rhs) :
    code 1 eq if lhs rhs add ret end
    code 2 eq if lhs rhs sub ret end
    code 3 eq if lhs rhs mul ret end
    code 4 eq code 5 eq and if
        rhs 0 eq if 1 >tb_error 0 ret end
        lhs rhs div ret
    end
    code 6 eq if
        rhs 0 eq if 1 >tb_error 0 ret end
        lhs rhs mod ret
    end
    code 7 eq if lhs rhs tb_power ret end
    code 8 eq if lhs rhs eq if 1 ret else 0 ret end end
    code 9 eq if lhs rhs lt if 1 ret else 0 ret end end
    code 10 eq if lhs rhs gt if 1 ret else 0 ret end end
    code 11 eq if lhs rhs le if 1 ret else 0 ret end end
    code 12 eq if lhs rhs ge if 1 ret else 0 ret end end
    code 13 eq if lhs rhs neq if 1 ret else 0 ret end end
    code 14 eq if lhs rhs and ret end
    code 15 eq if lhs rhs or ret end
    1 >tb_error
    0 ret
;

fn tb_eval_call (func argnode) :
    local value 0
    func 3 eq if
        argnode tb_ast_kind 2 eq if
            argnode tb_ast_val ldb ret
        end
        argnode tb_eval >value
        value ret
    end
    argnode tb_eval >value
    tb_error 0 neq if 0 ret end
    func 1 eq if
        value 0 lt if value neg >value end
        value ret
    end
    func 2 eq if
        value 0 gt if 1 ret end
        value 0 lt if -1 ret end
        0 ret
    end
    value 0 le if 0 ret end
    pec_rng.rand 0x7fffffff and value mod 1 add ret
;

fn tb_eval (node) :
    local kind 0
    local value 0
    local left 0
    local right 0
    node 0 eq if 1 >tb_error 0 ret end
    node tb_ast_kind >kind
    kind 1 eq if node tb_ast_val ret end
    kind 2 eq if node tb_ast_val ldb ret end
    kind 3 eq if node tb_ast_val tb_get_var ret end
    kind 4 eq if
        node tb_ast_a tb_eval >value
        tb_error 0 neq if 0 ret end
        value 0 lt value TB_ARRAY_SIZE ge and if 1 >tb_error 0 ret end
        TB_ARRAY_BASE value 2 shl add ld ret
    end
    kind 5 eq if
        node tb_ast_a tb_eval >left
        tb_error 0 neq if 0 ret end
        node tb_ast_b tb_eval >right
        tb_error 0 neq if 0 ret end
        node tb_ast_val left right tb_eval_binop ret
    end
    kind 6 eq if
        node tb_ast_a tb_eval >left
        tb_error 0 neq if 0 ret end
        node tb_ast_val 1 eq if left neg ret end
        left 0 eq if 1 ret else 0 ret end
    end
    kind 7 eq if
        node tb_ast_val node tb_ast_a tb_eval_call ret
    end
    1 >tb_error
    0 ret
;

# ---------------- EXECUTOR: AST statements ----------------

fn tb_exec_print (items) :
    local p 0
    local kind 0
    local value 0
    local width 0
    local nl 1
    items >p
    while
        p 0 eq if 1 else
            p tb_ast_kind >kind
            kind 12 eq if
                p tb_ast_val tb_print_string
                1 >nl
            end
            kind 13 eq if
                p tb_ast_a tb_eval >value
                tb_error 0 neq if 1 ret end
                p tb_ast_val >width
                width 0 eq if
                    value tb_print_number
                else
                    value width tb_print_padded
                end
                1 >nl
            end
            kind 14 eq if
                0 >nl
            end
            p tb_ast_c >p
            0
        end
    end
    nl 0 neq if tb_newline end
;

# Execute a detached statement chain (IF/THEN branch). Jumps set tb_jump
# and take over tb_pc/tb_cur; the caller must stop on tb_jump.
fn tb_exec_list (head) :
    local p 0
    local next 0
    head >p
    while
        p 0 eq tb_stop 0 neq and tb_error 0 neq and tb_jump 0 neq and if 1 else
            p tb_ast_c >next
            p tb_exec_one
            next >p
            0
        end
    end
;

fn tb_exec_one (node) :
    local kind 0
    local value 0
    local a 0
    local b 0
    local frame 0
    node tb_ast_kind >kind
    node tb_ast_val >value
    node tb_ast_a >a
    node tb_ast_b >b
    kind 11 eq if a tb_exec_print retn end
    kind 16 eq if
        a tb_eval >value
        tb_error 0 neq if retn end
        node tb_ast_val value tb_set_var
        retn
    end
    kind 17 eq if
        a tb_eval >value
        tb_error 0 neq if retn end
        b tb_eval >a
        tb_error 0 neq if retn end
        value a tb_array_store
        retn
    end
    kind 18 eq if
        a 0 neq if a tb_print_string else "? " tb_print_string end
        TB_INPUT_BASE 120 tb_read_line drop
        node tb_ast_val tb_input_value tb_set_var
        retn
    end
    kind 19 eq if
        a tb_eval >value
        tb_error 0 neq if retn end
        value tb_goto_line
        retn
    end
    kind 20 eq if
        tb_gsp 64 ge if 1 >tb_error retn end
        TB_GOSUB_BASE tb_gsp 8 mul add >frame
        tb_pc frame st
        tb_cur frame 4 add st
        tb_gsp 1 add >tb_gsp
        a tb_eval >value
        tb_error 0 neq if
            tb_gsp 1 sub >tb_gsp
            retn
        end
        value tb_goto_line
        retn
    end
    kind 21 eq if
        tb_gsp 0 le if 1 >tb_error retn end
        tb_gsp 1 sub >tb_gsp
        TB_GOSUB_BASE tb_gsp 8 mul add >frame
        frame ld >tb_pc
        frame 4 add ld >tb_cur
        tb_pc 0 ge if
            tb_pc tb_dir_line >tb_line
        else
            0 >tb_line
        end
        1 >tb_jump
        retn
    end
    kind 22 eq if
        a tb_eval >value
        tb_error 0 neq if retn end
        value 0 neq if
            node tb_ast_val 0 neq if
                node tb_ast_val tb_goto_line
            else
                b tb_exec_list
            end
        end
        retn
    end
    kind 23 eq if
        a tb_eval >value
        tb_error 0 neq if retn end
        b tb_eval >a
        tb_error 0 neq if retn end
        node tb_ast_val value tb_set_var
        tb_fsp 32 ge if 1 >tb_error retn end
        TB_FOR_BASE tb_fsp 16 mul add >frame
        node tb_ast_val frame st
        a frame 4 add st
        tb_pc frame 8 add st
        tb_cur frame 12 add st
        tb_fsp 1 add >tb_fsp
        retn
    end
    kind 24 eq if
        tb_fsp 0 le if 1 >tb_error retn end
        TB_FOR_BASE tb_fsp 1 sub 16 mul add >frame
        frame ld >b
        value -1 neq value b neq or if 1 >tb_error retn end
        b tb_get_var 1 add >value
        b value tb_set_var
        value frame 4 add ld le if
            frame 8 add ld >tb_pc
            frame 12 add ld >tb_cur
            tb_pc 0 ge if
                tb_pc tb_dir_line >tb_line
            else
                0 >tb_line
            end
            1 >tb_jump
        else
            tb_fsp 1 sub >tb_fsp
        end
        retn
    end
    kind 25 eq if
        value TB_SYS_LOAD eq value TB_SYS_SAVE eq and if
            value a b tb_exec_disk
        else
            value tb_exec_sys
        end
        retn
    end
    1 >tb_error
;

fn tb_exec_sys (code) :
    code 1 eq if tb_sys_run retn end
    code 2 eq if tb_list_program retn end
    code 3 eq if
        tb_clear_program
        tb_clear_variables
        tb_pc 0 ge if 1 >tb_stop end
        retn
    end
    code 4 eq if tb_clear_variables retn end
    code 5 eq if tb_cls retn end
    code 6 eq if tb_help retn end
    code 9 eq if 1 >tb_trace retn end
    code 10 eq if 0 >tb_trace retn end
    code 11 eq if 1 >tb_stop retn end
    code 12 eq if halt retn end
    1 >tb_error
;

# LOAD/SAVE carry optional (lbaNode, secNode) in a/b.
fn tb_exec_disk (cmd lbaNode secNode) :
    local lba 0
    local sectors 0
    local result 0
    local capacity 0
    local available 0
    lbaNode 0 neq if
        lbaNode tb_eval >lba
        tb_error 0 neq if retn end
    end
    secNode 0 neq if
        secNode tb_eval >sectors
        tb_error 0 neq if retn end
    else
        128 >sectors
    end
    cmd TB_SYS_LOAD eq if
        tb_disk_capacity >capacity
        lba capacity lt if
            capacity lba sub >available
            sectors available gt if
                available >sectors
            end
            lba sectors 1 tb_disk_transfer >result
            result 0 eq if
                sectors 512 mul tb_load_disk_lines
                "LOADED" tb_print_string tb_newline
            else
                "DISK READ ERROR" tb_print_string tb_newline
            end
        else
            "DISK READ ERROR" tb_print_string tb_newline
        end
    else
        lbaNode 0 eq secNode 0 eq and if
            "SAVE NEEDS LBA AND SECTORS" tb_print_string tb_newline
        else
            sectors 512 mul >tb_disk_limit
            tb_save_serialize
            tb_error 0 eq if
                lba sectors 2 tb_disk_transfer >result
                result 0 eq if
                    "SAVED" tb_print_string tb_newline
                else
                    "DISK WRITE ERROR" tb_print_string tb_newline
                end
            else
                "SAVE BUFFER FULL" tb_print_string tb_newline
            end
        end
    end
    tb_pc 0 ge if
        cmd TB_SYS_LOAD eq if 1 >tb_stop end
    end
;

# ---------------- CODEGEN HOOK (future compiler backend) ----------------
# These walk the same AST the interpreter executes and count nodes.
# A real backend would emit code at each visit; the traversal order here
# (expression post-order, statements in line order) is the contract it
# should keep.

fn tb_emit_expr (node count) :
    local kind 0
    node 0 eq if count ret end
    node tb_ast_kind >kind
    count 1 add >count
    kind 5 eq kind 6 eq and if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_expr >count
    else
        kind 7 eq kind 4 eq and if
            node tb_ast_a count tb_emit_expr >count
        end
    end
    count ret
;

fn tb_emit_items (items count) :
    local p 0
    items >p
    while
        p 0 eq if 1 else
            count 1 add >count
            p tb_ast_kind 13 eq if
                p tb_ast_a count tb_emit_expr >count
            end
            p tb_ast_c >p
            0
        end
    end
    count ret
;

# Single statement without following siblings (used for THEN branches).
fn tb_emit_single (node count) :
    local kind 0
    node tb_ast_kind >kind
    count 1 add >count
    kind 11 eq if node tb_ast_a count tb_emit_items >count end
    kind 16 eq if node tb_ast_a count tb_emit_expr >count end
    kind 17 eq if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_expr >count
    end
    kind 19 eq kind 20 eq and if node tb_ast_a count tb_emit_expr >count end
    kind 22 eq if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_chain >count
    end
    kind 23 eq if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_expr >count
    end
    count ret
;

fn tb_emit_chain (head count) :
    local p 0
    head >p
    while
        p 0 eq if 1 else
            p count tb_emit_single >count
            p tb_ast_c >p
            0
        end
    end
    count ret
;

fn tb_emit_stmt (node count) :
    local kind 0
    node 0 eq if count ret end
    node tb_ast_kind >kind
    count 1 add >count
    kind 11 eq if node tb_ast_a count tb_emit_items >count end
    kind 16 eq if node tb_ast_a count tb_emit_expr >count end
    kind 17 eq if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_expr >count
    end
    kind 19 eq kind 20 eq and if node tb_ast_a count tb_emit_expr >count end
    kind 22 eq if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_chain >count
    end
    kind 23 eq if
        node tb_ast_a count tb_emit_expr >count
        node tb_ast_b count tb_emit_expr >count
    end
    node tb_ast_c count tb_emit_stmt >count
    count ret
;

fn tb_emit_program () :
    local i 0
    local count 0
    0 >count
    0 >i
    while
        i tb_nlines lt
        if
            i tb_dir_root count tb_emit_stmt >count
            i 1 add >i
            0
        else
            1
        end
    end
    count ret
;

# ---------------- main execution loop ----------------
# One loop serves direct mode (tb_pc = -1, tb_cur = direct chain) and
# program mode (tb_pc = directory index). Jumps move the cursor; when it
# runs off the direct chain the loop ends, when it runs off the program
# it stops.

fn tb_run_loop () :
    local stmt 0
    while
        tb_stop 0 neq tb_error 0 neq and if 1 else
            tb_cur 0 eq if
                tb_pc 0 lt if 1 else
                    tb_pc 1 add >tb_pc
                    tb_pc tb_nlines ge if
                        1 >tb_stop
                        1
                    else
                        tb_pc tb_dir_line >tb_line
                        tb_pc tb_dir_root >tb_cur
                        tb_trace 0 neq if
                            tb_line tb_print_number
                            32 tb_putc
                            tb_line tb_line_addr tb_print_string
                            tb_newline
                        end
                        0
                    end
                end
            else
                tb_cur >stmt
                tb_cur tb_ast_c >tb_cur
                0 >tb_jump
                stmt tb_exec_one
                0
            end
        end
    end
    tb_error 0 neq if
        tb_report_error
        0 >tb_error
    end
;

fn tb_sys_run () :
    tb_clear_variables
    0 >tb_gsp
    0 >tb_fsp
    0 >tb_stop
    0 >tb_error
    tb_build_program
    tb_error 0 neq if retn end
    0 >tb_pc
    tb_nlines 0 gt if
        0 tb_dir_line >tb_line
        0 tb_dir_root >tb_cur
    else
        0 >tb_cur
        0 >tb_line
    end
    1 >tb_jump
;

fn tb_run_program () :
    tb_sys_run
    tb_run_loop
;

fn tb_direct_line () :
    local head 0
    TB_INPUT_BASE tb_lex_line
    tb_ntok 0 gt if
        0 tb_tok_type 1 eq if
            0 >tb_error
            tb_store_input_line
            retn
        end
    end
    tb_error 0 neq if
        "? ERROR" tb_print_string tb_newline
        0 >tb_error
        retn
    end
    tb_parse_line >head
    tb_error 0 neq if
        "? ERROR" tb_print_string tb_newline
        0 >tb_error
        retn
    end
    -1 >tb_pc
    head >tb_cur
    0 >tb_line
    0 >tb_stop
    tb_run_loop
;

fn tb_cls () :
    local i 0
    0 >pcg.pcg_scroll_y
    0 >pcg.pcg_vram_row
    0 0x8000f00e stb
    0 0x8000f00f stb
    while
        i 1200 lt
        if
            32 0x10000000 i add stb
            63 0x10001000 i add stb
            0 0x10002000 i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    0 0 pcg.locate
;

fn tb_repl () :
    tb_cls
    tb_help
    TB_AST_BASE >tb_ast_bump
    TB_STR_POOL >tb_str_pool
    while
        "OK" tb_print_string tb_newline
        "> " tb_print_string
        TB_INPUT_BASE 120 tb_read_line drop
        tb_direct_line
        0
    end
;

# ---------------- disk ----------------

fn tb_disk_transfer (lba sectors command) :
    local bytes 0
    local dma_control 0
    local status 0
    sectors 1 lt sectors 256 gt and if 1 ret end
    sectors 512 mul >bytes
    while
        TB_IDC 8 add ld 1 and 0 neq
        if 0 else 1 end
    end
    while
        TB_DMA_CH0 16 add ld 1 and 0 neq
        if 0 else 1 end
    end
    6 TB_IDC 8 add st
    6 TB_DMA_CH0 16 add st
    1 TB_DMA 4 add st
    1 TB_DMA 8 add st
    0 TB_IDC 20 add st
    lba TB_IDC 28 add st
    sectors TB_IDC 32 add st
    command 1 eq if
        TB_DISK_DATA TB_DMA_CH0 st
        TB_DISK_BASE TB_DMA_CH0 4 add st
        0x104 >dma_control
    else
        TB_DISK_BASE TB_DMA_CH0 st
        TB_DISK_DATA TB_DMA_CH0 4 add st
        0x110 >dma_control
    end
    bytes TB_DMA_CH0 8 add st
    command TB_IDC 24 add st
    dma_control TB_DMA_CH0 12 add st
    while
        TB_DMA_CH0 16 add ld 1 and 0 neq
        if 0 else 1 end
    end
    while
        TB_IDC 8 add ld 1 and 0 neq
        if 0 else 1 end
    end
    TB_DMA_CH0 16 add ld >status
    status 4 and 0 neq if 1 ret end
    TB_IDC 8 add ld 4 and 0 neq if 1 ret end
    0 ret
;

fn tb_disk_capacity () :
    local status 0
    while
        TB_IDC 8 add ld 1 and 0 neq
        if 0 else 1 end
    end
    6 TB_IDC 8 add st
    0 TB_IDC 20 add st
    3 TB_IDC 24 add st
    while
        TB_IDC 8 add ld 1 and 0 neq
        if 0 else 1 end
    end
    TB_IDC 8 add ld >status
    status 4 and 0 neq if 0 ret end
    TB_IDC 48 add ld
    ret
;

fn tb_buffer_byte (byte) :
    tb_disk_pos tb_disk_limit lt if
        byte TB_DISK_BASE tb_disk_pos add stb
        tb_disk_pos 1 add >tb_disk_pos
    else
        1 >tb_error
    end
;

fn tb_buffer_number (number) :
    local value 0
    local digit 0
    number >value
    value 1000 ge if
        value 1000 div >digit
        digit 48 add tb_buffer_byte
        value 1000 mod >value
    end
    value 100 ge if
        value 100 div >digit
        digit 48 add tb_buffer_byte
        value 100 mod >value
    end
    value 10 ge if
        value 10 div >digit
        digit 48 add tb_buffer_byte
        value 10 mod >value
    end
    value 48 add tb_buffer_byte
;

fn tb_save_serialize () :
    local line 1
    local addr 0
    local pos 0
    local ch 0
    0 >tb_disk_pos
    0 >tb_error
    while
        line TB_MAX_LINE le tb_error 0 eq or
        if
            line tb_line_addr >addr
            addr ldb 0 neq if
                line tb_buffer_number
                32 tb_buffer_byte
                0 >pos
                while
                    addr pos add ldb >ch
                    ch 0 neq tb_disk_pos tb_disk_limit lt or
                    if
                        ch tb_buffer_byte
                        pos 1 add >pos
                        0
                    else
                        1
                    end
                end
                13 tb_buffer_byte
                10 tb_buffer_byte
            end
            line 1 add >line
            0
        else
            1
        end
    end
    while
        tb_disk_pos tb_disk_limit lt
        if
            0 tb_buffer_byte
            0
        else
            1
        end
    end
;

# Finalize a streamed LOAD line when it ends (newline or end of image).
# The scan already parsed the number (state 1) and wrote the body straight
# into the line slot (state 3), so this only has to cover a number that ran
# to the line end with no body (state 1: validate then delete), a number
# followed by whitespace only (state 2: delete), or a written body (state 3:
# NUL-terminate). States 0/4 do nothing (no number seen, or line discarded).
fn tb_finish_load_line (state line addr idx) :
    state 1 eq if
        line 1 lt line TB_MAX_LINE gt and if
            "LINE RANGE" tb_print_string tb_newline
        else
            0 line tb_line_addr stb
        end
    end
    state 2 eq if
        0 addr stb
    end
    state 3 eq if
        0 addr idx add stb
    end
;

# Stream the disk image straight into program memory in one pass, replacing
# the old buffer-into-TB_INPUT_BASE + re-scan (tb_store_input_line) +
# re-copy (tb_copy_line) path. count mirrors the old buffer cap so a long
# line still truncates at TB_MAX_TEXT (line slots are TB_LINE_STRIDE bytes,
# so the cap also keeps a body from overflowing the slot).
#   state 0 = seeking the line number, 1 = parsing digits, 2 = skipping
#   whitespace after the number, 3 = writing the body, 4 = discarding.
fn tb_load_disk_lines (bytes) :
    local pos 0
    local ch 0
    local line 0
    local addr 0
    local idx 0
    local state 0
    local count 0
    tb_clear_program
    while
        pos bytes lt
        if
            TB_DISK_BASE pos add ldb >ch
            pos 1 add >pos
            ch 0 eq
            if
                1
            else
                ch 10 eq ch 13 eq and
                if
                    state 0 eq state 4 eq and if
                    else
                        state line addr idx tb_finish_load_line
                    end
                    0 >state
                    0 >line
                    0 >idx
                    0 >count
                    0
                else
                    count TB_MAX_TEXT lt if
                        state 3 eq if
                            ch addr idx add stb
                            idx 1 add >idx
                        else
                            state 2 eq if
                                ch 32 eq ch 9 eq and if
                                else
                                    ch addr stb
                                    1 >idx
                                    3 >state
                                end
                            else
                                state 1 eq if
                                    ch 48 ge ch 57 le or if
                                        line 10 mul ch 48 sub add >line
                                    else
                                        line 1 lt line TB_MAX_LINE gt and if
                                            "LINE RANGE" tb_print_string tb_newline
                                            4 >state
                                        else
                                            line tb_line_addr >addr
                                            ch 32 eq ch 9 eq and if
                                                2 >state
                                            else
                                                ch addr stb
                                                1 >idx
                                                3 >state
                                            end
                                        end
                                    end
                                else
                                    state 0 eq if
                                        ch 48 ge ch 57 le or if
                                            ch 48 sub >line
                                            1 >state
                                        else
                                            ch 32 eq ch 9 eq and if
                                            else
                                                4 >state
                                            end
                                        end
                                    end
                                end
                            end
                        end
                        count 1 add >count
                    end
                    0
                end
            end
        else
            1
        end
    end
    state line addr idx tb_finish_load_line
;

fn main () :
    1 0x80000000 stb
    pcg.initPCG
    pec_rng.initPRNG
    pcg.col80
    8 irqc.IRQC_PENDING sth
    8 irqc.IRQC_ENABLE sth
    tb_repl
;

main
