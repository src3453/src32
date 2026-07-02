
"""
csc - Frontend CLI for the C subset compiler

Reads a C-subset source file, runs lexing/parsing/semantic/codegen and emits
SRC32 assembly using the backend emitter.

Also supports compiling LLVM IR (.ll) files to SRC32 assembly.

Usage:
  python tools/csc/csc.py input.c -o out.s
  python tools/csc/csc.py input.c --dump-bc
  python tools/csc/csc.py input.ll --from-llvm -o out.s
  python tools/csc/csc.py input.ll --from-llvm --dump-bc
"""
import argparse
import sys
from csc_gen import compile_source
from backend_src32 import emit_src32
from llvm_ir_parser import compile_llvm_ir


def main(argv=None):
	p = argparse.ArgumentParser(description='CSC compiler frontend and SRC32 backend (with LLVM IR support)')
	p.add_argument('input', help='C source file or LLVM IR file (.ll)')
	p.add_argument('-o', '--out', help='Write assembly output to file (default stdout)')
	p.add_argument('--dump-bc', action='store_true', help='Dump bytecode emitter instead of assembly')
	p.add_argument('--from-llvm', action='store_true', help='Treat input as LLVM IR (.ll) instead of C source')
	args = p.parse_args(argv)

	src = open(args.input, 'r', encoding='utf-8').read()

	if args.from_llvm:
		emitter = compile_llvm_ir(src)
	else:
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

