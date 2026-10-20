#ifndef CPT32_PRINT_H
#define CPT32_PRINT_H
#include <stdint.h>
/* 引数の下位8 bitをUARTへ1文字出力します。 */
void print_putc(uint32_t character);
/* NUL終端文字列をUARTへ出力します。 */
void print_prn(const char *string);
/* NUL終端文字列をUARTへ出力し、CR/LFを追加します。 */
void print_prnln(const char *string);
#endif
