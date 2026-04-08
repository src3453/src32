from csc_ast import Return, ExpressionStatement, Block, If, While
import pytest
from parser import Parser
from lexer import Lexer

# Test naming convention: test_<feature being tested>_<specific case being tested>

def util_parse_expression(source_code):
    # A helper function to tokenize the source code and parse it into an AST.
    lexer = Lexer(source_code)
    tokens = []
    while True:
        token = lexer.next_token()
        tokens.append(token)
        if token.type == "EOF":
            break
    parser = Parser(tokens)
    return parser.parse_expression()

def util_parse_statement(source_code):
    # A helper function to tokenize the source code and parse it into an AST for general statements.
    lexer = Lexer(source_code)
    tokens = []
    while True:
        token = lexer.next_token()
        tokens.append(token)
        if token.type == "EOF":
            break
    parser = Parser(tokens)
    return parser.parse_statement()

def test_expression_precedence():
    # A simple test case to parse the expression "1 + 2 * 3" and check the structure of the resulting AST.
    # Also checks that multiplication has higher precedence than addition, so the AST should reflect that "2 * 3" is evaluated before adding "1".
    ast = util_parse_expression("1 + 2 * 3")

    assert ast.op == "+"
    assert ast.right.op == "*"

def test_expression_parentheses():
    # A test case to parse the expression "(1 + 2) * 3" and check that the parentheses correctly affect the structure of the AST.
    ast = util_parse_expression("(1 + 2) * 3")

    assert ast.op == "*"
    assert ast.left.op == "+"

def test_expression_nested_parentheses():
    # A test case to parse the expression "((1 + 2) * 3) - 4" and check that multiple levels of parentheses are handled correctly in the AST.
    ast = util_parse_expression("((1 + 2) * 3) - 4")

    assert ast.op == "-"
    assert ast.left.op == "*"
    assert ast.left.left.op == "+"

def test_return_statement():
    # A test case to parse a simple return statement and check that the AST correctly represents the return statement and its value.
    ast = util_parse_statement("return 1;")

    assert isinstance(ast, Return)

def test_return_expression():
    # A test case to parse a return statement with an expression and check that the AST correctly represents the return statement and its expression.
    ast = util_parse_statement("return 1 + 2;")

    assert isinstance(ast, Return)
    assert ast.value.op == "+"

def test_return_parenthesized_expression():
    # A test case to parse a return statement with a parenthesized expression and check that the AST correctly represents the return statement and its expression with correct precedence.
    ast = util_parse_statement("return (1 + 2) * 3;")

    assert isinstance(ast, Return)
    assert ast.value.op == "*"
    assert ast.value.left.op == "+"

def test_return_missing_semicolon():
    # A test case to parse a return statement without a semicolon and check that the parser raises a SyntaxError due to the missing semicolon.
    with pytest.raises(SyntaxError):
        util_parse_statement("return 1")

def test_expression_statement():
    # A test case to parse an expression statement and check that the AST correctly represents it as an ExpressionStatement with the correct expression.
    ast = util_parse_statement("1 + 2;")

    assert isinstance(ast, ExpressionStatement)

def test_expression_statement_expression():
    # A test case to parse an expression statement and check that the AST correctly represents the expression within the ExpressionStatement.
    ast = util_parse_statement("1 + 2;")

    assert ast.expression.op == "+"

def test_expression_statement_missing_semicolon():
    # A test case to parse an expression statement without a semicolon and check that the parser raises a SyntaxError due to the missing semicolon.
    with pytest.raises(SyntaxError):
        util_parse_statement("1 + 2")

def test_block_empty_block():
    # A test case to parse an empty block of code and check that the AST correctly represents it as a Block with no statements.
    ast = util_parse_statement("{}")

    assert isinstance(ast, Block)
    assert len(ast.statements) == 0

def test_block_single_statement():
    # A test case to parse a block of code with a single statement and check that the AST correctly represents it as a Block containing that statement.
    ast = util_parse_statement("{ return 1; }")

    assert len(ast.statements) == 1

def test_block_multiple_statements():
    # A test case to parse a block of code with multiple statements and check that the AST correctly represents it as a Block containing those statements.
    ast = util_parse_statement("""
    {
        1 + 2;
        return 3;
    }
    """)

    assert len(ast.statements) == 2

def test_block_nested_block():
    # A test case to parse a block of code with a nested block inside it and check that the AST correctly represents the nested structure of the blocks and their statements.
    ast = util_parse_statement("""
    {
        {
            return 1;
        }
    }
    """)

    assert len(ast.statements) == 1

def test_block_missing_rbrace():
    # A test case to parse a block of code that is missing the closing brace and check that the parser raises a SyntaxError due to the missing closing brace.
    with pytest.raises(SyntaxError):
        util_parse_statement("{ return 1;")

def test_if_statement():
    # A test case to parse a simple if statement and check that the AST correctly represents the if statement, its condition, and its then branch.
    ast = util_parse_statement("if (x) return 1;")

    assert ast.condition.name == "x"
    assert isinstance(ast.then_branch, Return)
    assert ast.then_branch.value.value == 1

def test_if_block():
    # A test case to parse an if statement with a block as its then branch and check that the AST correctly represents the if statement, its condition, and its then branch as a Block containing the correct statements.
    ast = util_parse_statement("""
    if (x) {
        return 1;
    }
    """)

    assert ast.condition.name == "x"
    assert isinstance(ast.then_branch, Block)

def test_if_else():
    # A test case to parse an if statement with both a then branch and an else branch and check that the AST correctly represents the if statement, its condition, its then branch, and its else branch.
    ast = util_parse_statement(
        "if (x) return 1; else return 2;"
    )

    assert ast.else_branch is not None

def test_if_nested_if():
    # A test case to parse nested if statements and check that the AST correctly represents the nested structure of the if statements, their conditions, and their branches.
    # Known "dangling else problem" test case to ensure that the parser correctly associates the else branch with the nearest if statement.
    ast = util_parse_statement(
        "if (x) if (y) return 1; else return 2;"
    )
    assert isinstance(ast.then_branch, If)
    assert ast.then_branch.condition.name == "y"
    assert isinstance(ast.then_branch.then_branch, Return)
    assert ast.then_branch.then_branch.value.value == 1
    assert isinstance(ast.then_branch.else_branch, Return)
    assert ast.then_branch.else_branch.value.value == 2

def test_if_nested_if_block():
    # A test case to parse an if statement with a nested if statement inside its then branch and check that the AST correctly represents the nested structure of the if statements, their conditions, and their branches.
    ast = util_parse_statement("""
    if (x) {
        if (y) {
            return 1;
        } else {
            return 2;
        }
    }
    """)
    assert isinstance(ast.then_branch, Block)
    inner_if = ast.then_branch.statements[0]
    assert isinstance(inner_if, If)
    assert inner_if.condition.name == "y"
    assert isinstance(inner_if.then_branch, Block)
    assert isinstance(inner_if.else_branch, Block) 

def test_if_missing_then_branch():
    # A test case to parse an if statement that is missing the then branch and check that the parser raises a SyntaxError due to the missing then branch.
    with pytest.raises(SyntaxError):
        util_parse_statement("if (x)")

def test_if_missing_condition():
    # A test case to parse an if statement that is missing the condition and check that the parser raises a SyntaxError due to the missing condition.
    with pytest.raises(SyntaxError):
        util_parse_statement("if return 1;")

def test_if_missing_else_branch():
    # A test case to parse an if statement that has an else keyword but is missing the else branch statement and check that the parser raises a SyntaxError due to the missing else branch.
    with pytest.raises(SyntaxError):
        util_parse_statement("if (x) return 1; else")

def test_if_missing_rparen():
    # A test case to parse an if statement that is missing the closing parenthesis for the condition and check that the parser raises a SyntaxError due to the missing closing parenthesis.
    with pytest.raises(SyntaxError):
        util_parse_statement("if (x return 1;")

def test_while_statement():
    # A test case to parse a simple while statement and check that the AST correctly represents the while statement, its condition, and its body.
    ast = util_parse_statement("while (x) return 1;")
    assert ast.condition.name == "x"

def test_while_block():
    # A test case to parse a while statement with a block as its body and check that the AST correctly represents the while statement, its condition, and its body as a Block containing the correct statements.
    ast = util_parse_statement("""
    while (x) {
        return 1;
    }
    """)
    assert len(ast.body.statements) == 1

def test_while_nested_while():
    # A test case to parse nested while statements and check that the AST correctly represents the nested structure of the while statements, their conditions, and their bodies.
    ast = util_parse_statement(
        "while (x) while (y) return 1;"
    )
    assert isinstance(ast.body, While)
    assert ast.body.condition.name == "y"
    assert isinstance(ast.body.body, Return)
    assert ast.body.body.value.value == 1

def test_while_missing_condition():
    # A test case to parse a while statement that is missing the condition and check that the parser raises a SyntaxError due to the missing condition.
    with pytest.raises(SyntaxError):
        util_parse_statement("while () return 1;")

def test_while_missing_body():
    # A test case to parse a while statement that is missing the body and check that the parser raises a SyntaxError due to the missing body.
    with pytest.raises(SyntaxError):
        util_parse_statement("while (x)")

def test_while_missing_rparen():
    # A test case to parse a while statement that is missing the closing parenthesis for the condition and check that the parser raises a SyntaxError due to the missing closing parenthesis.
    with pytest.raises(SyntaxError):
        util_parse_statement("while (x return 1;")