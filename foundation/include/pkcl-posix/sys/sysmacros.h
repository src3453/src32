#ifndef PKCL_SYS_SYSMACROS_H
#define PKCL_SYS_SYSMACROS_H

#define major(dev) (((dev) >> 8) & 0xfff)
#define minor(dev) ((dev) & 0xff)
#define makedev(ma, mi) ((((ma) & 0xfff) << 8) | ((mi) & 0xff))

#endif
