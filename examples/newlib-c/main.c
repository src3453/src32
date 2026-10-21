#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

static int compare(const void *a, const void *b)
{
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}

int main(void)
{
    char text[64];
    strcpy(text, "newlib");
    strcat(text, " on CPT32");
    if (strlen(text) != 15 || strcmp(text, "newlib on CPT32")) return 101;
    if (strncmp(text, "new", 3) || strchr(text, 'C') != text + 10) return 102;
    if (strrchr(text, 'n') != text + 8 || strstr(text, "CPT32") != text + 10) return 103;
    if (strspn("abc42", "abc") != 3 || strcspn(text, " ") != 6) return 104;
    if (strpbrk(text, "PT") != text + 11) return 105;
    char copy[64];
    memset(copy, 0, sizeof(copy));
    memcpy(copy, text, 16);
    if (memcmp(copy, text, 16) || memchr(copy, 'C', 16) != copy + 10) return 106;
    memmove(copy + 1, copy, 16);
    if (strcmp(copy + 1, text)) return 107;
    strncpy(copy, "abc", 5);
    if (copy[3] || copy[4]) return 108;
    strncat(copy, "defghi", 3);
    if (strcmp(copy, "abcdef")) return 109;

    int numbers[] = {8, -3, 42, 0, 8, -100, 17, 1, 2, 3, 4, 5, 6, 7, 9, 10};
    qsort(numbers, 16, sizeof(int), compare);
    for (unsigned i = 1; i < 16; ++i)
        if (numbers[i - 1] > numbers[i]) return 110;
    int key = 42;
    if (!bsearch(&key, numbers, 16, sizeof(int), compare)) return 111;
    key = 43;
    if (bsearch(&key, numbers, 16, sizeof(int), compare)) return 112;
    if (abs(-42) != 42 || labs(-42L) != 42 || llabs(-0x100000002LL) != 0x100000002LL) return 113;
    div_t d = div(-43, 5);
    ldiv_t l = ldiv(-43L, 5L);
    lldiv_t ll = lldiv(-0x100000002LL, 3LL);
    if (d.quot != -8 || d.rem != -3 || l.quot != -8 || l.rem != -3) return 114;
    if (ll.quot != -1431655766LL || ll.rem) return 115;

    char *p = calloc(16, 1);
    if (!p || ((uintptr_t)p & 7)) return 120;
    for (unsigned i = 0; i < 16; ++i) if (p[i]) return 121;
    strcpy(p, "preserved");
    char *grown = realloc(p, 128);
    if (!grown || strcmp(grown, "preserved")) return 122;
    p = realloc(grown, 10);
    if (!p || strcmp(p, "preserved")) return 123;
    errno = 0;
    if (realloc(p, 1000000) || errno != ENOMEM || strcmp(p, "preserved")) return 124;
    free(p);
    free(NULL);
    errno = 0;
    if (calloc(0x80000001u, 2) || errno != ENOMEM) return 125;
    errno = 0;
    if (malloc(1000000) || errno != ENOMEM) return 126;
    /* Repeatedly exceed total heap size to prove free blocks are reused. */
    for (unsigned i = 0; i < 80; ++i) {
        p = malloc(2048);
        if (!p) return 127;
        p[2047] = 42;
        free(p);
    }
    return 42;
}
