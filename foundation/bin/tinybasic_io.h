/* Stream I/O respects the shell's inherited stdin/stdout and redirection. */
static int tb_batch, tb_had_error;
static char tb_output[256];
static uint32_t tb_output_used, tb_random = 1;
static void tb_flush(void) {
  if (tb_output_used) {
    fnd_status status = fnd_write(fnd_std_out(), tb_output, tb_output_used, 0);
    tb_output_used = 0;
    if (status != FND_OK)
      fnd_exit(status);
  }
}
static void putc_tb(int32_t ch) {
  if (ch == 13)
    return;
  tb_output[tb_output_used++] = (char)ch;
  if (tb_output_used == sizeof tb_output || ch == 10)
    tb_flush();
}
static void print_string(const char *s) {
  while (*s)
    putc_tb((uint8_t)*s++);
}
static void newline(void) { putc_tb(10); }
static int32_t read_line(char *dest, int32_t cap) {
  int32_t count = 0;
  int overflow = 0;
  tb_flush();
  for (;;) {
    char ch;
    uint32_t got;
    fnd_status status = fnd_read(fnd_std_in(), &ch, 1, &got);
    if (status != FND_OK)
      fnd_exit(status);
    if (!got) {
      if (!count)
        fnd_exit(FND_OK);
      ch = '\n';
    }
    if (ch == '\r')
      continue;
    if (ch == '\n') {
      dest[count] = 0;
      if (overflow) {
        print_string("LINE TOO LONG");
        newline();
        dest[0] = 0;
        return 0;
      }
      return count;
    }
    if (count < cap - 1)
      dest[count++] = ch;
    else
      overflow = 1;
  }
}
/* RND is local to this process; it never accesses the PeC RNG registers. */
static uint32_t pec_rng_rand(void) {
  tb_random = tb_random * 1664525u + 1013904223u;
  return tb_random;
}
