/* pkcl の POSIX I/O を、実際の fs・console サービスに対して確かめるゲスト。
 * 失敗したら失敗した行番号で終了する（0 が成功）。 */
#include <errno.h>
#include <fcntl.h>
#include <foundation/fnd.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define CHECK(x)                \
    do {                        \
        if (!(x))               \
            _exit(__LINE__);    \
    } while (0)
#define FAILS(call, code) CHECK((call) == -1 && errno == (code))

extern char **environ;

int main(int argc, char **argv) {
    CHECK(argc == 1 && argv[1] == 0 && environ && environ[0] == 0);
    char buf[16];
    struct stat st;

    /* 標準記述子はコンソール。 */
    CHECK(isatty(0) && isatty(1) && isatty(2));
    CHECK(write(1, "pkcl: hello\n", 12) == 12);
    FAILS(lseek(1, 0, SEEK_CUR), ESPIPE);
    FAILS(read(1, buf, 1), EBADF); /* STDOUT は書き込み専用 */

    /* 作成・書き込み・stat。 */
    umask(022);
    int fd = open("/pkcl.txt", O_WRONLY | O_CREAT | O_EXCL, 0666);
    CHECK(fd >= 3);
    FAILS(open("/pkcl.txt", O_WRONLY | O_CREAT | O_EXCL, 0666), EEXIST);
    CHECK(write(fd, "abcde", 5) == 5);
    FAILS(read(fd, buf, 1), EBADF);
    CHECK(close(fd) == 0);
    FAILS(close(fd), EBADF);
    CHECK(stat("/pkcl.txt", &st) == 0);
    CHECK(S_ISREG(st.st_mode) && st.st_size == 5);
    CHECK((st.st_mode & 0777) == 0644); /* 0666 & ~022 */
    CHECK(access("/pkcl.txt", R_OK | W_OK) == 0);
    FAILS(access("/pkcl-missing", F_OK), ENOENT);

    /* シーク・読み出し・追記。 */
    fd = open("/pkcl.txt", O_RDWR);
    CHECK(fd >= 3);
    CHECK(lseek(fd, 2, SEEK_SET) == 2);
    CHECK(read(fd, buf, 16) == 3 && memcmp(buf, "cde", 3) == 0);
    CHECK(read(fd, buf, 16) == 0);
    CHECK(lseek(fd, 0, SEEK_END) == 5);
    CHECK(lseek(fd, -1, SEEK_CUR) == 4);
    FAILS(lseek(fd, -10, SEEK_SET), EINVAL);
    CHECK(close(fd) == 0);
    fd = open("/pkcl.txt", O_WRONLY | O_APPEND);
    CHECK(fd >= 3 && write(fd, "f", 1) == 1 && close(fd) == 0);
    CHECK(stat("/pkcl.txt", &st) == 0 && st.st_size == 6);

    /* dup は読み書き位置を共有し、dup2 は指定した番号に置く。 */
    fd = open("/pkcl.txt", O_RDWR);
    CHECK(fd >= 3);
    int copy = dup(fd);
    CHECK(copy > fd);
    CHECK(lseek(fd, 1, SEEK_SET) == 1);
    CHECK(read(copy, buf, 1) == 1 && buf[0] == 'b');
    CHECK(read(fd, buf, 1) == 1 && buf[0] == 'c');
    CHECK(dup2(fd, 20) == 20 && dup2(20, 20) == 20);
    CHECK(read(20, buf, 1) == 1 && buf[0] == 'd');
    CHECK(fcntl(20, F_GETFD) == 0 && fcntl(20, F_SETFD, FD_CLOEXEC) == 0 && fcntl(20, F_GETFD) == FD_CLOEXEC);
    CHECK((fcntl(fd, F_GETFL) & O_ACCMODE) == O_RDWR);
    int high = fcntl(fd, F_DUPFD, 30);
    CHECK(high == 30);
    FAILS(dup2(fd, 99), EBADF);

    /* 切り詰め。 */
    CHECK(ftruncate(fd, 3) == 0);
    CHECK(fstat(fd, &st) == 0 && st.st_size == 3);
    CHECK(fsync(fd) == 0);
    CHECK(close(fd) == 0 && close(copy) == 0 && close(20) == 0 && close(high) == 0);
    CHECK(truncate("/pkcl.txt", 1) == 0);
    CHECK(stat("/pkcl.txt", &st) == 0 && st.st_size == 1);

    /* ディレクトリ・rename・unlink・cwd。 */
    CHECK(mkdir("/pkcld", 0755) == 0);
    FAILS(mkdir("/pkcld", 0755), EEXIST);
    CHECK(stat("/pkcld", &st) == 0 && S_ISDIR(st.st_mode) && (st.st_mode & 0777) == 0755);
    FAILS(unlink("/pkcld"), EISDIR);
    CHECK(rename("/pkcl.txt", "/pkcld/moved.txt") == 0);
    FAILS(stat("/pkcl.txt", &st), ENOENT);
    FAILS(rmdir("/pkcld"), ENOTEMPTY);
    CHECK(chdir("/pkcld") == 0 && getcwd(buf, sizeof buf) == buf && memcmp(buf, "/pkcld", 7) == 0);
    FAILS(getcwd(buf, 3) == 0 ? -1 : 0, ERANGE);
    CHECK(open("moved.txt", O_RDONLY) == 3); /* 相対パスは fs が cwd から解決する。fd 3 が空いている */
    CHECK(close(3) == 0);
    CHECK(chmod("moved.txt", 0600) == 0 && stat("moved.txt", &st) == 0 && (st.st_mode & 0777) == 0600);
    CHECK(rename("moved.txt", "/pkcl2.txt") == 0); /* 別ディレクトリへ */
    CHECK(chdir("/") == 0);
    CHECK(rmdir("/pkcld") == 0);
    /* rename は通常ファイルへの上書きもする。 */
    fd = open("/pkcl3.txt", O_WRONLY | O_CREAT, 0644);
    CHECK(fd >= 3 && write(fd, "xy", 2) == 2 && close(fd) == 0);
    CHECK(rename("/pkcl3.txt", "/pkcl2.txt") == 0);
    CHECK(stat("/pkcl2.txt", &st) == 0 && st.st_size == 2);
    CHECK(unlink("/pkcl2.txt") == 0);
    FAILS(unlink("/pkcl2.txt"), ENOENT);

    CHECK(write(1, "pkcl guest passed\n", 18) == 18);
    return 0;
}
