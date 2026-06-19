
"""
csc - Frontend CLI for the C subset compiler

Reads a C-subset source file, runs lexing/parsing/semantic/codegen and emits
SRC32 assembly using the backend emitter.

Usage:
  python tools/csc/csc.py input.c -o out.s
  python tools/csc/csc.py input.c --dump-bc
"""
import argparse
import sys
from csc_gen import compile_source
from backend_src32 import emit_src32


def main(argv=None):
	p = argparse.ArgumentParser(description='CSC compiler frontend and SRC32 backend')
	p.add_argument('input', help='C source file')
	p.add_argument('-o', '--out', help='Write assembly output to file (default stdout)')
	p.add_argument('--dump-bc', action='store_true', help='Dump bytecode emitter instead of assembly')
	args = p.parse_args(argv)

	src = open(args.input, 'r', encoding='utf-8').read()
	emitter = compile_source(src)

	if args.dump_bc:
		out = emitter.dump()
	else:
		out = emit_src32(emitter)

	if args.out:
		with open(args.out, 'w', encoding='utf-8') as f:
			f.write(out)
		print(f'Wrote output to {args.out}')
	else:
		print(out)


if __name__ == '__main__':
	main()

