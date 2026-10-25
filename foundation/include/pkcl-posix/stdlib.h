/* Newlib's <stdlib.h> declares itoa and utoa (BSD), which BusyBox defines itself with another
 * prototype. Hide Newlib's two declarations from code that includes this header. */
#define itoa __newlib_itoa
#define utoa __newlib_utoa
#include_next <stdlib.h>
#undef itoa
#undef utoa
int clearenv(void);
