#ifndef SRC32_CONFORMANCE_PROJECT_H
#define SRC32_CONFORMANCE_PROJECT_H
#include <stddef.h>
#include <stdint.h>

struct Record { unsigned char tag; int value; short step; };
struct Packed { unsigned char tag; uint32_t word; } __attribute__((packed));
typedef int (*operation)(int, int);
extern volatile unsigned seed;
extern struct Record records[3];
extern int zero_area[7];
extern const char *message;
extern int *selected;
extern operation global_operation;
int add(int a, int b);
int recurse(int n);
int control(int x);
int memory_checks(void);
int integer_checks(void);
int callback_checks(operation op);
int other_static(void);
struct Record transform(struct Record input);
int wide_checks(void);
unsigned long long wide_transform(unsigned long long input, unsigned step);
int variable_stack(unsigned count);
#endif
