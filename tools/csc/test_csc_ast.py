from lexer import Lexer
from parser import Parser
import csc_ast as ast
import pytest


def parse_statements(src):
    lexer = Lexer(src)
    tokens = lexer.tokenize()
    parser = Parser(tokens)
    stmts = []
    while not parser.tokens.check('EOF'):
        stmts.append(parser.parse_statement())
    return stmts


def test_var_decl_and_assignment_parsing():
    stmts = parse_statements('int x = 1; x = x + 2;')
    assert isinstance(stmts[0], ast.VarDecl)
    assert stmts[0].name == 'x'
    assert isinstance(stmts[0].initializer, ast.Number)

    assert isinstance(stmts[1], ast.ExpressionStatement)
    assign = stmts[1].expression
    assert isinstance(assign, ast.Assignment)
    assert assign.target.name == 'x'


def test_comparison_and_logical_parsing():
    stmts = parse_statements('1 < 2 && 3 > 4;')
    expr = stmts[0].expression
    assert isinstance(expr, ast.BinaryOp)
    assert expr.op == '&&'
    assert expr.left.op == '<'
    assert expr.right.op == '>'


def test_assignment_lhs_must_be_identifier():
    with pytest.raises(SyntaxError):
        parse_statements('1 = 2;')


def test_function_definition_and_call_parsing():
    lexer = Lexer('int main(int a, int b) { return add(a, b); }')
    tokens = lexer.tokenize()
    parser = Parser(tokens)
    node = parser.parse_external_declaration()

    assert isinstance(node, ast.FunctionDef)
    assert node.name == 'main'
    assert node.params == ['a', 'b']
    assert isinstance(node.body, ast.Block)
    ret = node.body.statements[0]
    assert isinstance(ret, ast.Return)
    call = ret.value
    assert isinstance(call, ast.FunctionCall)
    assert call.callee.name == 'add'
    assert len(call.args) == 2
# test_ast.py - Test suite for the C subset compiler (CSC), ast.py

import pytest 