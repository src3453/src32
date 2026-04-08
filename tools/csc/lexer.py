# lexer.py - A simple lexer for the C subset compiler (csc)
# This lexer takes C source code as input and produces a stream of tokens that can be used by the parser.

# The lexer recognizes the following token types:
# - IDENT: for variable names and function names
# - NUMBER: for numeric literals (e.g., 123, 0x1A)
# - EOF: for end of file
# and reserved keywords in below dictionary:

KEYWORDS = {
    "int": "INT", # Note: SRC16 is a 16-bit big-endian system, so "int" type is 16 bits.
    "return": "RETURN",
    "if": "IF",
    "else": "ELSE",
    "while": "WHILE",
}

# Single-character tokens
SINGLE_TOKENS = {
    "+": "PLUS",
    "-": "MINUS",
    "*": "ASTERISK",
    "/": "SLASH",
    "=": "ASSIGN",
    ";": "SEMICOLON",
    "(": "LPAREN",
    ")": "RPAREN",
    "{": "LBRACE",
    "}": "RBRACE",
}

# Two-character tokens
TWO_CHAR_TOKENS = {
    "==": "EQUAL",
    "!=": "NOT_EQUAL",
    "<=": "LESS_EQUAL",
    ">=": "GREATER_EQUAL",
}

class Token:
    # A simple token class to represent different types of tokens in the C subset language.
    def __init__(self, type, value, pos):
        self.type = type
        self.value = value
        self.pos = pos

    def __repr__(self):
        return f"Token({self.type}, {self.value})"
    
class Lexer:
    # The Lexer class takes C source code as input and produces a stream of tokens.
    def __init__(self, source:str):
        self.source = source
        self.pos = 0
        self.length = len(source)

    def getchar(self, lookahead:int=0) -> str|None:
        # Return the current character or None if we've reached the end of the source.
        if self.pos + lookahead >= self.length:
            return None
        return self.source[self.pos + lookahead]
    
    def advance(self) -> str|None:
        # Move the position forward by one character, and return the current character.
        ch = self.getchar()
        if ch is not None: # avoid advancing past the end of the source
            self.pos += 1
        return ch
    
    def skip_whitespace(self):
        # Skip over any whitespace characters (spaces, tabs, newlines).
        while self.getchar() is not None and self.getchar().isspace():
            self.advance()

    def ignore_comment_inline(self):
        # Skip over comments in the source code.
        while self.getchar() is not None and self.getchar() != '\n': # skip characters until we reach the end of the line
            self.advance()
        if self.getchar() == '\n': # also skip the newline character at the end of the comment line
            self.advance()

    def ignore_comment_block(self):
        # Skip over block comments in the source code. 
        self.advance() # skip '/'
        self.advance() # skip '*'
        while True:
            if self.getchar() is None:
                raise SyntaxError("Unterminated block comment") # If we reach the end of the source without finding the closing "*/", it's an error.
            if self.getchar() == '*' and self.getchar(1) == '/':
                self.advance() # skip '*'
                self.advance() # skip '/'
                break
            self.advance() # continue advancing through the comment until we find the closing "*/"

    def read_number(self) -> Token:
        # Read a number token from the source code, handling decimal and hexadecimal formats.
        start_pos = self.pos
        if self.getchar() == '0' and self.getchar(1) in ['x', 'X']: # find "0x" or "0X" prefix for hexadecimal numbers
            # Hexadecimal number
            self.advance() # skip '0'
            self.advance() # skip 'x' or 'X'
            if not self.getchar() or not (self.getchar().isdigit() or self.getchar().lower() in 'abcdef'): # after "0x", there should be at least one valid hexadecimal digit
                raise SyntaxError(f"Invalid hexadecimal number at position {start_pos}")
            while self.getchar() is not None and (self.getchar().isdigit() or self.getchar().lower() in 'abcdef'): # valid hexadecimal digits
                self.advance()
            value = int(self.source[start_pos:self.pos], 16) # Convert the hexadecimal string to an integer.
        else:
            # Decimal number
            while self.getchar() is not None and self.getchar().isdigit():
                self.advance()
            value = int(self.source[start_pos:self.pos]) # Convert the decimal string to an integer.
        return Token("NUMBER", value, start_pos)
    
    def read_identifier(self) -> Token:
        # Read an identifier token from the source code, which can be a variable name, function name, or keyword.
        start_pos = self.pos
        while self.getchar() is not None and (self.getchar().isalnum() or self.getchar() == '_'):
            self.advance()
        value = self.source[start_pos:self.pos] # Extract the identifier string from the source.
        token_type = KEYWORDS.get(value, "IDENT") # Check if the identifier is a reserved keyword; if not, it's a regular identifier.
        return Token(token_type, value, start_pos)
    
    def skip_ignorable(self):
        # Skip over any whitespace and comments before looking for the next token.
        while True: # Loop to skip whitespace and comments before looking for the next token.
            self.skip_whitespace() # Skip any leading whitespace before looking for the next token.
            if self.getchar() == '/' and self.getchar(1) == '/': # If we encounter "//", it's the start of a single-line comment, so we should ignore it.
                self.ignore_comment_inline()
                continue
            if self.getchar() == '/' and self.getchar(1) == '*': # If we encounter "/*", it's the start of a block comment, so we should ignore it.
                self.ignore_comment_block()
                continue
            break

    def next_token(self) -> Token:
        # Get the next token from the source code.
        self.skip_ignorable() # Skip any whitespace and comments before looking for the next token.
        ch = self.getchar() # Get the current character to determine what type of token we are looking at.

        if ch is None:
            return Token("EOF", None, self.pos) # If we've reached the end of the source, return an EOF token.
        
        if ch.isdigit():
            return self.read_number() # If the current character is a digit, read a number token.
        
        if ch.isalpha() or ch == '_': # Identifiers can start with a letter or underscore
            return self.read_identifier() # Read an identifier token (variable names, function names, or keywords).
        
        # Note: run longer tokens (e.g., "==", "!=", "<=", ">=") before shorter tokens (e.g., "=", "<", ">") to ensure correct tokenization.
        two_char = ch + (self.getchar(1) or '') # Check for two-character tokens (e.g., "==", "!=", "<=", ">=").
        if two_char in TWO_CHAR_TOKENS:
            token_type = TWO_CHAR_TOKENS[two_char] # Get the token type for the two-character token.
            self.advance() # Move past the first character of the two-character token.
            self.advance() # Move past the second character of the two-character token.
            return Token(token_type, two_char, self.pos - 2) # Return the token for the two-character operator.
        
        if ch in SINGLE_TOKENS:
            token_type = SINGLE_TOKENS[ch] # Get the token type for the single-character token.
            self.advance() # Move past the single-character token.
            return Token(token_type, ch, self.pos - 1) # Return the token for the single-character operator or punctuation.
        
        raise SyntaxError(f"Unexpected character '{ch}' at position {self.pos}") # If the character doesn't match any known token type, raise a syntax error.
    
    def tokenize(self) -> list[Token]:
        # Tokenize the entire source code and return a list of tokens.
        tokens = []
        while True:
            token = self.next_token() # Get the next token from the source code.
            tokens.append(token) # Add the token to the list of tokens.
            if token.type == "EOF":
                break # If we've reached the end of the file, stop tokenizing.
        return tokens
            