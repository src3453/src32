!abi_compat "Foundation" 1
!include "../../sol/foundation/libfnd.sol"
fnd_init drop
fnd_argc 2 neq if -110 fnd_exit end
"hello from sol child\n" fnd_con_write drop
1 fnd_argv fnd_con_write drop
"\n" fnd_con_write drop
17 fnd_exit
