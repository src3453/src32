/* pkcl の POSIX 層（fork・wait・pipe・setjmp）を実際の Foundation で確かめるゲスト。Newlib の posix 構成で組む。
 * 失敗したら失敗した行番号で終了する（0 が成功）。 */
#include <errno.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define CHECK(x)                \
    do {                        \
        if (!(x))               \
            _exit(__LINE__);    \
    } while (0)

static int shared = 1;

int main(void) {
    /* 自分の pid と、子から見た親。 */
    pid_t me = getpid();
    CHECK(me > 0);

    /* fork: COW でメモリは分かれ、終了コードは waitpid で受け取れる。 */
    pid_t child = fork();
    CHECK(child >= 0);
    if (child == 0) {
        shared = 99;
        CHECK(getppid() == me && getpid() != me);
        _exit(7);
    }
    int status = 0;
    CHECK(waitpid(child, &status, 0) == child);
    CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 7);
    CHECK(shared == 1);
    CHECK(waitpid(child, &status, WNOHANG) == -1 && errno == ECHILD);

    /* Foundation の負の通常エラーをシグナルと誤認しない。POSIX は下位 8 bit。 */
    static const int codes[] = {-1, -2, -10, -64, -255, 0, 255, 256, 259};
    for (unsigned i = 0; i < sizeof codes / sizeof codes[0]; ++i) {
        child = fork();
        CHECK(child >= 0);
        if (child == 0)
            _exit(codes[i]);
        CHECK(waitpid(child, &status, 0) == child);
        CHECK(WIFEXITED(status) && WEXITSTATUS(status) == (codes[i] & 255));
    }

    /* 実際のフォールトと強制終了はシグナルのまま保持する。 */
    child = fork();
    CHECK(child >= 0);
    if (child == 0) {
        volatile uintptr_t unmapped = 0;
        *(volatile uint32_t *)unmapped = 1;
        _exit(99);
    }
    CHECK(waitpid(child, &status, 0) == child);
    CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGSEGV);
    child = fork();
    CHECK(child >= 0);
    if (child == 0) {
        for (;;)
            sleep(1);
    }
    CHECK(kill(child, SIGKILL) == 0);
    CHECK(waitpid(child, &status, 0) == child);
    CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGKILL);

    /* pipe: 4 KiB を超える書き込みも、読み手が減らすまで待って全部届き、閉じれば EOF。 */
    int p[2];
    CHECK(pipe(p) == 0);
    struct stat st;
    CHECK(fstat(p[0], &st) == 0 && S_ISFIFO(st.st_mode));
    child = fork();
    CHECK(child >= 0);
    if (child == 0) {
        static char block[6000];
        close(p[0]);
        memset(block, 'x', sizeof block);
        CHECK(write(p[1], "hello ", 6) == 6);
        CHECK(write(p[1], block, sizeof block) == (ssize_t)sizeof block);
        close(p[1]);
        _exit(0);
    }
    close(p[1]);
    char buf[8192];
    size_t total = 0;
    ssize_t n;
    int first = 1;
    while ((n = read(p[0], buf, sizeof buf)) > 0) {
        if (first)
            CHECK(memcmp(buf, "hello ", 6) == 0);
        first = 0;
        total += (size_t)n;
    }
    CHECK(n == 0 && total == 6006);
    CHECK(waitpid(child, &status, 0) == child && WEXITSTATUS(status) == 0);
    close(p[0]);

    /* 読み手がいない pipe への書き込みは EPIPE。 */
    int q[2];
    CHECK(pipe(q) == 0);
    close(q[0]);
    CHECK(write(q[1], "a", 1) == -1 && errno == EPIPE);
    close(q[1]);

    /* setjmp / longjmp。 */
    jmp_buf jb;
    volatile int hops = 0;
    int v = setjmp(jb);
    if (v == 0) {
        hops = 1;
        longjmp(jb, 5);
    }
    CHECK(v == 5 && hops == 1);

    puts("pkcl posix passed");
    return 0;
}
