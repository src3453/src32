# requires pytest to run the tests. You can install it with `pip install pytest`.

import pytest
from lexer import Lexer, Token

def util_tokenize(source_code: str) -> list[Token]:
    # test util for tokenizing source code into a list of tokens using the Lexer class.
    lexer = Lexer(source_code)
    return lexer.tokenize() # returns a list of tokens, including an EOF token at the end

def test_number_decimal():
    source_code = "12345" # A simple test case with a single decimal number
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "NUMBER"
    assert tokens[0].value == 12345
    assert tokens[1].type == "EOF"
    assert tokens[1].value is None

def test_number_hexadecimal():
    source_code = "0x1234" # A simple test case with a single hexadecimal number
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "NUMBER"
    assert tokens[0].value == 0x1234
    assert tokens[1].type == "EOF"
    assert tokens[1].value is None

def test_number_hexadecimal_uppercase():
    source_code = "0XABCD" # A simple test case with a single hexadecimal number using uppercase 'X'
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "NUMBER"
    assert tokens[0].value == 0xABCD
    assert tokens[1].type == "EOF"
    assert tokens[1].value is None

def test_number_invalid_hexadecimal():
    source_code = "0xG123" # An invalid hexadecimal number (contains 'G')
    lexer = Lexer(source_code)
    with pytest.raises(SyntaxError):
        lexer.next_token() # will raise a SyntaxError because 'G' is not a valid hexadecimal digit

def test_number_invalid_hexadecimal_no_digits():
    source_code = "0x" # An invalid hexadecimal number (missing digits after '0x')
    lexer = Lexer(source_code)
    with pytest.raises(SyntaxError):
        lexer.next_token() # will raise a SyntaxError because there are no valid hexadecimal digits after '0x'

def test_identifiers():
    source_code = "var1 _var2 var_3" # A test case with multiple identifiers
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "IDENT"
    assert tokens[0].value == "var1"
    assert tokens[1].type == "IDENT"
    assert tokens[1].value == "_var2"
    assert tokens[2].type == "IDENT"
    assert tokens[2].value == "var_3"
    assert tokens[3].type == "EOF"
    assert tokens[3].value is None

def test_keywords():
    source_code = "if else while return" # A test case with multiple keywords
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "IF"
    assert tokens[0].value == "if"
    assert tokens[1].type == "ELSE"
    assert tokens[1].value == "else"
    assert tokens[2].type == "WHILE"
    assert tokens[2].value == "while"
    assert tokens[3].type == "RETURN"
    assert tokens[3].value == "return"
    assert tokens[4].type == "EOF"
    assert tokens[4].value is None

def test_skip_whitespace():
    source_code = "   \t\n  42   \n\t  " # A test case with leading and trailing whitespace around a number
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "NUMBER"
    assert tokens[0].value == 42
    assert tokens[1].type == "EOF"
    assert tokens[1].value is None

def test_simple_program():
    # A test case with a simple C program to check if the lexer can correctly tokenize a combination of keywords, identifiers, numbers, and operators.
    code = """
    int main() {
        return 3 + 4;
    }
    """

    tokens = util_tokenize(code)

    types = [t.type for t in tokens]

    assert types == [
        "INT",
        "IDENT",
        "LPAREN",
        "RPAREN",
        "LBRACE",
        "RETURN",
        "NUMBER",
        "PLUS",
        "NUMBER",
        "SEMICOLON",
        "RBRACE",
        "EOF"
    ]

def test_single_tokens():
    # work in progress
    source_code = "+ - * / = ; ( ) { }" # A test case with various single-character tokens
    tokens = util_tokenize(source_code)
    assert [t.type for t in tokens[:-1]] == [
        "PLUS","MINUS","ASTERISK","SLASH",
        "ASSIGN","SEMICOLON",
        "LPAREN","RPAREN",
        "LBRACE","RBRACE"
    ] 

def test_invalid_character():
    source_code = "@" # An invalid character that is not recognized by the lexer
    lexer = Lexer(source_code)
    with pytest.raises(SyntaxError):
        lexer.next_token() # will raise a SyntaxError because '@' is not a valid token in our C subset language

def test_empty_source():
    source_code = "" # An empty source code should produce only an EOF token
    tokens = util_tokenize(source_code)
    assert len(tokens) == 1
    assert tokens[0].type == "EOF"
    assert tokens[0].value is None

def test_number_with_identifier():
    source_code = "var123 456abc" # A test case with an identifier followed by a number
    tokens = util_tokenize(source_code)
    assert tokens[0].type == "IDENT"
    assert tokens[0].value == "var123"
    assert tokens[1].type == "NUMBER"
    assert tokens[1].value == 456
    assert tokens[2].type == "IDENT"
    assert tokens[2].value == "abc"
    assert tokens[3].type == "EOF"
    assert tokens[3].value is None

def test_large_integer():
    # 16bit unsigned integer max value is 65535 (0xFFFF), so this test checks if the lexer can handle large integers correctly.
    tokens = util_tokenize("65535")
    assert tokens[0].value == 65535

def test_two_char_tokens():
    # This test checks if the lexer can correctly recognize two-character tokens like "==", "!=", "<=", and ">=" in the source code.
    tokens = util_tokenize("a == b != c <= d >= e")

    assert [t.type for t in tokens if t.type != "EOF"] == [
        "IDENT","EQUAL","IDENT",
        "NOT_EQUAL","IDENT",
        "LESS_EQUAL","IDENT",
        "GREATER_EQUAL","IDENT"
    ]

def test_inline_comment():
    # This test checks if the lexer correctly ignores inline comments (comments that start with "//" and continue until the end of the line) in the source code.
    tokens = util_tokenize("1 // comment\n 2")

    assert tokens[0].value == 1
    assert tokens[1].value == 2

def test_block_comment():
    # This test checks if the lexer correctly ignores block comments (comments that start with "/*" and end with "*/") in the source code.
    tokens = util_tokenize("1 /* comment */ 2")

    assert tokens[0].value == 1
    assert tokens[1].value == 2

def test_unterminated_block_comment():
    # This test checks if the lexer correctly raises a SyntaxError when it encounters an unterminated block comment (a comment that starts with "/*" but does not have a matching "*/").
    with pytest.raises(SyntaxError):
        util_tokenize("/* comment")