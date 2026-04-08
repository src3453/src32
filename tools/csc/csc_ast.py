# ast.py - Abstract Syntax Tree (AST) definitions for the C subset compiler (CSC)
# Abstract Syntax Tree (AST) is a data structure used to represent the structure
# of source code in a way that is easier for the compiler to analyze and manipulate. 
# In this file, we define the various node types that can appear in the AST for our C subset language.

# Nodes in the AST
class Number:
    def __init__(self, value):
        self.value = int(value) # Ensure that the value is stored as an integer (lexer may return it as a string)

class Identifier:
    def __init__(self, name):
        self.name = name

class BinaryOp:
    # node graph for binary operations (e.g., addition, subtraction, multiplication, division)
    def __init__(self, left, op, right):
        self.left = left # left edge of the binary operation
        self.op = op # node type (e.g., '+', '-', '*', '/')
        self.right = right # right edge of the binary operation

class Return:
    # node graph for return statements
    def __init__(self, value):
        self.value = value

class ExpressionStatement:
    # node graph for expression statements (e.g., a standalone expression followed by a semicolon)
    def __init__(self, expression):
        self.expression = expression

class Block:
    # node graph for a block of statements enclosed in braces
    def __init__(self, statements):
        self.statements = statements # statements in the block represented as a list of AST nodes

class If:
    # node graph for if statements
    def __init__(self, condition, then_branch, else_branch=None):
        self.condition = condition # condition expression for the if statement
        self.then_branch = then_branch # AST node representing the 'then' branch of the if statement
        self.else_branch = else_branch # AST node representing the 'else' branch of the if statement (optional)

class While:
    # node graph for while loops
    def __init__(self, condition, body):
        self.condition = condition # condition expression for the while loop
        self.body = body # AST node representing the body of the while loop