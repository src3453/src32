from token_utils import TokenStream
import csc_ast as ast

class Parser:
    def __init__(self, tokens):
        self.tokens = TokenStream(tokens) # convert list of tokens into a TokenStream for easier parsing

    def parse_primary(self):
        # Parse a primary expression (number or identifier).
        
        token = self.tokens.peek()

        if token is None:
            raise SyntaxError("Unexpected end of input, expected a primary expression")

        if token.type == "NUMBER": # Number token, create a Number AST node
            self.tokens.consume()
            return ast.Number(token.value)
        
        if token.type == "IDENT": # Identifier token, create an Identifier AST node
            self.tokens.consume()
            return ast.Identifier(token.value)
        
        if token.type == "LPAREN": # Parenthesized expression, parse the inner expression
            self.tokens.consume() # consume the '(' token
            expr = self.parse_expression() # parse the expression inside the parentheses
            self.tokens.expect("RPAREN") # expect and consume the ')' token
            return expr
        
        raise SyntaxError(f"Unexpected token {token.type} at position {token.pos}, expected a primary expression")

    def parse_mul_div(self):
        # Parse multiplication and division expressions.
        node = self.parse_primary() # Parse the left-hand side primary expression
        while True:
            if self.tokens.match("ASTERISK"): # If we see a '*', parse the right-hand side and create a BinaryOp node
                right = self.parse_primary()
                node = ast.BinaryOp(node, "*", right)
            elif self.tokens.match("SLASH"): # If we see a '/', parse the right-hand side and create a BinaryOp node
                right = self.parse_primary()
                node = ast.BinaryOp(node, "/", right)
            else:
                break # If we don't see '*' or '/', we're done parsing multiplication/division
        return node
    
    def parse_add_sub(self):
        # Parse addition and subtraction expressions.
        node = self.parse_mul_div() # Parse the left-hand side multiplication/division expression
        while True:
            if self.tokens.match("PLUS"): # If we see a '+', parse the right-hand side and create a BinaryOp node
                right = self.parse_mul_div()
                node = ast.BinaryOp(node, "+", right)
            elif self.tokens.match("MINUS"): # If we see a '-', parse the right-hand side and create a BinaryOp node
                right = self.parse_mul_div()
                node = ast.BinaryOp(node, "-", right)
            else:
                break # If we don't see '+' or '-', we're done parsing addition/subtraction
        return node
    
    def parse_expression(self):
        # Parse an expression, which currently only supports addition and subtraction.
        return self.parse_add_sub() # Start parsing with the lowest precedence level (addition/subtraction)
    
    def parse_return_statement(self):
        # Parse a return statement, which starts with the 'return' keyword followed by an expression and a semicolon.
        self.tokens.expect("RETURN") # Expect and consume the 'return' keyword
        expr = self.parse_expression() # Parse the expression that follows 'return'
        self.tokens.expect("SEMICOLON") # Expect and consume the ';' token
        return ast.Return(expr) # Create and return a Return AST node with the parsed expression as its value

    def parse_expression_statement(self):
        # Parse an expression statement, which is an expression followed by a semicolon.
        expr = self.parse_expression() # Parse the expression
        self.tokens.expect("SEMICOLON") # Expect and consume the ';' token
        return ast.ExpressionStatement(expr) # Create and return an ExpressionStatement AST node with the parsed expression

    def parse_block(self):
        # Parse a block of statements enclosed in braces.
        self.tokens.expect("LBRACE") # Expect and consume the '{' token
        statements = [] # Initialize an empty list to hold the statements in the block
        while not self.tokens.check("RBRACE"): # Keep parsing statements until we see the '}' token
            statements.append(self.parse_statement()) # Parse a statement and add it to the list of statements
        self.tokens.expect("RBRACE") # Expect and consume the '}' token
        return ast.Block(statements) # Create and return a Block AST node with the list of parsed statements

    def parse_if_statement(self):
        # Parse an if statement, which starts with the 'if' keyword followed by a condition in parentheses, a then branch, and an optional else branch.
        self.tokens.expect("IF") # Expect and consume the 'if' keyword
        self.tokens.expect("LPAREN") # Expect and consume the '(' token
        condition = self.parse_expression() # Parse the condition expression
        self.tokens.expect("RPAREN") # Expect and consume the ')' token
        then_branch = self.parse_statement() # Parse the 'then' branch statement
        else_branch = None # Initialize else_branch to None (it is optional)
        if self.tokens.match("ELSE"): # If we see an 'else' keyword, parse the 'else' branch statement
            else_branch = self.parse_statement()
        return ast.If(condition, then_branch, else_branch) # Create and return an If AST node with the parsed condition, then branch, and optional else branch

    def parse_while_statement(self):
        # Parse a while statement, which starts with the 'while' keyword followed by a condition in parentheses and a body statement.
        self.tokens.expect("WHILE") # Expect and consume the 'while' keyword
        self.tokens.expect("LPAREN") # Expect and consume the '(' token
        condition = self.parse_expression() # Parse the condition expression
        self.tokens.expect("RPAREN") # Expect and consume the ')' token
        body = self.parse_statement() # Parse the body statement
        return ast.While(condition, body) # Create and return a While AST node with the parsed condition and body

    def parse_statement(self):
        # Parse a statement, which can currently only be a return statement or an expression statement.
        if self.tokens.check("RETURN"): # If the current token is 'return', parse it as a return statement
            return self.parse_return_statement()
        if self.tokens.check("IF"): # If the current token is 'if', parse it as an if statement
            return self.parse_if_statement()
        if self.tokens.check("WHILE"): # If the current token is 'while', parse it as a while statement
            return self.parse_while_statement()
        if self.tokens.check("LBRACE"): # If the current token is '{', parse it as a block statement
            return self.parse_block()
        return self.parse_expression_statement() # Otherwise, parse it as an expression statement by default