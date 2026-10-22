; Protection checks (WAX v3 §8): write_code stores into its own RX block,
; exec_data jumps into its RW block. Both must fault. spin never yields, so
; the idle thread's HALT never ends a testbench run early. build.py
; prepends APP_BASE and DATA_ADDR.
.ORG APP_BASE
write_code:
    ST R0, [R30 + 0]
    JMP write_code
exec_data:
    LDIH R5, data
    LDIL R5, data
    JR R5
    NOP
spin:
    JMP spin

.ORG DATA_ADDR
data:
    .DWORD 0
