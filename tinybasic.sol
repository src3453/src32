# TinyBASIC for CPT32, implemented in sol.
# Interactive PCG console; keyboard input comes from the PeC event FIFO.
# Program lines are stored as plain source text (not tokenized).
!include "sol/cpt32/pcg.sol"
!include "sol/cpt32/pec_rng.sol"

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
!const TB_DISK_BASE 0x00400000
!const TB_MAX_LINE 9999
!const TB_MAX_TEXT 120
!const TB_ARRAY_SIZE 1000
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

# Shared lexer and interpreter state.
!var tb_src 0
!var tb_pos 0
!var tb_tok_start 0
!var tb_tok_end 0
!var tb_tok 0
!var tb_num 0
!var tb_word_len 0
!var tb_string 0
!var tb_line 0
!var tb_next_line 0
!var tb_running 0
!var tb_direct 0
!var tb_stop 0
!var tb_jump 0
!var tb_error 0
!var tb_shift 0
!var tb_caps 0
!var tb_trace 0
!var tb_gsp 0
!var tb_fsp 0
!var tb_seed 1
!var tb_disk_pos 0
!var tb_disk_limit 0

fn tb_src_byte (offset) :
    tb_src offset add ldb ret
;

fn tb_word_byte (offset) :
    TB_WORD_BASE offset add ldb ret
;

# Return 0 when the current lowercase identifier equals the supplied string.
fn tb_word_is (text) :
    local i 0
    local a 0
    local b 0
    while
        i tb_word_byte >a
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

fn tb_skip_space () :
    local ch 0
    while
        tb_pos tb_src_byte >ch
        ch 32 eq
        if
            tb_pos 1 add >tb_pos
            0
        else
            ch 9 eq
            if
                tb_pos 1 add >tb_pos
                0
            else
                1
            end
        end
    end
;

fn tb_next () :
    local ch 0
    local value 0
    local digit 0
    local addr 0
    tb_skip_space
    tb_pos >tb_tok_start
    tb_pos tb_src_byte >ch
    ch >tb_tok
    ch 0 eq if
        TB_TOKEN_EOF >tb_tok
        tb_pos >tb_tok_end
        retn
    end
    ch 39 eq if
        TB_TOKEN_EOF >tb_tok
        retn
    end
    ch 34 eq if
        tb_pos 1 add >tb_pos
        0 >value
        while
            tb_pos tb_src_byte >ch
            ch 0 neq
            if
                ch 34 neq
                if
                    ch TB_TEMP_BASE value add stb
                    value 1 add >value
                    tb_pos 1 add >tb_pos
                    0
                else
                    1
                end
            else
                1
            end
        end
        0 TB_TEMP_BASE value add stb
        TB_TEMP_BASE >tb_string
        tb_pos >tb_tok_end
        tb_pos 1 add >tb_pos
        TB_TOKEN_STRING >tb_tok
        retn
    end
    ch 48 ge ch 57 le or if
        0 >value
        while
            tb_pos tb_src_byte >ch
            ch 48 ge ch 57 le or
            if
                ch 48 sub >digit
                value 10 mul digit add >value
                tb_pos 1 add >tb_pos
                0
            else
                1
            end
        end
        value >tb_num
        TB_TOKEN_NUMBER >tb_tok
        tb_pos >tb_tok_end
        retn
    end
    ch 65 ge ch 90 le or
    ch 97 ge ch 122 le or and if
        0 >tb_word_len
        while
            tb_pos tb_src_byte >ch
            ch 65 ge ch 90 le or
            ch 97 ge ch 122 le or and
            ch 48 ge ch 57 le or and
            if
                ch 65 ge ch 90 le or if
                    ch 32 add >ch
                end
                TB_WORD_BASE tb_word_len add >addr
                ch addr stb
                tb_word_len 1 add >tb_word_len
                tb_pos 1 add >tb_pos
                0
            else
                1
            end
        end
        0 TB_WORD_BASE tb_word_len add stb
        TB_TOKEN_IDENT >tb_tok
        # Translate the language keywords to dedicated token codes.
        "and" tb_word_is 0 eq if TB_TOKEN_AND >tb_tok end
        "or" tb_word_is 0 eq if TB_TOKEN_OR >tb_tok end
        "not" tb_word_is 0 eq if TB_TOKEN_NOT >tb_tok end
        "mod" tb_word_is 0 eq if TB_TOKEN_MOD >tb_tok end
        "abs" tb_word_is 0 eq if TB_TOKEN_ABS >tb_tok end
        "asc" tb_word_is 0 eq if TB_TOKEN_ASC >tb_tok end
        "rnd" tb_word_is 0 eq if TB_TOKEN_RND >tb_tok end
        "irnd" tb_word_is 0 eq if TB_TOKEN_RND >tb_tok end
        "sgn" tb_word_is 0 eq if TB_TOKEN_SGN >tb_tok end
        "let" tb_word_is 0 eq if TB_TOKEN_LET >tb_tok end
        "print" tb_word_is 0 eq if TB_TOKEN_PRINT >tb_tok end
        "input" tb_word_is 0 eq if TB_TOKEN_INPUT >tb_tok end
        "if" tb_word_is 0 eq if TB_TOKEN_IF >tb_tok end
        "then" tb_word_is 0 eq if TB_TOKEN_THEN >tb_tok end
        "goto" tb_word_is 0 eq if TB_TOKEN_GOTO >tb_tok end
        "gosub" tb_word_is 0 eq if TB_TOKEN_GOSUB >tb_tok end
        "return" tb_word_is 0 eq if TB_TOKEN_RETURN >tb_tok end
        "for" tb_word_is 0 eq if TB_TOKEN_FOR >tb_tok end
        "to" tb_word_is 0 eq if TB_TOKEN_TO >tb_tok end
        "next" tb_word_is 0 eq if TB_TOKEN_NEXT >tb_tok end
        "run" tb_word_is 0 eq if TB_TOKEN_RUN >tb_tok end
        "list" tb_word_is 0 eq if TB_TOKEN_LIST >tb_tok end
        "new" tb_word_is 0 eq if TB_TOKEN_NEW >tb_tok end
        "clear" tb_word_is 0 eq if TB_TOKEN_CLEAR >tb_tok end
        "cls" tb_word_is 0 eq if TB_TOKEN_CLS >tb_tok end
        "end" tb_word_is 0 eq if TB_TOKEN_END >tb_tok end
        "stop" tb_word_is 0 eq if TB_TOKEN_STOP >tb_tok end
        "rem" tb_word_is 0 eq if TB_TOKEN_REM >tb_tok end
        "help" tb_word_is 0 eq if TB_TOKEN_HELP >tb_tok end
        "quit" tb_word_is 0 eq if TB_TOKEN_QUIT >tb_tok end
        "bye" tb_word_is 0 eq if TB_TOKEN_BYE >tb_tok end
        "load" tb_word_is 0 eq if TB_TOKEN_LOAD >tb_tok end
        "save" tb_word_is 0 eq if TB_TOKEN_SAVE >tb_tok end
        "tron" tb_word_is 0 eq if TB_TOKEN_TRON >tb_tok end
        "troff" tb_word_is 0 eq if TB_TOKEN_TROFF >tb_tok end
        tb_pos >tb_tok_end
        retn
    end
    tb_pos 1 add >tb_pos
    ch 60 eq if
        tb_pos tb_src_byte >value
        value 61 eq if 264 >tb_tok tb_pos 1 add >tb_pos end
        value 62 eq if 266 >tb_tok tb_pos 1 add >tb_pos end
    else
        ch 62 eq if
            tb_pos tb_src_byte >value
            value 61 eq if 265 >tb_tok tb_pos 1 add >tb_pos end
        end
    end
    tb_pos >tb_tok_end
;

fn tb_expect (token) :
    tb_tok token eq if
        tb_next
        0 ret
    end
    1 ret
;


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

# Evaluate a primary, function call, or unary expression.
fn tb_primary () :
    local value 0
    local index 0
    local ch 0
    tb_tok TB_TOKEN_NUMBER eq if
        tb_num >value
        tb_next
        value ret
    end
    tb_tok 45 eq if
        tb_next
        7 tb_expression neg ret
    end
    tb_tok 43 eq if
        tb_next
        7 tb_expression ret
    end
    tb_tok TB_TOKEN_NOT eq if
        tb_next
        3 tb_expression >value
        value 0 eq if 1 ret else 0 ret end
    end
    tb_tok 40 eq if
        tb_next
        0 tb_expression >value
        41 tb_expect drop
        value ret
    end
    tb_tok 64 eq if
        tb_next
        40 tb_expect drop
        0 tb_expression >index
        41 tb_expect drop
        index 0 lt if 0 ret end
        index TB_ARRAY_SIZE ge if 0 ret end
        TB_ARRAY_BASE index 2 shl add ld ret
    end
    tb_tok TB_TOKEN_ABS eq if
        tb_next 40 tb_expect drop
        0 tb_expression >value
        41 tb_expect drop
        value 0 lt if value neg >value end
        value ret
    end
    tb_tok TB_TOKEN_SGN eq if
        tb_next 40 tb_expect drop
        0 tb_expression >value
        41 tb_expect drop
        value 0 gt if 1 ret end
        value 0 lt if -1 ret end
        0 ret
    end
    tb_tok TB_TOKEN_ASC eq if
        tb_next 40 tb_expect drop
        tb_tok TB_TOKEN_STRING eq if
            tb_string ldb >value
            tb_next
        else
            0 tb_expression >value
        end
        41 tb_expect drop
        value ret
    end
    tb_tok TB_TOKEN_RND eq if
        tb_next 40 tb_expect drop
        0 tb_expression >value
        41 tb_expect drop
        value 0 le if 0 ret end
        rand value mod 1 add ret
    end
    tb_tok TB_TOKEN_IDENT eq if
        0 tb_word_byte >ch
        ch 97 sub >index
        tb_next
        index 0 ge index 26 lt or if
            index tb_get_var ret
        end
        0 ret
    end
    1 >tb_error
    tb_next
    0 ret
;

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
# Precedence-climbing integer expression parser. BASIC comparisons yield 1/0.
fn tb_expression (minimum) :
    local lhs 0
    local rhs 0
    local op 0
    local prec 0
    local done 0
    tb_primary >lhs
    0 >done
    while
        tb_tok tb_precedence >prec
        prec 0 gt prec minimum ge or if
            tb_tok tb_precedence >prec
            tb_tok >op
            tb_next
            prec 8 eq if
                prec tb_expression >rhs
            else
                prec 1 add tb_expression >rhs
            end
            op 43 eq if lhs rhs add >lhs end
            op 45 eq if lhs rhs sub >lhs end
            op 42 eq if lhs rhs mul >lhs end
            op 47 eq op 92 eq and if
                rhs 0 eq if 1 >tb_error 0 >lhs else lhs rhs div >lhs end
            end
            op TB_TOKEN_MOD eq if
                rhs 0 eq if 1 >tb_error 0 >lhs else lhs rhs mod >lhs end
            end
            op 94 eq if lhs rhs tb_power >lhs end
            op 61 eq if lhs rhs eq if 1 >lhs else 0 >lhs end end
            op 60 eq if lhs rhs lt if 1 >lhs else 0 >lhs end end
            op 62 eq if lhs rhs gt if 1 >lhs else 0 >lhs end end
            op 264 eq if lhs rhs le if 1 >lhs else 0 >lhs end end
            op 265 eq if lhs rhs ge if 1 >lhs else 0 >lhs end end
            op 266 eq if lhs rhs neq if 1 >lhs else 0 >lhs end end
            op TB_TOKEN_AND eq if lhs rhs and >lhs end
            op TB_TOKEN_OR eq if lhs rhs or >lhs end
            0
        else
            1
        end
    end
    lhs ret
;

fn tb_putc (char) :
    char 13 eq if retn end
    char 10 eq if _pcg_ln retn end
    char 8 eq if
        pcg_cur_x 0 gt if
            pcg_cur_x 1 sub pcg_cur_y locate
            32 putcscr
            pcg_cur_x 1 sub pcg_cur_y locate
        end
        retn
    end
    char putcscr
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
    local digit 0
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

fn tb_newline () :
    10 tb_putc
;


# Consume PeC keyboard events until Enter. HID shift/caps are tracked locally.
fn tb_read_line (dest capacity) :
    local count 0
    local event 0
    local usage 0
    local value 0
    local ch 0
    local status 0
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

fn tb_store_input_line () :
    local line 0
    local pos 0
    local ch 0
    TB_INPUT_BASE >tb_src
    0 >tb_pos
    tb_next
    tb_tok TB_TOKEN_NUMBER eq if
        tb_num >line
        line 1 lt line TB_MAX_LINE gt and if
            "LINE RANGE" tb_print_string tb_newline
            retn
        end
        tb_tok_end >pos
        while
            pos tb_src_byte >ch
            ch 32 eq ch 9 eq and
            if pos 1 add >pos 0 else 1 end
        end
        tb_src pos add ldb 0 eq if
            0 line tb_line_addr stb
        else
            tb_src pos add line tb_line_addr tb_copy_line
        end
    end
;

# Program command line execution, one statement per invocation.
fn tb_jump_line (line) :
    line 1 lt line TB_MAX_LINE gt and if
        1 >tb_error
        retn
    end
    line >tb_line
    line 1 add >tb_next_line
    line tb_line_addr >tb_src
    0 >tb_pos
    tb_next
    1 >tb_jump
;

fn tb_line_next () :
    local done 0
    tb_line 1 add >tb_next_line
    while
        tb_next_line TB_MAX_LINE le
        if
            tb_next_line tb_line_addr ldb 0 neq
            if
                tb_next_line >tb_line
                tb_next_line tb_line_addr >tb_src
                0 >tb_pos
                tb_next
                1 >done
                done 0 eq
            else
                tb_next_line 1 add >tb_next_line
                0
            end
        else
            1 >tb_stop
            1
        end
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
;

fn tb_list_program () :
    local line 1
    local addr 0
    local ch 0
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
    "LOAD/SAVE use slot 0: command followed by LBA and sector count." tb_print_string tb_newline
;

fn tb_clear_variables () :
    local i 0
    local addr 0
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

fn tb_assignment () :
    local index 0
    local value 0
    tb_tok TB_TOKEN_IDENT neq if 1 >tb_error retn end
    0 tb_word_byte 97 sub >index
    tb_next
    tb_tok 61 neq if 1 >tb_error retn end
    tb_next
    0 tb_expression >value
    index value tb_set_var
;

fn tb_array_assignment () :
    local index 0
    local value 0
    tb_next
    tb_tok 40 neq if 1 >tb_error retn end
    tb_next
    0 tb_expression >index
    tb_tok 41 neq if 1 >tb_error retn end
    tb_next
    tb_tok 61 neq if 1 >tb_error retn end
    tb_next
    0 tb_expression >value
    index value tb_array_store
;

fn tb_print_statement () :
    local done 0
    local newline 1
    tb_next
    0 >done
    while
        done
        if
            tb_tok TB_TOKEN_EOF eq tb_tok 58 eq and
            if
                1 >done
            else
                tb_tok TB_TOKEN_STRING eq
                if
                    tb_string tb_print_string
                    tb_next
                else
                    tb_tok 44 eq tb_tok 59 eq and
                    if
                        0 >newline
                        tb_next
                    else
                        0 tb_expression tb_print_number
                        1 >newline
                    end
                end
                tb_tok 44 eq tb_tok 59 eq and
                if
                    0 >newline
                    tb_next
                else
                    1 >done
                end
            end
            done
        else
            1
        end
    end
    newline 0 neq if tb_newline end
;

fn tb_input_statement () :
    local index 0
    tb_next
    tb_tok TB_TOKEN_STRING eq if
        tb_string tb_print_string
        tb_next
        tb_tok 44 eq if tb_next end
    else
        "? " tb_print_string
    end
    tb_tok TB_TOKEN_IDENT eq if
        0 tb_word_byte 97 sub >index
        tb_next
        TB_INPUT_BASE 120 tb_read_line drop
        index tb_input_value tb_set_var
    else
        1 >tb_error
    end
;

fn tb_gosub (line) :
    local frame 0
    tb_gsp 64 ge if 1 >tb_error retn end
    TB_GOSUB_BASE tb_gsp 12 mul add >frame
    tb_line frame st
    tb_src frame 4 add st
    tb_tok_start frame 8 add st
    tb_gsp 1 add >tb_gsp
    line tb_jump_line
;

fn tb_return () :
    local frame 0
    tb_gsp 0 le if 1 >tb_error retn end
    tb_gsp 1 sub >tb_gsp
    TB_GOSUB_BASE tb_gsp 12 mul add >frame
    frame ld >tb_line
    frame 4 add ld >tb_src
    frame 8 add ld >tb_pos
    tb_next
    1 >tb_jump
;

fn tb_find_line_after (line) :
    local candidate 0
    line 1 add >candidate
    while
        candidate TB_MAX_LINE le
        if
            candidate tb_line_addr ldb 0 neq
            if candidate ret end
            candidate 1 add >candidate
            0
        else
            1
        end
    end
    0 ret
;

fn tb_for_statement () :
    local index 0
    local start 0
    local limit 0
    local frame 0
    local body_line 0
    tb_next
    tb_tok TB_TOKEN_IDENT neq if 1 >tb_error retn end
    0 tb_word_byte 97 sub >index
    tb_next
    tb_tok 61 neq if 1 >tb_error retn end
    tb_next
    0 tb_expression >start
    index start tb_set_var
    tb_tok TB_TOKEN_TO neq if 1 >tb_error retn end
    tb_next
    0 tb_expression >limit
    tb_fsp 32 ge if 1 >tb_error retn end
    TB_FOR_BASE tb_fsp 16 mul add >frame
    index frame st
    limit frame 4 add st
    tb_line >body_line
    tb_tok TB_TOKEN_EOF eq if
        tb_line tb_find_line_after >body_line
        0 frame 12 add st
    else
        tb_tok_start frame 12 add st
    end
    body_line frame 8 add st
    tb_fsp 1 add >tb_fsp
;

fn tb_next_statement () :
    local index 0
    local frame 0
    local value 0
    tb_fsp 0 le if 1 >tb_error retn end
    tb_next
    TB_FOR_BASE tb_fsp 1 sub 16 mul add >frame
    frame ld >index
    tb_tok TB_TOKEN_IDENT eq if
        0 tb_word_byte 97 sub index neq if 1 >tb_error retn end
        tb_next
    end
    index tb_get_var 1 add >value
    index value tb_set_var
    value frame 4 add ld le if
        frame 8 add ld >tb_line
        tb_line tb_line_addr >tb_src
        frame 12 add ld >tb_pos
        tb_next
        1 >tb_jump
    else
        tb_fsp 1 sub >tb_fsp
    end
;

fn tb_skip_line () :
    TB_TOKEN_EOF >tb_tok
;

fn tb_statement () :
    local value 0
    local line 0
    tb_tok TB_TOKEN_PRINT eq if tb_print_statement retn end
    tb_tok 63 eq if tb_print_statement retn end
    tb_tok TB_TOKEN_INPUT eq if tb_input_statement retn end
    tb_tok TB_TOKEN_LET eq if tb_next tb_assignment retn end
    tb_tok TB_TOKEN_IDENT eq if tb_assignment retn end
    tb_tok 64 eq if tb_array_assignment retn end
    tb_tok TB_TOKEN_GOTO eq if
        tb_next
        0 tb_expression >line
        line tb_jump_line
        retn
    end
    tb_tok TB_TOKEN_GOSUB eq if
        tb_next
        0 tb_expression >line
        line tb_gosub
        retn
    end
    tb_tok TB_TOKEN_RETURN eq if tb_return retn end
    tb_tok TB_TOKEN_IF eq if
        tb_next
        0 tb_expression >value
        tb_tok TB_TOKEN_THEN eq if tb_next end
        value 0 neq if
            tb_tok TB_TOKEN_NUMBER eq if
                tb_num tb_jump_line
            else
                tb_statement
            end
        else
            tb_skip_line
        end
        retn
    end
    tb_tok TB_TOKEN_FOR eq if tb_for_statement retn end
    tb_tok TB_TOKEN_NEXT eq if tb_next_statement retn end
    tb_tok TB_TOKEN_END eq tb_tok TB_TOKEN_STOP eq and if 1 >tb_stop tb_skip_line retn end
    tb_tok TB_TOKEN_RUN eq if tb_run_program tb_skip_line retn end
    tb_tok TB_TOKEN_LIST eq if tb_list_program tb_next retn end
    tb_tok TB_TOKEN_NEW eq if tb_clear_program tb_clear_variables tb_next retn end
    tb_tok TB_TOKEN_CLEAR eq if tb_clear_variables tb_next retn end
    tb_tok TB_TOKEN_CLS eq if tb_cls tb_next retn end
    tb_tok TB_TOKEN_HELP eq if tb_help tb_next retn end
    tb_tok TB_TOKEN_LOAD eq if 1 tb_disk_statement retn end
    tb_tok TB_TOKEN_SAVE eq if 2 tb_disk_statement retn end
    tb_tok TB_TOKEN_TRON eq if 1 >tb_trace tb_next retn end
    tb_tok TB_TOKEN_TROFF eq if 0 >tb_trace tb_next retn end
    tb_tok TB_TOKEN_REM eq if tb_skip_line retn end
    tb_tok TB_TOKEN_EOF eq tb_tok 58 eq and if retn end
    1 >tb_error
;

fn tb_cls () :
    local i 0
    0 >pcg_scroll_y
    0 >pcg_vram_row
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
    0 0 locate
;

fn tb_direct_line () :
    local done 0
    TB_INPUT_BASE >tb_src
    0 >tb_pos
    tb_next
    tb_tok TB_TOKEN_NUMBER eq if
        tb_store_input_line
        retn
    end
    1 >tb_direct
    0 >done
    while
        done
        if
            tb_tok TB_TOKEN_EOF eq if
                1 >done
            else
                tb_tok 58 eq if
                    tb_next
                else
                    0 >tb_jump
                    tb_statement
                    tb_jump 0 neq if
                        tb_run_program
                        1 >done
                    end
                    tb_error 0 neq if
                        "? SYNTAX/LINE ERROR" tb_print_string tb_newline
                        0 >tb_error
                        1 >done
                    end
                    tb_tok TB_TOKEN_EOF eq if 1 >done end
                end
            end
            done
        else
            1
        end
    end
    0 >tb_direct
;

fn tb_program_loop () :
    local done 0
    0 >tb_line
    0 >tb_stop
    0 >tb_error
    0 >tb_gsp
    0 >tb_fsp
    tb_clear_variables
    tb_line_next
    0 >done
    while
        tb_stop 0 eq tb_error 0 eq or
        if
            tb_tok TB_TOKEN_EOF eq if
                tb_line_next
            else
                tb_tok 58 eq if
                    tb_next
                else
                    0 >tb_jump
                    tb_trace 0 neq if tb_line tb_print_number 32 tb_putc tb_src tb_print_string tb_newline end
                    tb_statement
                    tb_jump 0 eq tb_tok TB_TOKEN_EOF eq or if tb_line_next end
                end
            end
            0
        else
            1
        end
    end
    0 >tb_running
    tb_error 0 neq if
        "? SYNTAX/LINE ERROR" tb_print_string tb_newline
        0 >tb_error
    end
;

fn tb_repl () :
    tb_cls
    tb_help
    while
        "OK" tb_print_string tb_newline
        "> " tb_print_string
        TB_INPUT_BASE 120 tb_read_line drop
        tb_direct_line
        0
    end
;

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

fn tb_load_disk_lines (bytes) :
    local pos 0
    local count 0
    local ch 0
    tb_clear_program
    0 >count
    while
        pos bytes lt
        if
            TB_DISK_BASE pos add ldb >ch
            ch 0 eq
            if
                1
            else
                ch 10 eq ch 13 eq and
                if
                    0 TB_INPUT_BASE count add stb
                    count 0 gt if tb_store_input_line end
                    0 >count
                    pos 1 add >pos
                    0
                else
                    count TB_MAX_TEXT lt if
                        ch TB_INPUT_BASE count add stb
                        count 1 add >count
                    end
                    pos 1 add >pos
                    0
                end
            end
        else
            1
        end
    end
    count 0 gt if
        0 TB_INPUT_BASE count add stb
        tb_store_input_line
    end
;

fn tb_disk_statement (command) :
    local lba 0
    local sectors 0
    local saved_src 0
    local saved_pos 0
    local result 0
    local have_lba 0
    local have_sectors 0
    tb_src >saved_src
    tb_next
    tb_tok TB_TOKEN_NUMBER eq if
        0 tb_expression >lba
        1 >have_lba
    end
    tb_tok TB_TOKEN_NUMBER eq if
        0 tb_expression >sectors
        1 >have_sectors
    else
        128 >sectors
    end
    tb_tok_start >saved_pos
    command 1 eq if
        lba sectors 1 tb_disk_transfer >result
        result 0 eq if
            sectors 512 mul tb_load_disk_lines
            "LOADED" tb_print_string tb_newline
        else
            "DISK READ ERROR" tb_print_string tb_newline
        end
    else
        have_lba 0 eq have_sectors 0 eq and
        if
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
    saved_src >tb_src
    saved_pos >tb_pos
    tb_next
;

fn tb_run_program () :
    1 >tb_running
    tb_program_loop
;

fn main () :
    1 0x80000000 stb
    initPCG
    initPRNG
    tb_repl
;

main
