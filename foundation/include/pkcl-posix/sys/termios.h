#ifndef PKCL_SYS_TERMIOS_H
#define PKCL_SYS_TERMIOS_H
#include <sys/types.h>

/* Terminal attributes over the console's LINE and RAW modes (doc/spec_pkcl.md §6.6).
 * Newlib's <termios.h> only includes this file. The values are Linux's. */
typedef unsigned char cc_t;
typedef unsigned int speed_t;
typedef unsigned int tcflag_t;

#define NCCS 32

struct termios {
    tcflag_t c_iflag, c_oflag, c_cflag, c_lflag;
    cc_t c_line;
    cc_t c_cc[NCCS];
    speed_t c_ispeed, c_ospeed;
};

/* c_iflag */
#define IGNBRK 0x001
#define BRKINT 0x002
#define IGNPAR 0x004
#define PARMRK 0x008
#define INPCK 0x010
#define ISTRIP 0x020
#define INLCR 0x040
#define IGNCR 0x080
#define ICRNL 0x100
#define IXON 0x400
#define IXOFF 0x1000
/* c_oflag */
#define OPOST 0x01
#define ONLCR 0x04
/* c_cflag */
#define CSIZE 0x30
#define CS8 0x30
#define CREAD 0x80
#define HUPCL 0x400
#define CLOCAL 0x800
#define PARENB 0x100
#define PARODD 0x200
#define CSTOPB 0x40
/* c_lflag */
#define ISIG 0x01
#define ICANON 0x02
#define ECHO 0x08
#define ECHOE 0x10
#define ECHOK 0x20
#define ECHONL 0x40
#define NOFLSH 0x80
#define IEXTEN 0x8000
#define TOSTOP 0x100
/* c_cc */
#define VINTR 0
#define VQUIT 1
#define VERASE 2
#define VKILL 3
#define VEOF 4
#define VTIME 5
#define VMIN 6
#define VSTART 8
#define VSTOP 9
#define VSUSP 10
/* tcsetattr */
#define TCSANOW 0
#define TCSADRAIN 1
#define TCSAFLUSH 2
/* tcflush */
#define TCIFLUSH 0
#define TCOFLUSH 1
#define TCIOFLUSH 2
/* speeds */
#define B0 0
#define B9600 13
#define B38400 15
#define B115200 0x1002

int tcgetattr(int fd, struct termios *t);
int tcsetattr(int fd, int actions, const struct termios *t);
int tcflush(int fd, int queue);
int tcdrain(int fd);
speed_t cfgetispeed(const struct termios *t);
speed_t cfgetospeed(const struct termios *t);
int cfsetispeed(struct termios *t, speed_t speed);
int cfsetospeed(struct termios *t, speed_t speed);
pid_t tcgetpgrp(int fd);
int tcsetpgrp(int fd, pid_t pgrp);

#endif
