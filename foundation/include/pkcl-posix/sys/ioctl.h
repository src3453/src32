#ifndef PKCL_SYS_IOCTL_H
#define PKCL_SYS_IOCTL_H

/* Only the terminal requests pkcl translates (doc/spec_pkcl.md §6.6). */
#define TIOCGWINSZ 0x5413
#define TIOCSWINSZ 0x5414
#define FIONREAD 0x541B

struct winsize {
    unsigned short ws_row, ws_col, ws_xpixel, ws_ypixel;
};

int ioctl(int fd, unsigned long request, ...);

#endif
