/* ssh [-b ADDR] [-t TERM] [-p PORT] [USER@]HOST: SSH through the CNET TCP
 * bridge, which speaks telnet to us and SSH to HOST (doc/spec_NET_bridge.md
 * §5). The client is telnet.c. */
#define TELNET_SSH 1
#include "telnet.c"
