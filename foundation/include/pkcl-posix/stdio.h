#ifndef PKCL_STDIO_H
#define PKCL_STDIO_H
#include_next <stdio.h>
#include <sys/types.h>
ssize_t getline(char **line, size_t *size, FILE *stream);
#endif
