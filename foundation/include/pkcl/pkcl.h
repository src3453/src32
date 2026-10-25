#ifndef PKCL_PKCL_H
#define PKCL_PKCL_H
#include <foundation/fnd.h>
#include <sys/stat.h>

/* pkcl の内部境界（doc/spec_pkcl.md §3）。アプリケーションは POSIX の名前だけを使い、
 * これは pkcl の .c 同士と、pkcl をテストするコードのためのものです。 */

/* ファイル記述子は 0..PKCL_OPEN_MAX-1。カーネルのハンドル上限（63）に合わせる。 */
#ifndef PKCL_OPEN_MAX
#define PKCL_OPEN_MAX 64
#endif

/* Foundation のエラーを errno の値に直す。0 以上（成功）は 0。 */
int pkcl_errno(fnd_status status);
/* status が負なら errno を設定して -1、そうでなければ 0 を返す。 */
int pkcl_fail(fnd_status status);

/* fd が開いていれば Foundation のハンドル、そうでなければ 0（errno = EBADF）。 */
fnd_handle pkcl_handle(int fd);

/* 最初の POSIX 呼び出しで自動的に呼ばれる。fnd_init と fd 0〜2 の準備をする。 */
void pkcl_init(void);

/* 絶対パス化せずにそのまま fs へ送るので、パス長（NUL を除く）の上限だけ検査する。 */
#define PKCL_PATH_MAX 4096

/* POSIX の mode と FS2 の mode（owner と other の 6 bit、spec_FS2 §8）の変換。
 * group の bit は捨て、stat では other と同じ値にする。 */
uint32_t pkcl_mode_to_fnd(uint32_t posix_mode);
uint32_t pkcl_mode_from_fnd(uint32_t type, uint32_t fnd_mode);

/* fnd_stat を struct stat に直す。大きさが 2 GiB 以上なら EOVERFLOW で -1。 */
int pkcl_stat_convert(const fnd_stat_t *fs, struct stat *st);
/* umask の現在値（umask() は設定と取得を同時に行うため）。 */
mode_t pkcl_current_umask(void);

#endif
