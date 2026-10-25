/* BusyBox 向けに足す定義（doc/spec_pkcl.md §8）。リンクするのは busybox.py だけで、pkcl 本体には入らない。
 *
 * Newlib の nano scanf は浮動小数の読み取りを _scanf_float への弱い参照で呼ぶ。弱いシンボルがないので
 * 常に strtod 一式（約 60 KB）が入り、BusyBox を 1 MiB（FND_EXEC_IMAGE_MAX）に収められなくなる。
 * BusyBox は scanf で浮動小数を読まないので、一致失敗を返す定義に置き換える。 */
#include <stdarg.h>
#include <stddef.h>

struct _reent;
struct _scan_data_t;

int _scanf_float(struct _reent *data, void *pdata, void *fp, va_list *ap) {
    (void)data, (void)pdata, (void)fp, (void)ap;
    return -1;
}
