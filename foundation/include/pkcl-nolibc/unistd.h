#ifndef PKCL_UNISTD_H
#define PKCL_UNISTD_H
#include <sys/types.h>

#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4

int close(int fd);
ssize_t read(int fd, void *buf, size_t len);
ssize_t write(int fd, const void *buf, size_t len);
off_t lseek(int fd, off_t offset, int whence);
int dup(int fd);
int dup2(int fd, int newfd);
int isatty(int fd);
int ftruncate(int fd, off_t length);
int truncate(const char *path, off_t length);
int fsync(int fd);
int access(const char *path, int mode);
int unlink(const char *path);
int rmdir(const char *path);
int rename(const char *old_path, const char *new_path);
int chdir(const char *path);
char *getcwd(char *buf, size_t size);
void _exit(int code) __attribute__((noreturn));

uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);

#endif
