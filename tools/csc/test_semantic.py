from lexer import Lexer
from parser import Parser
from semantic import SemanticAnalyzer
import csc_ast as ast
import pytest
from csc_gen import compile_source


def parse_block(src):
    lexer = Lexer(src)
    tokens = lexer.tokenize()
    parser = Parser(tokens)
    stmts = []
    while not parser.tokens.check('EOF'):
        stmts.append(parser.parse_statement())
    return ast.Block(stmts)


def test_var_decl_and_usage_semantic():
    block = parse_block('int x = 2; x = x + 3; return x;')
    sem = SemanticAnalyzer()
    # should not raise
    sem.analyze(block)


def test_undeclared_variable_error():
    block = parse_block('return y;')
    sem = SemanticAnalyzer()
    with pytest.raises(SyntaxError):
        sem.analyze(block)


def test_redefinition_error():
    block = parse_block('int x = 1; int x = 2;')
    sem = SemanticAnalyzer()
    with pytest.raises(SyntaxError):
        sem.analyze(block)


def test_compile_pipeline_emits_bytecode():
    emitter = compile_source('int x = 5; return x;')
    bc = emitter.code
    # expect PUSH_CONST 5 and STORE_VAR and LOAD_VAR and RETURN somewhere
    assert any(instr[0] == 'PUSH_CONST' and instr[1] == 5 for instr in bc)
    assert any(instr[0] == 'STORE_VAR' for instr in bc)
    assert any(instr[0] == 'LOAD_VAR' for instr in bc)
    assert any(instr[0] == 'RETURN' for instr in bc)


def test_compile_pipeline_with_main_and_call():
    source = (
        'int add(int a, int b) { return a + b; } '
        'int main() { return add(1, 2); }'
    )
    emitter = compile_source(source)
    bc = emitter.code
    assert any(instr[0] == 'CALL' and instr[1] == 'main' for instr in bc)
    assert any(instr[0] == 'CALL' and instr[1] == 'add' for instr in bc)