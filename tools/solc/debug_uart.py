import sys, pathlib
sys.path.insert(0, str(pathlib.Path('.').resolve()))
import sol_vm

src='''!var char 0 # define var

fn write (c) :
c 0x80040000 stb # write to uart
;

@loop # loop label
char write
char 1 add >char # increment counter and store to var
char 256 sub
jnz @loop # infinite loop
'''
print('SOURCE:\n', src)
print('TOKENS:')
print(sol_vm.tokenize(src))
try:
    prog = sol_vm.compile_program(src)
    print('COMPILED Program:')
    print('labels=', prog.labels)
    print('functions=', prog.functions)
    print('instructions count=', len(prog.instructions))
except Exception as e:
    import traceback
    print('COMPILE ERROR', type(e).__name__, e)
    traceback.print_exc()
    raise
