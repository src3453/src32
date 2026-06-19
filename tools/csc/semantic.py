"""
semantic.py - Semantic analysis for the CSC compiler
Simple semantic analyzer: symbol table (scopes), declarations, and basic type checks (int only).
"""
import csc_ast as ast


class SymbolTable:
	def __init__(self, parent=None):
		self.parent = parent
		self.table = {}

	def define(self, name, info):
		if name in self.table:
			raise SyntaxError(f"Redefinition of '{name}'")
		self.table[name] = info

	def lookup(self, name):
		if name in self.table:
			return self.table[name]
		if self.parent:
			return self.parent.lookup(name)
		return None


class SemanticAnalyzer:
	def __init__(self):
		self.global_scope = SymbolTable()
		self.current_scope = self.global_scope

	def analyze(self, node):
		method = f"analyze_{node.__class__.__name__}"
		fn = getattr(self, method, None)
		if fn is None:
			raise NotImplementedError(f"No semantic handler for {node.__class__.__name__}")
		return fn(node)

	def analyze_Number(self, node: ast.Number):
		return 'int'

	def analyze_Identifier(self, node: ast.Identifier):
		info = self.current_scope.lookup(node.name)
		if info is None:
			raise SyntaxError(f"Undefined identifier '{node.name}'")
		return info['type']

	def analyze_BinaryOp(self, node: ast.BinaryOp):
		left_t = self.analyze(node.left)
		right_t = self.analyze(node.right)
		# both must be int
		if left_t != 'int' or right_t != 'int':
			raise SyntaxError("Type error in binary operation: operands must be int")
		# comparisons return int (0/1)
		return 'int'

	def analyze_Assignment(self, node: ast.Assignment):
		if not isinstance(node.target, ast.Identifier):
			raise SyntaxError("Left-hand side of assignment must be identifier")
		varinfo = self.current_scope.lookup(node.target.name)
		if varinfo is None:
			raise SyntaxError(f"Assignment to undeclared variable '{node.target.name}'")
		val_t = self.analyze(node.value)
		if val_t != varinfo['type']:
			raise SyntaxError("Type mismatch in assignment")
		return varinfo['type']

	def analyze_VarDecl(self, node: ast.VarDecl):
		# only 'int' exists
		if node.initializer is not None:
			init_t = self.analyze(node.initializer)
			if init_t != 'int':
				raise SyntaxError("Initializer must be int")
		self.current_scope.define(node.name, {'type': 'int'})
		return None

	def analyze_Return(self, node: ast.Return):
		if node.value is not None:
			return self.analyze(node.value)
		return None

	def analyze_ExpressionStatement(self, node: ast.ExpressionStatement):
		return self.analyze(node.expression)

	def analyze_Block(self, node: ast.Block):
		# new scope
		old = self.current_scope
		self.current_scope = SymbolTable(parent=old)
		for stmt in node.statements:
			self.analyze(stmt)
		self.current_scope = old

	def analyze_If(self, node: ast.If):
		cond_t = self.analyze(node.condition)
		if cond_t != 'int':
			raise SyntaxError("Condition must be int")
		self.analyze(node.then_branch)
		if node.else_branch:
			self.analyze(node.else_branch)

	def analyze_While(self, node: ast.While):
		cond_t = self.analyze(node.condition)
		if cond_t != 'int':
			raise SyntaxError("Condition must be int")
		self.analyze(node.body)

	def analyze_FunctionCall(self, node: ast.FunctionCall):
		# resolve callee
		if not isinstance(node.callee, ast.Identifier):
			raise SyntaxError("Function call must be to an identifier")
		info = self.current_scope.lookup(node.callee.name)
		if info is None or info.get('type') != 'func':
			raise SyntaxError(f"Call to undefined function '{node.callee.name}'")
		# check arity
		expected = info.get('params', 0)
		if len(node.args) != expected:
			raise SyntaxError(f"Function '{node.callee.name}' expects {expected} args, got {len(node.args)}")
		for a in node.args:
			t = self.analyze(a)
			if t != 'int':
				raise SyntaxError("Function arguments must be int")
		return 'int'

	def analyze_FunctionDef(self, node: ast.FunctionDef):
		# create new scope for function body
		old = self.current_scope
		self.current_scope = SymbolTable(parent=self.global_scope)
		# define parameters
		for p in node.params:
			self.current_scope.define(p, {'type': 'int'})
		# analyze body
		self.analyze(node.body)
		self.current_scope = old
