"""
csc_gen - C subset compiler (bytecode backend)
This module provides a simple front-end pipeline: lexing, parsing, semantic analysis,
and generation of a simple bytecode format (no optimizations).
"""
from lexer import Lexer
from parser import Parser
from semantic import SemanticAnalyzer
import csc_ast as ast


class CodeEmitter:
	def __init__(self):
		self.code = []

	def emit(self, instr, *args):
		if args:
			self.code.append((instr, *args))
		else:
			self.code.append((instr,))

	def dump(self):
		# Return human-readable bytecode
		out = []
		for i, instr in enumerate(self.code):
			out.append(f"{i:04}: {instr[0]} {' '.join(map(str,instr[1:]))}")
		return "\n".join(out)


class CodeGenerator:
	def __init__(self, emitter: CodeEmitter):
		self.emitter = emitter
		self.var_index = {}  # mapping name -> index (per-function local slots)
		self.next_var = 0
		self.global_next_var = 0  # tracks the next free global slot

	def ensure_var(self, name):
		if name not in self.var_index:
			self.var_index[name] = self.next_var
			self.next_var += 1
		return self.var_index[name]

	def gen(self, node):
		method = f"gen_{node.__class__.__name__}"
		fn = getattr(self, method, None)
		if fn is None:
			raise NotImplementedError(f"No codegen for {node.__class__.__name__}")
		return fn(node)

	def gen_FunctionCall(self, node: ast.FunctionCall):
		# push args right-to-left so that after JAL+SAVE_RET,
		# the first argument is at the top of the stack.
		for arg in reversed(node.args):
			self.gen(arg)
		# emit call; backend will handle JAL and pushing return value
		self.emitter.emit('CALL', node.callee.name)


	def gen_Number(self, node: ast.Number):
		self.emitter.emit('PUSH_CONST', node.value)

	def gen_Identifier(self, node: ast.Identifier):
		idx = self.ensure_var(node.name)
		self.emitter.emit('LOAD_VAR', idx)

	def gen_BinaryOp(self, node: ast.BinaryOp):
		self.gen(node.left)
		self.gen(node.right)
		op = node.op
		if op == '+':
			self.emitter.emit('ADD')
		elif op == '-':
			self.emitter.emit('SUB')
		elif op == '*':
			self.emitter.emit('MUL')
		elif op == '/':
			self.emitter.emit('DIV')
		elif op in ('==','!=','<','>','<=','>='):
			self.emitter.emit('CMP', op)
		elif op == '&&':
			self.emitter.emit('AND')
		elif op == '||':
			self.emitter.emit('OR')
		else:
			raise NotImplementedError(f"Unknown binary operator {op}")

	def gen_Assignment(self, node: ast.Assignment):
		# target is Identifier
		self.gen(node.value)
		idx = self.ensure_var(node.target.name)
		self.emitter.emit('STORE_VAR', idx)

	def gen_VarDecl(self, node: ast.VarDecl):
		idx = self.ensure_var(node.name)
		if node.initializer is not None:
			self.gen(node.initializer)
			self.emitter.emit('STORE_VAR', idx)

	def gen_FunctionDef(self, node: ast.FunctionDef):
		# Reset local variable namespace for this function.
		# Save the current global_next_var so we can update it after.
		saved_var_index = self.var_index
		saved_next_var = self.next_var
		self.var_index = {}
		self.next_var = 0

		# Save the return address from the stack into R31 before
		# popping parameters (JAL pushes return address on top of args).
		self.emitter.emit('SAVE_RET')

		# Bind parameters from the call stack into local variable slots.
		# Args were pushed right-to-left, so first arg is at the top.
		for p in node.params:
			idx = self.ensure_var(p)
			self.emitter.emit('STORE_VAR', idx)
		self.gen(node.body)
		# Implicit return if control reaches the end of the function.
		self.emitter.emit('PUSH_CONST', 0)
		self.emitter.emit('RETURN')

		# Update global_next_var: function locals used slots 0..next_var-1,
		# so the next function must start after that to avoid overlap.
		self.global_next_var = max(self.global_next_var, self.next_var)
		# Restore the outer scope's var_index
		self.var_index = saved_var_index
		self.next_var = saved_next_var

	def gen_Return(self, node: ast.Return):
		if node.value is not None:
			self.gen(node.value)
			# Result is on the stack; RETURN will pop it into R1 and JR R31.
		else:
			self.emitter.emit('PUSH_CONST', 0)
		self.emitter.emit('RETURN')

	def gen_ExpressionStatement(self, node: ast.ExpressionStatement):
		self.gen(node.expression)
		self.emitter.emit('POP')

	def gen_Block(self, node: ast.Block):
		for stmt in node.statements:
			self.gen(stmt)

	def gen_If(self, node: ast.If):
		self.gen(node.condition)
		# emit conditional jump placeholder
		self.emitter.emit('JUMP_IF_FALSE', 'PLACEHOLDER')
		jmp_false_idx = len(self.emitter.code)-1
		self.gen(node.then_branch)
		if node.else_branch:
			self.emitter.emit('JUMP', 'PLACEHOLDER')
			jmp_end_idx = len(self.emitter.code)-1
			# fix false jump
			self.emitter.code[jmp_false_idx] = ('JUMP_IF_FALSE', len(self.emitter.code))
			self.gen(node.else_branch)
			# fix end jump
			self.emitter.code[jmp_end_idx] = ('JUMP', len(self.emitter.code))
		else:
			# fix false jump
			self.emitter.code[jmp_false_idx] = ('JUMP_IF_FALSE', len(self.emitter.code))

	def gen_While(self, node: ast.While):
		loop_start = len(self.emitter.code)
		self.gen(node.condition)
		self.emitter.emit('JUMP_IF_FALSE', 'PLACEHOLDER')
		jmp_out_idx = len(self.emitter.code)-1
		self.gen(node.body)
		self.emitter.emit('JUMP', loop_start)
		self.emitter.code[jmp_out_idx] = ('JUMP_IF_FALSE', len(self.emitter.code))


def compile_source(source: str):
	lexer = Lexer(source)
	tokens = lexer.tokenize()
	parser = Parser(tokens)
	# top-level parse: collect external declarations (functions and statements)
	units = []
	while not parser.tokens.check('EOF'):
		units.append(parser.parse_external_declaration())

	# separate functions and top-level statements
	functions = [u for u in units if isinstance(u, ast.FunctionDef)]
	statements = [u for u in units if not isinstance(u, ast.FunctionDef)]

	# semantic analysis
	sem = SemanticAnalyzer()

	# predefine functions in global scope so calls can resolve
	for f in functions:
		sem.global_scope.define(f.name, {'type': 'func', 'params': len(f.params)})

	# analyze top-level and function bodies
	block = ast.Block(statements)
	sem.analyze(block)
	for f in functions:
		sem.analyze(f)

	# codegen
	emitter = CodeEmitter()
	gen = CodeGenerator(emitter)

	# Pre-register function labels so that CALL instructions can resolve
	# targets during codegen of other functions.
	emitter.func_labels = {}
	for f in functions:
		emitter.func_labels[f.name] = -1  # placeholder
	for f in functions:
		emitter.func_labels[f.name] = len(emitter.code)
		gen.gen(f)

	# If top-level statements exist and no main() exists, generate code for them.
	# If main() exists, top-level statements are still semantically analyzed
	# (which may catch errors) but not code-generated.
	if not any(f.name == 'main' for f in functions):
		gen.gen(block)

	# Attach var_index to emitter so backend can determine variable count
	emitter.var_index = gen.var_index

	# emit a call to main and return (only if main exists)
	if any(f.name == 'main' for f in functions):
		emitter.emit('CALL', 'main')
		emitter.emit('HALT')

	return emitter


if __name__ == '__main__':
	import sys
	if len(sys.argv) < 2:
		print('Usage: csc.py <source.c>')
		sys.exit(1)
	path = sys.argv[1]
	src = open(path).read()
	emitter = compile_source(src)
	print(emitter.dump())