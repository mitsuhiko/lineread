/* lineread C shim: the pieces stable Rust cannot express.
   - rl_message(const char *fmt, ...): formats with vsnprintf and hands the
     result to the Rust side (lr_message_str in src/display.rs).
   - MB_CUR_MAX is a macro (a function call on every libc). */
#define _XOPEN_SOURCE 700
#if defined(__APPLE__)
#define _DARWIN_C_SOURCE
#endif
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

/* ------------------------------------------------------------------ */
/* Coroutines for the callback interface (src/callback.rs): the editing
   loop runs on its own stack in the same thread and is suspended whenever
   it needs input that has not arrived yet. */
#include <ucontext.h>

typedef struct lr_coro {
  ucontext_t caller;
  ucontext_t self;
  void *stack;
  size_t stack_size;
  void (*fn)(void *);
  void *arg;
  int done;
  void *top_level; /* the coroutine's lr_top_level while suspended */
} lr_coro;

static void lr_coro_entry(void) ;
static lr_coro *lr_coro_starting;

static void lr_coro_entry(void) {
  lr_coro *c = lr_coro_starting;
  c->fn(c->arg);
  c->done = 1;
  /* returns to uc_link (the caller context) */
}

lr_coro *lr_coro_new(size_t stack_size, void (*fn)(void *), void *arg) {
  lr_coro *c = calloc(1, sizeof *c);
  if (!c) return NULL;
  /* malloc'd (not mmap'd) so that leak checkers scan abandoned stacks */
  c->stack = malloc(stack_size);
  if (!c->stack) { free(c); return NULL; }
  c->stack_size = stack_size;
  c->fn = fn;
  c->arg = arg;
  getcontext(&c->self);
  c->self.uc_stack.ss_sp = c->stack;
  c->self.uc_stack.ss_size = stack_size;
  c->self.uc_link = &c->caller;
  makecontext(&c->self, lr_coro_entry, 0);
  return c;
}

#include <setjmp.h>
static sigjmp_buf *lr_top_level;
static void *lr_swap_top_level(void *t);

/* Run the coroutine until it yields or finishes.  Each side keeps its own
   abort target (lr_catch below). */
void lr_coro_resume(lr_coro *c) {
  lr_coro_starting = c;
  void *mine = lr_swap_top_level(c->top_level);
  swapcontext(&c->caller, &c->self);
  c->top_level = lr_swap_top_level(mine);
}

/* Inside the coroutine: return to the resumer. */
void lr_coro_yield(lr_coro *c) {
  swapcontext(&c->self, &c->caller);
}

int lr_coro_done(lr_coro *c) { return c->done; }

void lr_coro_free(lr_coro *c) {
  if (!c) return;
  free(c->stack);
  free(c);
}

/* ------------------------------------------------------------------ */
/* Non-local exit to the editing loop's top level (rl_abort and friends
   leave the command that called them, like the reference).  The frames
   jumped over are plain Rust/C frames without cleanup obligations. */
#include <setjmp.h>

int lr_catch(void (*fn)(void *), void *arg) {
  sigjmp_buf jb;
  sigjmp_buf *prev = lr_top_level;
  lr_top_level = &jb;
  if (sigsetjmp(jb, 0) == 0) {
    fn(arg);
    lr_top_level = prev;
    return 0;
  }
  lr_top_level = prev;
  return 1;
}

int lr_catching(void) { return lr_top_level != NULL; }

static void *lr_swap_top_level(void *t) {
  void *old = lr_top_level;
  lr_top_level = t;
  return old;
}

void lr_throw(void) {
  if (lr_top_level) siglongjmp(*lr_top_level, 1);
}
