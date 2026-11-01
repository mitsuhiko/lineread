/* lineread C shim: the pieces stable Rust cannot express.
   - rl_message(const char *fmt, ...): formats with vsnprintf and hands the
     result to the Rust side (lr_message_str in src/display.rs).
   - MB_CUR_MAX is a macro (a function call on every libc). */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

extern int lr_message_str(const char *msg);

int rl_message(const char *fmt, ...) {
  char small[512];
  va_list ap;
  va_start(ap, fmt);
  int n = vsnprintf(small, sizeof small, fmt ? fmt : "", ap);
  va_end(ap);
  if (n < 0) return lr_message_str("");
  if ((size_t)n < sizeof small) return lr_message_str(small);
  char *big = malloc((size_t)n + 1);
  if (!big) return lr_message_str(small);
  va_start(ap, fmt);
  vsnprintf(big, (size_t)n + 1, fmt, ap);
  va_end(ap);
  int r = lr_message_str(big);
  free(big);
  return r;
}

size_t lr_mb_cur_max(void) { return MB_CUR_MAX; }

FILE *lr_stdin(void) { return stdin; }
FILE *lr_stdout(void) { return stdout; }
FILE *lr_stderr(void) { return stderr; }
