# utils about tokens, TokenStream, etc. used by both lexer and parser.

class TokenStream:
    # A simple token stream class to manage tokens during parsing.
    def __init__(self, tokens):
        self.tokens = tokens
        self.index = 0

    def peek(self, lookahead=0):
        # Return the current token without consuming it, or None if we've reached the end of the token stream.
        if self.index + lookahead < len(self.tokens):
            return self.tokens[self.index + lookahead]
        return None

    def consume(self):
        # Consume the current token and return it, or None if we've reached the end of the token stream.
        # No bounds checking required, because there is always an EOF token at the end of the token stream.
        token = self.tokens[self.index]
        self.index += 1
        return token
    
    def match(self, token_type):
        # Check if the current token matches the expected type. If it does, consume and return it; otherwise, return None.
        token = self.peek()
        if token is not None and token.type == token_type:
            return self.consume()
        return None

    def check(self, token_type):
        # Check if the current token matches the expected type without consuming it. Return True if it matches, False otherwise.
        token = self.peek()
        return token is not None and token.type == token_type
    
    def expect(self, token_type):
        # Expect the current token to be of a specific type, consume it, and return it. 
        # If the current token is not of the expected type, raise a SyntaxError.
        token = self.peek()
        if token is None:
            raise SyntaxError(f"Expected token of type {token_type}, but reached end of input")
        if token.type != token_type:
            raise SyntaxError(f"Expected token of type {token_type}, but got {token.type} at position {token.pos}")
        return self.consume()
