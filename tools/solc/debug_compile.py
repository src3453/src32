import sys, pathlib
sys.path.insert(0, str(pathlib.Path('.').resolve()))
import sol_vm

s='fn write (c) : c ret ;'
print('SOURCE:', s)
print('TOKENS:', sol_vm.tokenize(s))
try:
    prog = sol_vm.compile_program(s)
    print('COMPILED PROGRAM:', prog)
except Exception as e:
    print('COMPILE ERROR:', type(e).__name__, e)
    # dump tokens again
    print('TOKENS (again):', sol_vm.tokenize(s))
    raise
