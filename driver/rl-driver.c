/* rl-driver: script-driven test driver for GNU Readline compatible libraries.
 *
 * Compiled against the recreated headers in include/readline only and uses
 * nothing but the public API.  See docs/TESTING.md for the script language.
 *
 * Usage: rl-driver SCRIPT
 *
 * Environment:
 *   RLD_RESULTS   path of the JSON-lines results file (default: stderr)
 *   RLD_SYNC_FD   fd the driver writes sync messages to (optional)
 *   RLD_CTL_FD    fd the driver reads control commands from (optional)
 *
 * Sync protocol (driver -> runner, one message per line):
 *   I <consumed>        about to block waiting for terminal input; <consumed>
 *                       is the number of bytes read from the terminal so far
 *   A <nonce>           control command <nonce> has been processed
 *   G <signo> <count>   a signal was delivered and processed
 *   P <nonce>           please snapshot the screen now, then send "<nonce> ack"
 * Control protocol (runner -> driver):
 *   <nonce> sync | dump | dumpfull | eof | ack
 */
#define _GNU_SOURCE 1
#define _DARWIN_C_SOURCE 1
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <locale.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <sys/stat.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include <readline/readline.h>
#include <readline/history.h>

/* ------------------------------------------------------------------ */
/* small utilities                                                     */

static void die(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  fprintf(stderr, "rl-driver: ");
  vfprintf(stderr, fmt, ap);
  fprintf(stderr, "\n");
  va_end(ap);
  exit(70);
}

static void *xmalloc(size_t n) {
  void *p = malloc(n ? n : 1);
  if (!p) die("out of memory");
  return p;
}

static char *xstrdup(const char *s) {
  size_t n = strlen(s);
  char *p = xmalloc(n + 1);
  memcpy(p, s, n + 1);
  return p;
}

/* growable byte buffer */
typedef struct {
  char *p;
  size_t len, cap;
} buf_t;

static void buf_putn(buf_t *b, const char *s, size_t n) {
  if (b->len + n + 1 > b->cap) {
    size_t nc = b->cap ? b->cap * 2 : 256;
    while (nc < b->len + n + 1) nc *= 2;
    b->p = realloc(b->p, nc);
    if (!b->p) die("out of memory");
    b->cap = nc;
  }
  memcpy(b->p + b->len, s, n);
  b->len += n;
  b->p[b->len] = 0;
}
static void buf_puts(buf_t *b, const char *s) { buf_putn(b, s, strlen(s)); }
static void buf_putc(buf_t *b, char c) { buf_putn(b, &c, 1); }
static void buf_printf(buf_t *b, const char *fmt, ...) {
  char tmp[512];
  va_list ap;
  va_start(ap, fmt);
  int n = vsnprintf(tmp, sizeof tmp, fmt, ap);
  va_end(ap);
  if (n >= (int)sizeof tmp) n = sizeof tmp - 1;
  buf_putn(b, tmp, (size_t)n);
}

/* ------------------------------------------------------------------ */
/* JSON output                                                         */

/* Strings are emitted byte-exact: valid UTF-8 passes through, control
   characters use \uXXXX / short escapes, and bytes that are not part of a
   valid UTF-8 sequence are emitted as lone surrogates \udcXX (Python's
   "surrogateescape" convention). */
static int utf8_seq_len(const unsigned char *s, size_t n) {
  unsigned char c = s[0];
  int len;
  unsigned int cp;
  if (c < 0x80) return 1;
  if (c >= 0xc2 && c <= 0xdf) { len = 2; cp = c & 0x1f; }
  else if (c >= 0xe0 && c <= 0xef) { len = 3; cp = c & 0x0f; }
  else if (c >= 0xf0 && c <= 0xf4) { len = 4; cp = c & 0x07; }
  else return 0;
  if ((size_t)len > n) return 0;
  for (int i = 1; i < len; i++) {
    if ((s[i] & 0xc0) != 0x80) return 0;
    cp = (cp << 6) | (s[i] & 0x3f);
  }
  if (len == 3 && (cp < 0x800 || (cp >= 0xd800 && cp <= 0xdfff))) return 0;
  if (len == 4 && (cp < 0x10000 || cp > 0x10ffff)) return 0;
  return len;
}

static void json_strn(buf_t *b, const char *s, size_t n) {
  const unsigned char *u = (const unsigned char *)s;
  buf_putc(b, '"');
  size_t i = 0;
  while (i < n) {
    unsigned char c = u[i];
    if (c == '"') { buf_puts(b, "\\\""); i++; }
    else if (c == '\\') { buf_puts(b, "\\\\"); i++; }
    else if (c == '\n') { buf_puts(b, "\\n"); i++; }
    else if (c == '\t') { buf_puts(b, "\\t"); i++; }
    else if (c == '\r') { buf_puts(b, "\\r"); i++; }
    else if (c < 0x20 || c == 0x7f) { buf_printf(b, "\\u%04x", c); i++; }
    else if (c < 0x80) { buf_putc(b, (char)c); i++; }
    else {
      int l = utf8_seq_len(u + i, n - i);
      if (l == 0) { buf_printf(b, "\\udc%02x", c); i++; }
      else { buf_putn(b, s + i, (size_t)l); i += (size_t)l; }
    }
  }
  buf_putc(b, '"');
}

static void json_str(buf_t *b, const char *s) {
  if (!s) buf_puts(b, "null");
  else json_strn(b, s, strlen(s));
}

/* A result record is built as a JSON object incrementally. */
typedef struct {
  buf_t b;
  int nfields;
} rec_t;

static void rec_key(rec_t *r, const char *k) {
  buf_puts(&r->b, r->nfields++ ? ", " : "{");
  json_str(&r->b, k);
  buf_puts(&r->b, ": ");
}
static void rec_int(rec_t *r, const char *k, long v) { rec_key(r, k); buf_printf(&r->b, "%ld", v); }
static void rec_bool(rec_t *r, const char *k, int v) { rec_key(r, k); buf_puts(&r->b, v ? "true" : "false"); }
static void rec_str(rec_t *r, const char *k, const char *v) { rec_key(r, k); json_str(&r->b, v); }
static void rec_strn(rec_t *r, const char *k, const char *v, size_t n) { rec_key(r, k); json_strn(&r->b, v, n); }
static void rec_null(rec_t *r, const char *k) { rec_key(r, k); buf_puts(&r->b, "null"); }
/* string list: NULL-terminated */
static void rec_strv(rec_t *r, const char *k, char **v) {
  rec_key(r, k);
  if (!v) { buf_puts(&r->b, "null"); return; }
  buf_putc(&r->b, '[');
  for (int i = 0; v[i]; i++) {
    if (i) buf_puts(&r->b, ", ");
    json_str(&r->b, v[i]);
  }
  buf_putc(&r->b, ']');
}

static FILE *results_fp;

static void rec_begin(rec_t *r, const char *op) {
  memset(r, 0, sizeof *r);
  rec_str(r, "op", op);
}

static void rec_end(rec_t *r) {
  if (r->nfields == 0) buf_putc(&r->b, '{');
  buf_puts(&r->b, "}\n");
  fputs(r->b.p, results_fp);
  fflush(results_fp);
  free(r->b.p);
  r->b.p = NULL;
}

/* ------------------------------------------------------------------ */
/* script representation                                               */

typedef struct {
  char **v;      /* token values (NULL pointer = NULL token) */
  size_t *lens;  /* token byte lengths */
  int *quoted;   /* token was quoted */
  int n;
  char *text;    /* original line text (for the "op" field) */
  int lineno;
} cmd_t;

typedef struct {
  char *name;
  cmd_t *cmds;
  int n, cap;
} block_t;

static block_t *blocks;
static int nblocks, capblocks;

static block_t *block_new(const char *name) {
  if (nblocks == capblocks) {
    capblocks = capblocks ? capblocks * 2 : 16;
    blocks = realloc(blocks, sizeof *blocks * (size_t)capblocks);
  }
  block_t *b = &blocks[nblocks++];
  memset(b, 0, sizeof *b);
  b->name = xstrdup(name);
  return b;
}

static block_t *block_find(const char *name) {
  for (int i = 0; i < nblocks; i++)
    if (strcmp(blocks[i].name, name) == 0) return &blocks[i];
  return NULL;
}

static void block_add(block_t *b, cmd_t c) {
  if (b->n == b->cap) {
    b->cap = b->cap ? b->cap * 2 : 16;
    b->cmds = realloc(b->cmds, sizeof *b->cmds * (size_t)b->cap);
  }
  b->cmds[b->n++] = c;
}

static int hexval(int c) {
  if (c >= '0' && c <= '9') return c - '0';
  if (c >= 'a' && c <= 'f') return c - 'a' + 10;
  if (c >= 'A' && c <= 'F') return c - 'A' + 10;
  return -1;
}

/* Tokenize one script line.  Returns 0 on success. */
static int tokenize(const char *line, cmd_t *c, int lineno) {
  memset(c, 0, sizeof *c);
  c->lineno = lineno;
  int cap = 8;
  c->v = xmalloc(sizeof(char *) * (size_t)cap);
  c->lens = xmalloc(sizeof(size_t) * (size_t)cap);
  c->quoted = xmalloc(sizeof(int) * (size_t)cap);
  const char *p = line;
  for (;;) {
    while (*p == ' ' || *p == '\t') p++;
    if (!*p || *p == '\n') break;
    if (c->n == cap) {
      cap *= 2;
      c->v = realloc(c->v, sizeof(char *) * (size_t)cap);
      c->lens = realloc(c->lens, sizeof(size_t) * (size_t)cap);
      c->quoted = realloc(c->quoted, sizeof(int) * (size_t)cap);
    }
    buf_t t = {0};
    int quoted = 0, is_null = 0;
    if (*p == '"') {
      quoted = 1;
      p++;
      while (*p && *p != '"') {
        if (*p != '\\') { buf_putc(&t, *p++); continue; }
        p++;
        switch (*p) {
          case 'n': buf_putc(&t, '\n'); p++; break;
          case 't': buf_putc(&t, '\t'); p++; break;
          case 'r': buf_putc(&t, '\r'); p++; break;
          case 'e': buf_putc(&t, 27); p++; break;
          case 'a': buf_putc(&t, 7); p++; break;
          case 'b': buf_putc(&t, 8); p++; break;
          case 'f': buf_putc(&t, 12); p++; break;
          case 'v': buf_putc(&t, 11); p++; break;
          case 'x': {
            p++;
            int v = 0, k = 0;
            while (k < 2 && hexval(*p) >= 0) { v = v * 16 + hexval(*p++); k++; }
            buf_putc(&t, (char)v);
            break;
          }
          case '0': case '1': case '2': case '3': case '4': case '5': case '6': case '7': {
            int v = 0, k = 0;
            while (k < 3 && *p >= '0' && *p <= '7') { v = v * 8 + (*p++ - '0'); k++; }
            buf_putc(&t, (char)v);
            break;
          }
          case 0: break;
          case '\\': case '"': buf_putc(&t, *p++); break;
          default: buf_putc(&t, '\\'); buf_putc(&t, *p++); break; /* keep unknown escapes */
        }
      }
      if (*p != '"') {
        fprintf(stderr, "rl-driver: line %d: unterminated string\n", lineno);
        return -1;
      }
      p++;
    } else if (*p == '\'') {
      quoted = 1;
      p++;
      while (*p && *p != '\'') buf_putc(&t, *p++);
      if (*p != '\'') {
        fprintf(stderr, "rl-driver: line %d: unterminated string\n", lineno);
        return -1;
      }
      p++;
    } else {
      while (*p && *p != ' ' && *p != '\t' && *p != '\n') buf_putc(&t, *p++);
      if (strcmp(t.p, "NULL") == 0) is_null = 1;
    }
    if (!t.p) buf_puts(&t, "");
    c->lens[c->n] = t.len;
    c->quoted[c->n] = quoted;
    if (is_null) { free(t.p); c->v[c->n] = NULL; }
    else c->v[c->n] = t.p;
    c->n++;
  }
  return 0;
}

/* Parse a whole script into blocks; block 0 is "main". */
static void parse_script(const char *path) {
  FILE *f = fopen(path, "r");
  if (!f) die("cannot open script %s: %s", path, strerror(errno));
  block_t *main_b = block_new("main");
  int cur = 0; /* index into blocks */
  char *line = NULL;
  size_t lcap = 0;
  ssize_t len;
  int lineno = 0;
  (void)main_b;
  while ((len = getline(&line, &lcap, f)) >= 0) {
    lineno++;
    while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) line[--len] = 0;
    const char *s = line;
    while (*s == ' ' || *s == '\t') s++;
    if (!*s || *s == '#') continue;
    cmd_t c;
    if (tokenize(s, &c, lineno) != 0) exit(70);
    if (c.n == 0) continue;
    c.text = xstrdup(s);
    /* heredoc: last token <<TAG */
    if (c.v[c.n - 1] && !c.quoted[c.n - 1] && strncmp(c.v[c.n - 1], "<<", 2) == 0 && c.v[c.n - 1][2]) {
      char *tag = xstrdup(c.v[c.n - 1] + 2);
      buf_t body = {0};
      buf_puts(&body, "");
      int closed = 0;
      while ((len = getline(&line, &lcap, f)) >= 0) {
        lineno++;
        while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) line[--len] = 0;
        const char *e = line;
        while (*e == ' ' || *e == '\t') e++;
        if (strcmp(e, tag) == 0) { closed = 1; break; }
        buf_puts(&body, line);
        buf_putc(&body, '\n');
      }
      if (!closed) die("line %d: heredoc %s not terminated", c.lineno, tag);
      free(c.v[c.n - 1]);
      c.v[c.n - 1] = body.p;
      c.lens[c.n - 1] = body.len;
      c.quoted[c.n - 1] = 1;
      free(tag);
    }
    if (strcmp(c.v[0] ? c.v[0] : "", "def") == 0) {
      if (cur != 0) die("line %d: nested def", lineno);
      if (c.n != 2 || !c.v[1]) die("line %d: usage: def NAME", lineno);
      if (block_find(c.v[1])) die("line %d: block %s defined twice", lineno, c.v[1]);
      block_new(c.v[1]);
      cur = nblocks - 1;
      continue;
    }
    if (strcmp(c.v[0] ? c.v[0] : "", "end") == 0 && c.n == 1) {
      if (cur == 0) die("line %d: end without def", lineno);
      cur = 0;
      continue;
    }
    block_add(&blocks[cur], c);
  }
  if (cur != 0) die("block %s not terminated with end", blocks[cur].name);
  free(line);
  fclose(f);
}

/* ------------------------------------------------------------------ */
/* time normalization                                                  */

/* Readline stamps history entries with the wall clock.  Timestamps within
   two days of "now" are reported as "<now>" to keep results deterministic. */
static int is_recent_epoch(const char *digits, size_t n) {
  if (n < 9 || n > 11) return 0;
  long v = 0;
  for (size_t i = 0; i < n; i++) {
    if (digits[i] < '0' || digits[i] > '9') return 0;
    v = v * 10 + (digits[i] - '0');
  }
  long now = (long)time(NULL);
  return v > now - 2 * 86400 && v < now + 2 * 86400;
}

/* Replace recent epoch digit runs inside S; returns malloc'd string. */
static char *norm_times(const char *s, size_t n, size_t *outlen) {
  buf_t b = {0};
  buf_puts(&b, "");
  size_t i = 0;
  while (i < n) {
    if (s[i] >= '0' && s[i] <= '9' && (i == 0 || s[i - 1] < '0' || s[i - 1] > '9')) {
      size_t j = i;
      while (j < n && s[j] >= '0' && s[j] <= '9') j++;
      if (is_recent_epoch(s + i, j - i)) buf_puts(&b, "<now>");
      else buf_putn(&b, s + i, j - i);
      i = j;
    } else {
      buf_putc(&b, s[i++]);
    }
  }
  if (outlen) *outlen = b.len;
  return b.p;
}

static void rec_time(rec_t *r, const char *k, time_t t) {
  char tmp[32];
  snprintf(tmp, sizeof tmp, "%ld", (long)t);
  if (is_recent_epoch(tmp, strlen(tmp))) rec_str(r, k, "<now>");
  else rec_int(r, k, (long)t);
}

static void json_hist_entry(buf_t *b, HIST_ENTRY *e) {
  if (!e) { buf_puts(b, "null"); return; }
  buf_puts(b, "{\"line\": ");
  json_str(b, e->line);
  buf_puts(b, ", \"timestamp\": ");
  if (e->timestamp) {
    size_t n;
    char *t = norm_times(e->timestamp, strlen(e->timestamp), &n);
    json_strn(b, t, n);
    free(t);
  } else buf_puts(b, "null");
  buf_printf(b, ", \"data\": %ld}", (long)(intptr_t)e->data);
}

static void rec_hist_entry(rec_t *r, const char *k, HIST_ENTRY *e) {
  rec_key(r, k);
  json_hist_entry(&r->b, e);
}

/* ------------------------------------------------------------------ */
/* names of functions and keymaps                                      */

#define MAX_DEFUNS 32
static struct {
  char *name;
  char *block;
} defuns[MAX_DEFUNS];
static int ndefuns;

static const char *func_name(rl_command_func_t *f) {
  if (!f) return NULL;
  rl_initialize_funmap();
  if (funmap)
    for (int i = 0; funmap[i]; i++)
      if (funmap[i]->function == f) return funmap[i]->name;
  return "?unknown";
}

static struct {
  char *name;
  Keymap map;
} kmreg[64];
static int nkmreg;

static const char *keymap_name(Keymap m) {
  if (!m) return NULL;
  char *n = rl_get_keymap_name(m);
  if (n) return n;
  for (int i = 0; i < nkmreg; i++)
    if (kmreg[i].map == m) return kmreg[i].name;
  return "?anon";
}

static Keymap keymap_lookup(const char *name, int *ok) {
  *ok = 1;
  if (!name) return NULL;
  if (strcmp(name, "current") == 0) return rl_get_keymap();
  for (int i = 0; i < nkmreg; i++)
    if (strcmp(kmreg[i].name, name) == 0) return kmreg[i].map;
  Keymap m = rl_get_keymap_by_name(name);
  if (!m) *ok = 0;
  return m;
}

static void keymap_register(const char *name, Keymap m) {
  for (int i = 0; i < nkmreg; i++)
    if (strcmp(kmreg[i].name, name) == 0) { kmreg[i].map = m; return; }
  if (nkmreg == 64) die("too many keymaps");
  kmreg[nkmreg].name = xstrdup(name);
  kmreg[nkmreg].map = m;
  nkmreg++;
}

static void keymap_forget(Keymap m) {
  for (int i = 0; i < nkmreg; i++)
    if (kmreg[i].map == m) kmreg[i].map = NULL;
}

/* ------------------------------------------------------------------ */
/* key notation used for key arguments: \C-x \M-x \e \t \n \r \\ ^X    */

/* Parse S into bytes; returns number of bytes written to OUT. */
static int parse_keys(const char *s, size_t n, unsigned char *out, int max) {
  int k = 0;
  size_t i = 0;
  while (i < n && k < max) {
    int meta = 0, ctrl = 0;
    for (;;) {
      if (i + 2 < n && s[i] == '\\' && s[i + 1] == 'C' && s[i + 2] == '-') { ctrl = 1; i += 3; continue; }
      if (i + 2 < n && s[i] == '\\' && s[i + 1] == 'M' && s[i + 2] == '-') { meta = 1; i += 3; continue; }
      break;
    }
    if (i >= n) break;
    int c;
    if (s[i] == '\\' && i + 1 < n) {
      i++;
      switch (s[i]) {
        case 'e': c = 27; break;
        case 't': c = '\t'; break;
        case 'n': c = '\n'; break;
        case 'r': c = '\r'; break;
        case 'd': c = 127; break;
        default: c = (unsigned char)s[i]; break;
      }
      i++;
    } else if (s[i] == '^' && i + 1 < n) {
      c = (unsigned char)s[i + 1] == '?' ? 127 : (toupper((unsigned char)s[i + 1]) & 0x1f);
      i += 2;
    } else {
      c = (unsigned char)s[i++];
    }
    if (ctrl) c = (c == '?') ? 127 : (toupper(c) & 0x1f);
    if (meta) c |= 0x80;
    out[k++] = (unsigned char)c;
  }
  return k;
}

/* Integer argument: decimal/hex/octal number, or a quoted string denoting
   exactly one key (e.g. "a", "\C-a", "\M-x"). */
static int parse_int_arg(cmd_t *c, int idx, long *out) {
  const char *s = c->v[idx];
  if (!s) return -1;
  if (c->quoted[idx]) {
    unsigned char kb[8];
    int n = parse_keys(s, c->lens[idx], kb, 8);
    if (n != 1) return -1;
    *out = kb[0];
    return 0;
  }
  char *end;
  errno = 0;
  long v = strtol(s, &end, 0);
  if (*end || errno) return -1;
  *out = v;
  return 0;
}

static const char *const state_names[] = {
  "INITIALIZING", "INITIALIZED", "TERMPREPPED", "READCMD", "METANEXT",
  "DISPATCHING", "MOREINPUT", "ISEARCH", "NSEARCH", "SEARCH", "NUMERICARG",
  "MACROINPUT", "MACRODEF", "OVERWRITE", "COMPLETING", "SIGHANDLER",
  "UNDOING", "INPUTPENDING", "TTYCSAVED", "CALLBACK", "VIMOTION", "MULTIKEY",
  "VICMDONCE", "CHARSEARCH", "REDISPLAYING", "DONE", "TIMEOUT", "EOF",
  "READSTR",
};

static void rec_state(rec_t *r, const char *k, unsigned long st) {
  rec_key(r, k);
  buf_putc(&r->b, '[');
  int first = 1;
  for (unsigned i = 0; i < sizeof state_names / sizeof state_names[0]; i++) {
    if (st & (1UL << i)) {
      if (!first) buf_puts(&r->b, ", ");
      first = 0;
      json_str(&r->b, state_names[i]);
    }
  }
  for (unsigned i = sizeof state_names / sizeof state_names[0]; i < 64; i++)
    if (st & (1UL << i)) {
      if (!first) buf_puts(&r->b, ", ");
      first = 0;
      buf_printf(&r->b, "\"0x%lx\"", 1UL << i);
    }
  buf_putc(&r->b, ']');
}

static void rec_undo(rec_t *r, const char *k) {
  static const char *const what[] = {"delete", "insert", "begin", "end"};
  rec_key(r, k);
  buf_putc(&r->b, '[');
  int i = 0;
  for (UNDO_LIST *u = rl_undo_list; u && i < 1000; u = u->next, i++) {
    if (i) buf_puts(&r->b, ", ");
    buf_puts(&r->b, "[");
    unsigned w = (unsigned)u->what;
    if (w < 4) json_str(&r->b, what[w]);
    else buf_printf(&r->b, "%u", w);
    buf_printf(&r->b, ", %d, %d, ", u->start, u->end);
    json_str(&r->b, u->text);
    buf_puts(&r->b, "]");
  }
  buf_putc(&r->b, ']');
}

/* ------------------------------------------------------------------ */
/* state dump                                                          */

static void do_dump(const char *label, int full) {
  rec_t r;
  rec_begin(&r, full ? "dumpfull" : "dump");
  if (label) rec_str(&r, "label", label);
  if (rl_line_buffer) rec_strn(&r, "line", rl_line_buffer, rl_end >= 0 ? (size_t)rl_end : 0);
  else rec_null(&r, "line");
  rec_int(&r, "point", rl_point);
  rec_int(&r, "end", rl_end);
  rec_int(&r, "mark", rl_mark);
  rec_str(&r, "mode", rl_editing_mode == 0 ? "vi" : rl_editing_mode == 1 ? "emacs" : "?");
  rec_str(&r, "keymap", keymap_name(rl_get_keymap()));
  rec_int(&r, "explicit_arg", rl_explicit_arg);
  rec_int(&r, "numeric_arg", rl_numeric_arg);
  rec_int(&r, "insert_mode", rl_insert_mode);
  if (full) {
    rec_state(&r, "state", rl_readline_state);
    rec_undo(&r, "undo");
    rec_str(&r, "prompt", rl_prompt);
    rec_str(&r, "display_prompt", rl_display_prompt);
    rec_str(&r, "last_func", func_name(rl_last_func));
    rec_int(&r, "done", rl_done);
    rec_int(&r, "eof_found", rl_eof_found);
    rec_int(&r, "pending_input", rl_pending_input);
    rec_int(&r, "mark_active", rl_mark_active_p());
    rec_int(&r, "executing_key", rl_executing_key);
    if (rl_executing_keyseq && rl_key_sequence_length >= 0)
      rec_strn(&r, "executing_keyseq", rl_executing_keyseq, (size_t)rl_key_sequence_length);
    else rec_null(&r, "executing_keyseq");
    rec_str(&r, "executing_macro", rl_executing_macro);
    rec_int(&r, "history_length", history_length);
    rec_int(&r, "history_base", history_base);
    rec_int(&r, "history_pos", where_history());
  }
  rec_end(&r);
}

/* ------------------------------------------------------------------ */
/* synchronisation with the runner                                     */

static int sync_fd = -1, ctl_fd = -1;
static int sigpipe_fds[2] = {-1, -1};
static long consumed;          /* bytes read from the terminal */
static int eof_flag;           /* runner says: no more input */
static volatile sig_atomic_t sig_count[NSIG];
static long sig_reported[NSIG];
static int sig_mode[NSIG];     /* 0 record, 1 default, 2 ignore */

static void sync_write(const char *fmt, ...) {
  if (sync_fd < 0) return;
  char tmp[128];
  va_list ap;
  va_start(ap, fmt);
  int n = vsnprintf(tmp, sizeof tmp, fmt, ap);
  va_end(ap);
  ssize_t w;
  do w = write(sync_fd, tmp, (size_t)n); while (w < 0 && errno == EINTR);
}

static void drv_sig_handler(int sig) {
  int e = errno;
  if (sig > 0 && sig < NSIG) sig_count[sig]++;
  if (sigpipe_fds[1] >= 0) {
    char c = (char)sig;
    ssize_t w = write(sigpipe_fds[1], &c, 1);
    (void)w;
  }
  errno = e;
}

static void install_sig(int sig, int mode) {
  struct sigaction sa;
  memset(&sa, 0, sizeof sa);
  sigemptyset(&sa.sa_mask);
  sig_mode[sig] = mode;
  if (mode == 0) sa.sa_handler = drv_sig_handler;
  else if (mode == 1) sa.sa_handler = SIG_DFL;
  else sa.sa_handler = SIG_IGN;
  sigaction(sig, &sa, NULL);
}

static int fd_readable(int fd) {
  if (fd < 0) return 0;
  fd_set fds;
  struct timeval tv = {0, 0};
  FD_ZERO(&fds);
  FD_SET(fd, &fds);
  int r;
  do r = select(fd + 1, &fds, NULL, NULL, &tv); while (r < 0 && errno == EINTR);
  return r > 0;
}

static void run_block_named(const char *name);
static char *onsig_block;   /* callback mode: block run when a signal arrives */
static int in_callback_wait;

static const char *sig_name(int s) {
  switch (s) {
    case SIGINT: return "INT";
    case SIGQUIT: return "QUIT";
    case SIGWINCH: return "WINCH";
    case SIGTSTP: return "TSTP";
    case SIGTERM: return "TERM";
    case SIGHUP: return "HUP";
    case SIGALRM: return "ALRM";
    case SIGCONT: return "CONT";
    case SIGTTIN: return "TTIN";
    case SIGTTOU: return "TTOU";
    case SIGUSR1: return "USR1";
    case SIGUSR2: return "USR2";
    default: return "?";
  }
}

static void report_signals(void) {
  for (int s = 1; s < NSIG; s++) {
    long c = sig_count[s];
    if (c != sig_reported[s]) {
      sig_reported[s] = c;
      rec_t r;
      rec_begin(&r, "signal");
      rec_str(&r, "sig", sig_name(s));
      rec_end(&r);
      if (in_callback_wait && onsig_block) run_block_named(onsig_block);
      sync_write("G %d %ld\n", s, c);
    }
  }
}

static void ctl_command(char *line) {
  char *sp = strchr(line, ' ');
  if (!sp) return;
  *sp = 0;
  const char *nonce = line, *cmd = sp + 1;
  if (strcmp(cmd, "dump") == 0) do_dump(NULL, 0);
  else if (strcmp(cmd, "dumpfull") == 0) do_dump(NULL, 1);
  else if (strcmp(cmd, "eof") == 0) eof_flag = 1;
  sync_write("A %s\n", nonce);
}

static char ctl_buf[4096];
static size_t ctl_len;

/* Read available control data; process complete lines.  If WANT_ACK is
   non-NULL, return 1 when "<WANT_ACK> ack" was seen. */
static int ctl_read(const char *want_ack) {
  int got = 0;
  ssize_t n = read(ctl_fd, ctl_buf + ctl_len, sizeof ctl_buf - 1 - ctl_len);
  if (n <= 0) {
    if (n == 0) { close(ctl_fd); ctl_fd = -1; eof_flag = 1; }
    return 0;
  }
  ctl_len += (size_t)n;
  ctl_buf[ctl_len] = 0;
  char *nl;
  while ((nl = memchr(ctl_buf, '\n', ctl_len))) {
    *nl = 0;
    char *sp = strchr(ctl_buf, ' ');
    if (want_ack && sp && strcmp(sp + 1, "ack") == 0 && strncmp(ctl_buf, want_ack, (size_t)(sp - ctl_buf)) == 0)
      got = 1;
    else if (sp && strcmp(sp + 1, "ack") != 0)
      ctl_command(ctl_buf);
    size_t used = (size_t)(nl - ctl_buf) + 1;
    memmove(ctl_buf, nl + 1, ctl_len - used);
    ctl_len -= used;
  }
  return got;
}

/* Block until terminal input is available (1) or input is exhausted (0).
   While waiting: report idleness, process control commands and signals. */
static int wait_for_input(void) {
  for (;;) {
    report_signals();
    if (fd_readable(0)) return 1;
    if (eof_flag) return 0;
    if (sync_fd < 0) return 1; /* standalone: let the read block */
    sync_write("I %ld\n", consumed);
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(0, &fds);
    int mx = 0;
    if (ctl_fd >= 0) { FD_SET(ctl_fd, &fds); if (ctl_fd > mx) mx = ctl_fd; }
    if (sigpipe_fds[0] >= 0) { FD_SET(sigpipe_fds[0], &fds); if (sigpipe_fds[0] > mx) mx = sigpipe_fds[0]; }
    int r = select(mx + 1, &fds, NULL, NULL, NULL);
    if (r < 0) {
      if (errno != EINTR) die("select: %s", strerror(errno));
      rl_check_signals();
      if (!in_callback_wait && rl_signal_event_hook) (*rl_signal_event_hook)();
      continue;
    }
    if (sigpipe_fds[0] >= 0 && FD_ISSET(sigpipe_fds[0], &fds)) {
      char tmp[64];
      ssize_t n = read(sigpipe_fds[0], tmp, sizeof tmp);
      (void)n;
      rl_check_signals();
    }
    if (ctl_fd >= 0 && FD_ISSET(ctl_fd, &fds)) ctl_read(NULL);
  }
}

static int drv_getc(FILE *f) {
  if (!wait_for_input()) return EOF;
  int c = rl_getc(f);
  if (c >= 0) consumed++;
  return c;
}

/* Ask the runner to take a screen snapshot now (used by driver-snap). */
static void request_snapshot(void) {
  static int nonce = 0;
  char tag[32];
  fflush(rl_outstream ? rl_outstream : stdout);
  if (sync_fd < 0 || ctl_fd < 0) return;
  snprintf(tag, sizeof tag, "s%d", ++nonce);
  sync_write("P %s\n", tag);
  for (;;) {
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(ctl_fd, &fds);
    int r = select(ctl_fd + 1, &fds, NULL, NULL, NULL);
    if (r < 0 && errno == EINTR) continue;
    if (r < 0) die("select: %s", strerror(errno));
    if (ctl_read(tag)) return;
    if (ctl_fd < 0) return;
  }
}

/* ------------------------------------------------------------------ */
/* completion word list, ignore list, tilde table                      */

static char **words;
static int nwords, words_icase;
static char **ign_words;
static int nign_words;
static char **tmap_keys, **tmap_vals;
static int ntmap;

static char *words_gen(const char *text, int state) {
  static int idx;
  static size_t len;
  if (!state) { idx = 0; len = strlen(text); }
  while (idx < nwords) {
    const char *w = words[idx++];
    int m = words_icase ? strncasecmp(w, text, len) : strncmp(w, text, len);
    if (m == 0) return xstrdup(w);
  }
  return NULL;
}

static char *empty_gen(const char *text, int state) {
  (void)text; (void)state;
  return NULL;
}

static rl_compentry_func_t *generator_by_name(const char *n, int *ok) {
  *ok = 1;
  if (!n) return NULL;
  if (strcmp(n, "words") == 0) return words_gen;
  if (strcmp(n, "filename") == 0) return rl_filename_completion_function;
  if (strcmp(n, "username") == 0) return rl_username_completion_function;
  if (strcmp(n, "empty") == 0) return empty_gen;
  *ok = 0;
  return NULL;
}

static const char *generator_name(rl_compentry_func_t *f) {
  if (!f) return NULL;
  if (f == words_gen) return "words";
  if (f == rl_filename_completion_function) return "filename";
  if (f == rl_username_completion_function) return "username";
  if (f == empty_gen) return "empty";
  return "?unknown";
}

/* ------------------------------------------------------------------ */
/* hooks                                                               */

typedef struct hctx {
  const char *hook;
  buf_t args;           /* ", "-joined JSON fields */
  int has_ret, ret_null;
  long ret_i;
  char *ret_s;
  char **matches;       /* completion display / ignore hooks */
  int nmatches, maxlen;
  char **strarg;        /* directory hooks: replace_arg target */
  struct hctx *prev;
} hctx_t;

static hctx_t *hctx;

static void hctx_arg_str(hctx_t *c, const char *k, const char *v) {
  if (c->args.len) buf_puts(&c->args, ", ");
  json_str(&c->args, k);
  buf_puts(&c->args, ": ");
  json_str(&c->args, v);
}
static void hctx_arg_int(hctx_t *c, const char *k, long v) {
  if (c->args.len) buf_puts(&c->args, ", ");
  json_str(&c->args, k);
  buf_printf(&c->args, ": %ld", v);
}
static void hctx_arg_strv(hctx_t *c, const char *k, char **v, int n) {
  if (c->args.len) buf_puts(&c->args, ", ");
  json_str(&c->args, k);
  buf_puts(&c->args, ": [");
  for (int i = 0; v && (n < 0 ? v[i] != NULL : i < n); i++) {
    if (i) buf_puts(&c->args, ", ");
    json_str(&c->args, v[i]);
  }
  buf_puts(&c->args, "]");
}

static void emit_hook_args(hctx_t *c) {
  rec_t r;
  rec_begin(&r, "hook");
  rec_str(&r, "hook", c->hook);
  if (c->args.len) { buf_puts(&r.b, ", "); buf_puts(&r.b, c->args.p); }
  rec_end(&r);
}

enum {
  H_STARTUP, H_PREINPUT, H_EVENT, H_SIGEVENT, H_TIMEOUTEV, H_INPUTAVAIL,
  H_REDISPLAY, H_PREPTERM, H_DEPREPTERM, H_MACRODISP,
  H_ENTRY, H_MENUENTRY, H_IGNORE, H_ATTEMPT, H_WORDBREAK,
  H_DIRCOMP, H_DIRREWRITE, H_STAT, H_FNREWRITE, H_COMPREWRITE,
  H_COMPDISP, H_QUOTE, H_DEQUOTE, H_QUOTEDP,
  H_TILDEPRE, H_TILDEFAIL, H_INHIBIT, H_COUNT
};

typedef struct {
  const char *name;
  void **addr;
  void *tramp;
  void *dflt;
  char *block;
  int log;
} hook_t;

static hook_t hooks[H_COUNT];

static void hook_run(int id, hctx_t *c) {
  hook_t *h = &hooks[id];
  c->hook = h->name;
  c->prev = hctx;
  hctx = c;
  if (h->log) emit_hook_args(c);
  if (h->block) run_block_named(h->block);
  hctx = c->prev;
}

static void hctx_free(hctx_t *c) {
  free(c->args.p);
  free(c->ret_s);
}

static int hctx_ret_int(hctx_t *c, int dflt) {
  return c->has_ret && !c->ret_null ? (int)c->ret_i : dflt;
}

#define HCTX(c) hctx_t c; memset(&c, 0, sizeof c)

static int t_hook_common(int id) {
  HCTX(c);
  hook_run(id, &c);
  int r = hctx_ret_int(&c, 0);
  hctx_free(&c);
  return r;
}
static int t_startup(void) { return t_hook_common(H_STARTUP); }
static int t_preinput(void) { return t_hook_common(H_PREINPUT); }
static int t_sigevent(void) { return t_hook_common(H_SIGEVENT); }
static int t_timeoutev(void) { return t_hook_common(H_TIMEOUTEV); }
static int t_inputavail(void) { return t_hook_common(H_INPUTAVAIL); }
static int t_event(void) {
  /* readline polls with the event hook instead of calling rl_getc_function
     while it is set, so keep the runner protocol alive from here */
  report_signals();
  if (!fd_readable(0)) {
    if (eof_flag) rl_stuff_char(EOF);
    else {
      sync_write("I %ld\n", consumed);
      if (fd_readable(ctl_fd)) ctl_read(NULL);
    }
  }
  int r = 0;
  if (hooks[H_EVENT].block || hooks[H_EVENT].log) r = t_hook_common(H_EVENT);
  return r;
}

static void t_redisplay(void) {
  HCTX(c);
  hook_run(H_REDISPLAY, &c);
  if (!hooks[H_REDISPLAY].block) rl_redisplay();
  hctx_free(&c);
}
static void t_prepterm(int meta) {
  HCTX(c);
  hctx_arg_int(&c, "meta", meta);
  hook_run(H_PREPTERM, &c);
  if (!hooks[H_PREPTERM].block) rl_prep_terminal(meta);
  hctx_free(&c);
}
static void t_deprepterm(void) {
  HCTX(c);
  hook_run(H_DEPREPTERM, &c);
  if (!hooks[H_DEPREPTERM].block) rl_deprep_terminal();
  hctx_free(&c);
}
static void t_macrodisp(const char *kseq, const char *value, int readable, const char *prefix) {
  HCTX(c);
  hctx_arg_str(&c, "keyseq", kseq);
  hctx_arg_str(&c, "value", value);
  hctx_arg_int(&c, "readable", readable);
  hctx_arg_str(&c, "prefix", prefix);
  hook_run(H_MACRODISP, &c);
  hctx_free(&c);
}

static char **t_attempt(const char *text, int start, int end) {
  HCTX(c);
  hctx_arg_str(&c, "text", text);
  hctx_arg_int(&c, "start", start);
  hctx_arg_int(&c, "end", end);
  hook_run(H_ATTEMPT, &c);
  char **m = NULL;
  if (c.has_ret && c.ret_null) m = NULL;
  else {
    int ok;
    rl_compentry_func_t *g = c.has_ret ? generator_by_name(c.ret_s, &ok) : words_gen;
    if (!c.has_ret || ok) m = g ? rl_completion_matches(text, g) : NULL;
  }
  hctx_free(&c);
  return m;
}

static int t_ignore(char **matches) {
  HCTX(c);
  hctx_arg_strv(&c, "matches", matches, -1);
  c.matches = matches;
  /* drop entries listed by ignore_words (never the common prefix, [0]) */
  if (matches && nign_words) {
    int j = 1;
    for (int i = 1; matches[i]; i++) {
      int drop = 0;
      for (int k = 0; k < nign_words; k++)
        if (strcmp(matches[i], ign_words[k]) == 0) drop = 1;
      if (drop) free(matches[i]);
      else matches[j++] = matches[i];
    }
    matches[j] = NULL;
  }
  hook_run(H_IGNORE, &c);
  int r = hctx_ret_int(&c, 0);
  hctx_free(&c);
  return r;
}

static char *wordbreak_keep;
static char *t_wordbreak(void) {
  HCTX(c);
  hook_run(H_WORDBREAK, &c);
  char *r = NULL;
  if (c.has_ret && !c.ret_null && c.ret_s) {
    free(wordbreak_keep);
    wordbreak_keep = xstrdup(c.ret_s);
    r = wordbreak_keep;
  }
  hctx_free(&c);
  return r;
}

static int t_dirhook_common(int id, char **dirname) {
  HCTX(c);
  hctx_arg_str(&c, "dirname", dirname ? *dirname : NULL);
  c.strarg = dirname;
  hook_run(id, &c);
  int r = hctx_ret_int(&c, 0);
  hctx_free(&c);
  return r;
}
static int t_dircomp(char **d) { return t_dirhook_common(H_DIRCOMP, d); }
static int t_dirrewrite(char **d) { return t_dirhook_common(H_DIRREWRITE, d); }
static int t_stat(char **d) { return t_dirhook_common(H_STAT, d); }

static char *t_rewrite_common(int id, char *s, int len) {
  HCTX(c);
  hctx_arg_str(&c, "text", s);
  hctx_arg_int(&c, "len", len);
  hook_run(id, &c);
  char *r = s;
  if (c.has_ret) r = c.ret_null ? NULL : xstrdup(c.ret_s ? c.ret_s : "");
  hctx_free(&c);
  return r;
}
static char *t_fnrewrite(char *s, int len) { return t_rewrite_common(H_FNREWRITE, s, len); }
static char *t_comprewrite(char *s, int len) { return t_rewrite_common(H_COMPREWRITE, s, len); }

static char **compdisp_matches;
static int compdisp_len, compdisp_max;
static void t_compdisp(char **matches, int num, int maxlen) {
  HCTX(c);
  hctx_arg_strv(&c, "matches", matches, num + 1);
  hctx_arg_int(&c, "num_matches", num);
  hctx_arg_int(&c, "max_length", maxlen);
  c.matches = matches;
  c.nmatches = num;
  c.maxlen = maxlen;
  char **om = compdisp_matches;
  int ol = compdisp_len, ox = compdisp_max;
  compdisp_matches = matches;
  compdisp_len = num;
  compdisp_max = maxlen;
  hook_run(H_COMPDISP, &c);
  compdisp_matches = om;
  compdisp_len = ol;
  compdisp_max = ox;
  hctx_free(&c);
}

static char *t_quote(char *text, int match_type, char *qcp) {
  HCTX(c);
  hctx_arg_str(&c, "text", text);
  hctx_arg_int(&c, "match_type", match_type);
  if (qcp) {
    char q[2] = {*qcp, 0};
    hctx_arg_str(&c, "quote_char", q);
  } else hctx_arg_str(&c, "quote_char", NULL);
  hook_run(H_QUOTE, &c);
  char *r;
  if (c.has_ret) r = c.ret_null ? NULL : xstrdup(c.ret_s ? c.ret_s : "");
  else r = xstrdup(text);
  hctx_free(&c);
  return r;
}

static char *t_dequote(char *text, int qc) {
  HCTX(c);
  hctx_arg_str(&c, "text", text);
  hctx_arg_int(&c, "quote_char", qc);
  hook_run(H_DEQUOTE, &c);
  char *r;
  if (c.has_ret) r = c.ret_null ? NULL : xstrdup(c.ret_s ? c.ret_s : "");
  else {
    /* default: drop backslashes */
    r = xmalloc(strlen(text) + 1);
    char *o = r;
    for (const char *p = text; *p; p++) {
      if (*p == '\\' && p[1]) p++;
      *o++ = *p;
    }
    *o = 0;
  }
  hctx_free(&c);
  return r;
}

static int t_quotedp(char *text, int index) {
  HCTX(c);
  hctx_arg_str(&c, "text", text);
  hctx_arg_int(&c, "index", index);
  hook_run(H_QUOTEDP, &c);
  int dflt = (index > 0 && text[index - 1] == '\\');
  int r = hctx_ret_int(&c, dflt);
  hctx_free(&c);
  return r;
}

static char *t_tilde_common(int id, char *word) {
  HCTX(c);
  hctx_arg_str(&c, "word", word);
  hook_run(id, &c);
  char *r = NULL;
  if (c.has_ret) r = c.ret_null ? NULL : xstrdup(c.ret_s ? c.ret_s : "");
  else
    for (int i = 0; i < ntmap; i++)
      if (strcmp(tmap_keys[i], word) == 0) { r = xstrdup(tmap_vals[i]); break; }
  hctx_free(&c);
  return r;
}
static char *t_tildepre(char *w) { return t_tilde_common(H_TILDEPRE, w); }
static char *t_tildefail(char *w) { return t_tilde_common(H_TILDEFAIL, w); }

static int t_inhibit(char *s, int i) {
  HCTX(c);
  hctx_arg_str(&c, "string", s);
  hctx_arg_int(&c, "index", i);
  hook_run(H_INHIBIT, &c);
  int r = hctx_ret_int(&c, 0);
  hctx_free(&c);
  return r;
}

#define HOOKDEF(id, var, tr) hooks[id].name = #var; hooks[id].addr = (void **)&var; hooks[id].tramp = (void *)tr; hooks[id].dflt = (void *)var
static void hooks_init(void) {
  HOOKDEF(H_STARTUP, rl_startup_hook, t_startup);
  HOOKDEF(H_PREINPUT, rl_pre_input_hook, t_preinput);
  HOOKDEF(H_EVENT, rl_event_hook, t_event);
  HOOKDEF(H_SIGEVENT, rl_signal_event_hook, t_sigevent);
  HOOKDEF(H_TIMEOUTEV, rl_timeout_event_hook, t_timeoutev);
  HOOKDEF(H_INPUTAVAIL, rl_input_available_hook, t_inputavail);
  HOOKDEF(H_REDISPLAY, rl_redisplay_function, t_redisplay);
  HOOKDEF(H_PREPTERM, rl_prep_term_function, t_prepterm);
  HOOKDEF(H_DEPREPTERM, rl_deprep_term_function, t_deprepterm);
  HOOKDEF(H_MACRODISP, rl_macro_display_hook, t_macrodisp);
  HOOKDEF(H_ENTRY, rl_completion_entry_function, NULL);
  HOOKDEF(H_MENUENTRY, rl_menu_completion_entry_function, NULL);
  HOOKDEF(H_IGNORE, rl_ignore_some_completions_function, t_ignore);
  HOOKDEF(H_ATTEMPT, rl_attempted_completion_function, t_attempt);
  HOOKDEF(H_WORDBREAK, rl_completion_word_break_hook, t_wordbreak);
  HOOKDEF(H_DIRCOMP, rl_directory_completion_hook, t_dircomp);
  HOOKDEF(H_DIRREWRITE, rl_directory_rewrite_hook, t_dirrewrite);
  HOOKDEF(H_STAT, rl_filename_stat_hook, t_stat);
  HOOKDEF(H_FNREWRITE, rl_filename_rewrite_hook, t_fnrewrite);
  HOOKDEF(H_COMPREWRITE, rl_completion_rewrite_hook, t_comprewrite);
  HOOKDEF(H_COMPDISP, rl_completion_display_matches_hook, t_compdisp);
  HOOKDEF(H_QUOTE, rl_filename_quoting_function, t_quote);
  HOOKDEF(H_DEQUOTE, rl_filename_dequoting_function, t_dequote);
  HOOKDEF(H_QUOTEDP, rl_char_is_quoted_p, t_quotedp);
  HOOKDEF(H_TILDEPRE, tilde_expansion_preexpansion_hook, t_tildepre);
  HOOKDEF(H_TILDEFAIL, tilde_expansion_failure_hook, t_tildefail);
  HOOKDEF(H_INHIBIT, history_inhibit_expansion_function, t_inhibit);
}

static hook_t *hook_find(const char *name) {
  for (int i = 0; i < H_COUNT; i++)
    if (strcmp(hooks[i].name, name) == 0) return &hooks[i];
  return NULL;
}

/* Values: NULL, default, log, script:BLOCK, tramp (no block, no log),
   and for completion entry functions words|filename|username|empty. */
static const char *hook_set(hook_t *h, const char *val) {
  free(h->block);
  h->block = NULL;
  h->log = 0;
  if (!val) { *h->addr = NULL; return NULL; }
  if (strcmp(val, "default") == 0) { *h->addr = h->dflt; return NULL; }
  if (h == &hooks[H_ENTRY] || h == &hooks[H_MENUENTRY]) {
    int ok;
    rl_compentry_func_t *g = generator_by_name(val, &ok);
    if (!ok) return "unknown generator";
    *h->addr = (void *)g;
    return NULL;
  }
  if (!h->tramp) return "hook has no trampoline";
  if (strncmp(val, "script:", 7) == 0) {
    if (!block_find(val + 7)) return "no such block";
    h->block = xstrdup(val + 7);
  } else if (strcmp(val, "log") == 0) {
    h->log = 1;
  } else if (strcmp(val, "tramp") != 0 && strcmp(val, "words") != 0 &&
             strcmp(val, "table") != 0 && strcmp(val, "ignore_words") != 0 &&
             strcmp(val, "backslash") != 0) {
    return "unknown hook value";
  }
  *h->addr = h->tramp;
  return NULL;
}

static const char *hook_get(hook_t *h) {
  void *v = *h->addr;
  if (!v) return NULL;
  if (v == h->dflt) return "default";
  if (h == &hooks[H_ENTRY] || h == &hooks[H_MENUENTRY]) return generator_name((rl_compentry_func_t *)v);
  if (v == h->tramp) return h->block ? "script" : h->log ? "log" : "tramp";
  return "?unknown";
}

/* ------------------------------------------------------------------ */
/* bindable functions defined by the script                            */

static int defun_run(int idx, int count, int key) {
  HCTX(c);
  hctx_arg_str(&c, "name", defuns[idx].name);
  hctx_arg_int(&c, "count", count);
  hctx_arg_int(&c, "key", key);
  c.hook = defuns[idx].name;
  c.prev = hctx;
  hctx = &c;
  run_block_named(defuns[idx].block);
  hctx = c.prev;
  int r = hctx_ret_int(&c, 0);
  hctx_free(&c);
  return r;
}

#define DF(n) static int defun_##n(int count, int key) { return defun_run(n, count, key); }
DF(0) DF(1) DF(2) DF(3) DF(4) DF(5) DF(6) DF(7) DF(8) DF(9) DF(10) DF(11) DF(12) DF(13) DF(14) DF(15)
DF(16) DF(17) DF(18) DF(19) DF(20) DF(21) DF(22) DF(23) DF(24) DF(25) DF(26) DF(27) DF(28) DF(29) DF(30) DF(31)
static rl_command_func_t *const defun_tramps[MAX_DEFUNS] = {
  defun_0, defun_1, defun_2, defun_3, defun_4, defun_5, defun_6, defun_7, defun_8, defun_9, defun_10,
  defun_11, defun_12, defun_13, defun_14, defun_15, defun_16, defun_17, defun_18, defun_19, defun_20,
  defun_21, defun_22, defun_23, defun_24, defun_25, defun_26, defun_27, defun_28, defun_29, defun_30,
  defun_31,
};

static int fn_driver_dump(int count, int key) { (void)count; (void)key; do_dump(NULL, 0); return 0; }
static int fn_driver_dump_full(int count, int key) { (void)count; (void)key; do_dump(NULL, 1); return 0; }
static int fn_driver_snap(int count, int key) { (void)count; (void)key; request_snapshot(); return 0; }

static rl_command_func_t *func_lookup(const char *name, int *ok) {
  *ok = 1;
  if (!name) return NULL;
  rl_command_func_t *f = rl_named_function(name);
  if (!f) *ok = 0;
  return f;
}

/* ------------------------------------------------------------------ */
/* generic API call table                                              */

typedef union {
  long i;
  void *p;
  time_t t;
} val_t;

/* generated: call FN with the argument shape SHAPE ("R:AAA") */
static int call_shape(const char *shape, void (*fn)(void), val_t *a, val_t *r) {
  if (strcmp(shape, "v:") == 0) { ((void (*)(void))fn)(); return 0; }
  if (strcmp(shape, "I:") == 0) { r->i = ((int (*)(void))fn)(); return 0; }
  if (strcmp(shape, "P:") == 0) { r->p = ((void * (*)(void))fn)(); return 0; }
  if (strcmp(shape, "T:") == 0) { r->t = ((time_t (*)(void))fn)(); return 0; }
  if (strcmp(shape, "v:I") == 0) { ((void (*)(int))fn)((int)a[0].i); return 0; }
  if (strcmp(shape, "I:I") == 0) { r->i = ((int (*)(int))fn)((int)a[0].i); return 0; }
  if (strcmp(shape, "P:I") == 0) { r->p = ((void * (*)(int))fn)((int)a[0].i); return 0; }
  if (strcmp(shape, "T:I") == 0) { r->t = ((time_t (*)(int))fn)((int)a[0].i); return 0; }
  if (strcmp(shape, "v:P") == 0) { ((void (*)(void *))fn)(a[0].p); return 0; }
  if (strcmp(shape, "I:P") == 0) { r->i = ((int (*)(void *))fn)(a[0].p); return 0; }
  if (strcmp(shape, "P:P") == 0) { r->p = ((void * (*)(void *))fn)(a[0].p); return 0; }
  if (strcmp(shape, "T:P") == 0) { r->t = ((time_t (*)(void *))fn)(a[0].p); return 0; }
  if (strcmp(shape, "v:II") == 0) { ((void (*)(int, int))fn)((int)a[0].i, (int)a[1].i); return 0; }
  if (strcmp(shape, "I:II") == 0) { r->i = ((int (*)(int, int))fn)((int)a[0].i, (int)a[1].i); return 0; }
  if (strcmp(shape, "P:II") == 0) { r->p = ((void * (*)(int, int))fn)((int)a[0].i, (int)a[1].i); return 0; }
  if (strcmp(shape, "T:II") == 0) { r->t = ((time_t (*)(int, int))fn)((int)a[0].i, (int)a[1].i); return 0; }
  if (strcmp(shape, "v:IP") == 0) { ((void (*)(int, void *))fn)((int)a[0].i, a[1].p); return 0; }
  if (strcmp(shape, "I:IP") == 0) { r->i = ((int (*)(int, void *))fn)((int)a[0].i, a[1].p); return 0; }
  if (strcmp(shape, "P:IP") == 0) { r->p = ((void * (*)(int, void *))fn)((int)a[0].i, a[1].p); return 0; }
  if (strcmp(shape, "T:IP") == 0) { r->t = ((time_t (*)(int, void *))fn)((int)a[0].i, a[1].p); return 0; }
  if (strcmp(shape, "v:PI") == 0) { ((void (*)(void *, int))fn)(a[0].p, (int)a[1].i); return 0; }
  if (strcmp(shape, "I:PI") == 0) { r->i = ((int (*)(void *, int))fn)(a[0].p, (int)a[1].i); return 0; }
  if (strcmp(shape, "P:PI") == 0) { r->p = ((void * (*)(void *, int))fn)(a[0].p, (int)a[1].i); return 0; }
  if (strcmp(shape, "T:PI") == 0) { r->t = ((time_t (*)(void *, int))fn)(a[0].p, (int)a[1].i); return 0; }
  if (strcmp(shape, "v:PP") == 0) { ((void (*)(void *, void *))fn)(a[0].p, a[1].p); return 0; }
  if (strcmp(shape, "I:PP") == 0) { r->i = ((int (*)(void *, void *))fn)(a[0].p, a[1].p); return 0; }
  if (strcmp(shape, "P:PP") == 0) { r->p = ((void * (*)(void *, void *))fn)(a[0].p, a[1].p); return 0; }
  if (strcmp(shape, "T:PP") == 0) { r->t = ((time_t (*)(void *, void *))fn)(a[0].p, a[1].p); return 0; }
  if (strcmp(shape, "v:III") == 0) { ((void (*)(int, int, int))fn)((int)a[0].i, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "I:III") == 0) { r->i = ((int (*)(int, int, int))fn)((int)a[0].i, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "P:III") == 0) { r->p = ((void * (*)(int, int, int))fn)((int)a[0].i, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "T:III") == 0) { r->t = ((time_t (*)(int, int, int))fn)((int)a[0].i, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "v:IIP") == 0) { ((void (*)(int, int, void *))fn)((int)a[0].i, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "I:IIP") == 0) { r->i = ((int (*)(int, int, void *))fn)((int)a[0].i, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "P:IIP") == 0) { r->p = ((void * (*)(int, int, void *))fn)((int)a[0].i, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "T:IIP") == 0) { r->t = ((time_t (*)(int, int, void *))fn)((int)a[0].i, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "v:IPI") == 0) { ((void (*)(int, void *, int))fn)((int)a[0].i, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "I:IPI") == 0) { r->i = ((int (*)(int, void *, int))fn)((int)a[0].i, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "P:IPI") == 0) { r->p = ((void * (*)(int, void *, int))fn)((int)a[0].i, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "T:IPI") == 0) { r->t = ((time_t (*)(int, void *, int))fn)((int)a[0].i, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "v:IPP") == 0) { ((void (*)(int, void *, void *))fn)((int)a[0].i, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "I:IPP") == 0) { r->i = ((int (*)(int, void *, void *))fn)((int)a[0].i, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "P:IPP") == 0) { r->p = ((void * (*)(int, void *, void *))fn)((int)a[0].i, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "T:IPP") == 0) { r->t = ((time_t (*)(int, void *, void *))fn)((int)a[0].i, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "v:PII") == 0) { ((void (*)(void *, int, int))fn)(a[0].p, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "I:PII") == 0) { r->i = ((int (*)(void *, int, int))fn)(a[0].p, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "P:PII") == 0) { r->p = ((void * (*)(void *, int, int))fn)(a[0].p, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "T:PII") == 0) { r->t = ((time_t (*)(void *, int, int))fn)(a[0].p, (int)a[1].i, (int)a[2].i); return 0; }
  if (strcmp(shape, "v:PIP") == 0) { ((void (*)(void *, int, void *))fn)(a[0].p, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "I:PIP") == 0) { r->i = ((int (*)(void *, int, void *))fn)(a[0].p, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "P:PIP") == 0) { r->p = ((void * (*)(void *, int, void *))fn)(a[0].p, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "T:PIP") == 0) { r->t = ((time_t (*)(void *, int, void *))fn)(a[0].p, (int)a[1].i, a[2].p); return 0; }
  if (strcmp(shape, "v:PPI") == 0) { ((void (*)(void *, void *, int))fn)(a[0].p, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "I:PPI") == 0) { r->i = ((int (*)(void *, void *, int))fn)(a[0].p, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "P:PPI") == 0) { r->p = ((void * (*)(void *, void *, int))fn)(a[0].p, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "T:PPI") == 0) { r->t = ((time_t (*)(void *, void *, int))fn)(a[0].p, a[1].p, (int)a[2].i); return 0; }
  if (strcmp(shape, "v:PPP") == 0) { ((void (*)(void *, void *, void *))fn)(a[0].p, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "I:PPP") == 0) { r->i = ((int (*)(void *, void *, void *))fn)(a[0].p, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "P:PPP") == 0) { r->p = ((void * (*)(void *, void *, void *))fn)(a[0].p, a[1].p, a[2].p); return 0; }
  if (strcmp(shape, "T:PPP") == 0) { r->t = ((time_t (*)(void *, void *, void *))fn)(a[0].p, a[1].p, a[2].p); return 0; }
  return -1;
}

/* Signature "R:ARGS".
   ARGS: i int, k key (int or quoted key), s string (NULL allowed),
         r string the library keeps (not freed),
         f bindable function name, m keymap name, d histdata (integer).
   R:    v void, i int, t time_t, s borrowed string, S owned string,
         L owned NULL-terminated string vector, N string vector whose array
         (not elements) is owned, H borrowed HIST_ENTRY*, F owned HIST_ENTRY*,
         R owned HIST_ENTRY** vector, f function, m keymap. */
typedef struct {
  const char *name;
  const char *sig;
  void (*fn)(void);
  int capture; /* capture rl_outstream output into "out" */
} api_t;

#define API(n, s) {#n, s, (void (*)(void))n, 0}
#define APIC(n, s) {#n, s, (void (*)(void))n, 1}

static int drv_rl_getc(void) {
  if (!wait_for_input()) return EOF;
  int c = rl_getc(rl_instream ? rl_instream : stdin);
  if (c >= 0) consumed++;
  return c;
}

static const api_t api_table[] = {
  /* history */
  API(using_history, "v:"),
  API(add_history, "v:s"),
  API(add_history_time, "v:s"),
  API(remove_history, "F:i"),
  API(remove_history_range, "R:ii"),
  API(alloc_history_entry, "F:sr"),
  API(replace_history_entry, "F:isd"),
  API(clear_history, "v:"),
  API(stifle_history, "v:i"),
  API(unstifle_history, "i:"),
  API(history_is_stifled, "i:"),
  API(where_history, "i:"),
  API(current_history, "H:"),
  API(history_get, "H:i"),
  API(history_total_bytes, "i:"),
  API(history_set_pos, "i:i"),
  API(previous_history, "H:"),
  API(next_history, "H:"),
  API(history_search, "i:si"),
  API(history_search_prefix, "i:si"),
  API(history_search_pos, "i:sii"),
  API(read_history, "i:s"),
  API(read_history_range, "i:sii"),
  API(write_history, "i:s"),
  API(append_history, "i:is"),
  API(history_truncate_file, "i:si"),
  API(history_arg_extract, "S:iis"),
  API(history_tokenize, "L:s"),
  /* tilde */
  API(tilde_expand, "S:s"),
  API(tilde_expand_word, "S:s"),
  /* binding */
  API(rl_bind_key, "i:kf"),
  API(rl_bind_key_in_map, "i:kfm"),
  API(rl_unbind_key, "i:k"),
  API(rl_unbind_key_in_map, "i:km"),
  API(rl_bind_key_if_unbound, "i:kf"),
  API(rl_bind_key_if_unbound_in_map, "i:kfm"),
  API(rl_unbind_function_in_map, "i:fm"),
  API(rl_unbind_command_in_map, "i:sm"),
  API(rl_bind_keyseq, "i:sf"),
  API(rl_bind_keyseq_in_map, "i:sfm"),
  API(rl_bind_keyseq_if_unbound, "i:sf"),
  API(rl_bind_keyseq_if_unbound_in_map, "i:sfm"),
  API(rl_variable_value, "s:s"),
  API(rl_variable_bind, "i:ss"),
  API(rl_set_key, "i:sfm"),
  API(rl_macro_bind, "i:ssm"),
  API(rl_untranslate_keyseq, "s:k"),
  API(rl_named_function, "f:s"),
  APIC(rl_list_funmap_names, "v:"),
  API(rl_invoking_keyseqs_in_map, "L:fm"),
  API(rl_invoking_keyseqs, "L:f"),
  APIC(rl_print_keybinding, "v:smi"),
  APIC(rl_function_dumper, "v:i"),
  APIC(rl_macro_dumper, "v:i"),
  APIC(rl_variable_dumper, "v:i"),
  API(rl_read_init_file, "i:s"),
  API(rl_parse_and_bind, "i:s"),
  /* keymaps */
  API(rl_empty_keymap, "i:m"),
  API(rl_get_keymap_by_name, "m:s"),
  API(rl_get_keymap_name, "s:m"),
  API(rl_set_keymap, "v:m"),
  API(rl_get_keymap, "m:"),
  API(rl_set_keymap_name, "i:sm"),
  API(rl_set_keymap_from_edit_mode, "v:"),
  API(rl_get_keymap_name_from_edit_mode, "s:"),
  API(rl_add_funmap_entry, "i:rf"),
  API(rl_funmap_names, "N:"),
  API(rl_initialize_funmap, "v:"),
  /* undo */
  API(rl_free_undo_list, "v:"),
  API(rl_do_undo, "i:"),
  API(rl_begin_undo_group, "i:"),
  API(rl_end_undo_group, "i:"),
  API(rl_modifying, "i:ii"),
  /* redisplay */
  API(rl_redisplay, "v:"),
  API(rl_on_new_line, "i:"),
  API(rl_on_new_line_with_prompt, "i:"),
  API(rl_forced_update_display, "i:"),
  API(rl_clear_visible_line, "i:"),
  API(rl_clear_message, "i:"),
  API(rl_reset_line_state, "i:"),
  API(rl_crlf, "i:"),
  API(rl_keep_mark_active, "v:"),
  API(rl_activate_mark, "v:"),
  API(rl_deactivate_mark, "v:"),
  API(rl_mark_active_p, "i:"),
  API(rl_show_char, "i:k"),
  API(rl_character_len, "i:ki"),
  API(rl_redraw_prompt_last_line, "v:"),
  API(rl_save_prompt, "v:"),
  API(rl_restore_prompt, "v:"),
  API(rl_set_prompt, "i:s"),
  API(rl_expand_prompt, "i:s"),
  /* line buffer */
  API(rl_replace_line, "v:si"),
  API(rl_insert_text, "i:s"),
  API(rl_delete_text, "i:ii"),
  API(rl_kill_text, "i:ii"),
  API(rl_copy_text, "S:ii"),
  /* terminal */
  API(rl_prep_terminal, "v:i"),
  API(rl_deprep_terminal, "v:"),
  API(rl_tty_set_default_bindings, "v:m"),
  API(rl_tty_unset_default_bindings, "v:m"),
  API(rl_tty_set_echoing, "i:i"),
  API(rl_reset_terminal, "i:s"),
  API(rl_resize_terminal, "v:"),
  API(rl_set_screen_size, "v:ii"),
  API(rl_reset_screen_size, "v:"),
  API(rl_get_termcap, "s:s"),
  API(rl_reparse_colors, "v:"),
  /* input */
  API(rl_stuff_char, "i:k"),
  API(rl_execute_next, "i:k"),
  API(rl_clear_pending_input, "i:"),
  API(rl_read_key, "i:"),
  {"rl_getc", "i:", (void (*)(void))drv_rl_getc, 0},
  API(rl_set_keyboard_input_timeout, "i:i"),
  API(rl_set_timeout, "i:ii"),
  /* utilities */
  API(rl_extend_line_buffer, "v:i"),
  API(rl_ding, "i:"),
  API(rl_alphabetic, "i:k"),
  /* signals */
  API(rl_set_signals, "i:"),
  API(rl_clear_signals, "i:"),
  API(rl_cleanup_after_signal, "v:"),
  API(rl_reset_after_signal, "v:"),
  API(rl_free_line_state, "v:"),
  API(rl_pending_signal, "i:"),
  API(rl_check_signals, "v:"),
  API(rl_echo_signal_char, "v:i"),
  API(rl_set_paren_blink_timeout, "i:i"),
  /* history helpers in libreadline */
  API(rl_clear_history, "v:"),
  API(rl_maybe_save_line, "i:"),
  API(rl_maybe_unsave_line, "i:"),
  API(rl_maybe_replace_line, "i:"),
  /* completion */
  API(rl_complete_internal, "i:k"),
  API(rl_completion_mode, "i:f"),
  /* init and misc */
  API(rl_initialize, "i:"),
  API(rl_discard_argument, "i:"),
  API(rl_callback_sigcleanup, "v:"),
  API(rl_callback_handler_remove, "v:"),
  API(rl_vi_check, "i:"),
  API(rl_vi_bracktype, "i:k"),
  API(rl_vi_start_inserting, "v:iii"),
  /* every bindable command, as rl_xxx COUNT KEY (generated) */
#include "bindable.inc"
};

static const api_t *api_find(const char *name) {
  for (size_t i = 0; i < sizeof api_table / sizeof api_table[0]; i++)
    if (strcmp(api_table[i].name, name) == 0) return &api_table[i];
  return NULL;
}

/* ------------------------------------------------------------------ */
/* variables                                                           */

typedef enum { V_INT, V_ULONG, V_CHAR, V_STR, V_CSTR, V_STRV, V_KEYMAP, V_FUNC, V_FILE, V_UNDO, V_FUNMAP, V_KSEQ } vtype_t;

typedef struct {
  const char *name;
  vtype_t t;
  void *addr;
  int ro;
} var_t;

#define VAR(n, t) {#n, t, (void *)&n, 0}
#define VRO(n, t) {#n, t, (void *)&n, 1}

static const var_t var_table[] = {
  VAR(history_base, V_INT), VAR(history_length, V_INT), VAR(history_max_entries, V_INT),
  VAR(history_offset, V_INT), VAR(history_lines_read_from_file, V_INT),
  VAR(history_lines_written_to_file, V_INT), VAR(history_expansion_char, V_CHAR),
  VAR(history_subst_char, V_CHAR), VAR(history_word_delimiters, V_STR),
  VAR(history_comment_char, V_CHAR), VAR(history_no_expand_chars, V_STR),
  VAR(history_search_delimiter_chars, V_STR), VAR(history_quotes_inhibit_expansion, V_INT),
  VAR(history_quoting_state, V_INT), VAR(history_write_timestamps, V_INT),
  VAR(history_multiline_entries, V_INT), VAR(history_file_version, V_INT),
  VAR(max_input_history, V_INT),
  VAR(tilde_additional_prefixes, V_STRV), VAR(tilde_additional_suffixes, V_STRV),
  VRO(rl_library_version, V_CSTR), VAR(rl_readline_version, V_INT), VAR(rl_gnu_readline_p, V_INT),
  VAR(rl_readline_state, V_ULONG), VAR(rl_editing_mode, V_INT), VAR(rl_insert_mode, V_INT),
  VAR(rl_readline_name, V_CSTR), VRO(rl_prompt, V_STR), VRO(rl_display_prompt, V_STR),
  VRO(rl_line_buffer, V_STR), VAR(rl_point, V_INT), VAR(rl_end, V_INT), VAR(rl_mark, V_INT),
  VAR(rl_done, V_INT), VAR(rl_eof_found, V_INT), VAR(rl_pending_input, V_INT),
  VAR(rl_dispatching, V_INT), VAR(rl_explicit_arg, V_INT), VAR(rl_numeric_arg, V_INT),
  VAR(rl_last_func, V_FUNC), VAR(rl_terminal_name, V_CSTR), VAR(rl_instream, V_FILE),
  VAR(rl_outstream, V_FILE), VAR(rl_prefer_env_winsize, V_INT),
  VRO(rl_executing_keymap, V_KEYMAP), VRO(rl_binding_keymap, V_KEYMAP),
  VAR(rl_executing_key, V_INT), VRO(rl_executing_keyseq, V_KSEQ),
  VAR(rl_key_sequence_length, V_INT), VAR(rl_erase_empty_line, V_INT),
  VAR(rl_already_prompted, V_INT), VAR(rl_num_chars_to_read, V_INT),
  VRO(rl_executing_macro, V_STR), VAR(rl_catch_signals, V_INT), VAR(rl_catch_sigwinch, V_INT),
  VAR(rl_change_environment, V_INT),
  VAR(rl_basic_word_break_characters, V_CSTR), VAR(rl_completer_word_break_characters, V_CSTR),
  VAR(rl_completer_quote_characters, V_CSTR), VAR(rl_basic_quote_characters, V_CSTR),
  VAR(rl_filename_quote_characters, V_CSTR), VAR(rl_special_prefixes, V_CSTR),
  VAR(rl_filename_completion_desired, V_INT), VAR(rl_filename_quoting_desired, V_INT),
  VAR(rl_full_quoting_desired, V_INT), VAR(rl_attempted_completion_over, V_INT),
  VAR(rl_completion_type, V_INT), VAR(rl_completion_invoking_key, V_INT),
  VAR(rl_completion_query_items, V_INT), VAR(rl_completion_append_character, V_INT),
  VAR(rl_completion_suppress_append, V_INT), VAR(rl_completion_quote_character, V_INT),
  VAR(rl_completion_found_quote, V_INT), VAR(rl_completion_suppress_quote, V_INT),
  VAR(rl_sort_completion_matches, V_INT), VAR(rl_completion_mark_symlink_dirs, V_INT),
  VAR(rl_ignore_completion_duplicates, V_INT), VAR(rl_inhibit_completion, V_INT),
  VAR(rl_persistent_signal_handlers, V_INT),
  VRO(rl_undo_list, V_UNDO), VRO(funmap, V_FUNMAP),
};

static const var_t *var_find(const char *name) {
  for (size_t i = 0; i < sizeof var_table / sizeof var_table[0]; i++)
    if (strcmp(var_table[i].name, name) == 0) return &var_table[i];
  return NULL;
}

static void rec_var_value(rec_t *r, const char *k, const var_t *v) {
  switch (v->t) {
    case V_INT: rec_int(r, k, *(int *)v->addr); break;
    case V_ULONG:
      if (v->addr == (void *)&rl_readline_state) rec_state(r, k, rl_readline_state);
      else rec_int(r, k, (long)*(unsigned long *)v->addr);
      break;
    case V_CHAR: {
      char ch[2] = {*(char *)v->addr, 0};
      rec_strn(r, k, ch, ch[0] ? 1 : 0);
      break;
    }
    case V_STR: rec_str(r, k, *(char **)v->addr); break;
    case V_CSTR: rec_str(r, k, *(const char **)v->addr); break;
    case V_STRV: rec_strv(r, k, *(char ***)v->addr); break;
    case V_KEYMAP: rec_str(r, k, keymap_name(*(Keymap *)v->addr)); break;
    case V_FUNC: rec_str(r, k, func_name(*(rl_command_func_t **)v->addr)); break;
    case V_FILE: {
      FILE *f = *(FILE **)v->addr;
      rec_str(r, k, !f ? NULL : f == stdin ? "stdin" : f == stdout ? "stdout" : f == stderr ? "stderr" : "other");
      break;
    }
    case V_UNDO: rec_undo(r, k); break;
    case V_FUNMAP: {
      rec_key(r, k);
      buf_putc(&r->b, '[');
      for (int i = 0; funmap && funmap[i]; i++) {
        if (i) buf_puts(&r->b, ", ");
        json_str(&r->b, funmap[i]->name);
      }
      buf_putc(&r->b, ']');
      break;
    }
    case V_KSEQ:
      if (rl_executing_keyseq && rl_key_sequence_length >= 0) rec_strn(r, k, rl_executing_keyseq, (size_t)rl_key_sequence_length);
      else rec_null(r, k);
      break;
  }
}

/* ------------------------------------------------------------------ */
/* command execution                                                   */

static void rec_error(cmd_t *c, const char *msg) {
  rec_t r;
  rec_begin(&r, c->text);
  rec_str(&r, "error", msg);
  rec_end(&r);
}

static char *arg_dup(cmd_t *c, int i) { return c->v[i] ? xstrdup(c->v[i]) : NULL; }

static void run_cmd(cmd_t *c);

static void run_block(block_t *b) {
  for (int i = 0; i < b->n; i++) run_cmd(&b->cmds[i]);
}

static void run_block_named(const char *name) {
  block_t *b = block_find(name);
  if (b) run_block(b);
}

/* capture rl_outstream output (nestable) */
typedef struct {
  FILE *saved, *tmp;
} cap_t;

static void capture_begin(cap_t *cp) {
  fflush(rl_outstream ? rl_outstream : stdout);
  cp->saved = rl_outstream;
  cp->tmp = tmpfile();
  if (!cp->tmp) die("tmpfile: %s", strerror(errno));
  rl_outstream = cp->tmp;
}

static char *capture_end(cap_t *cp, size_t *len) {
  fflush(cp->tmp);
  long n = ftell(cp->tmp);
  if (n < 0) n = 0;
  char *s = xmalloc((size_t)n + 1);
  rewind(cp->tmp);
  size_t got = fread(s, 1, (size_t)n, cp->tmp);
  s[got] = 0;
  if (rl_outstream == cp->tmp) rl_outstream = cp->saved;
  fclose(cp->tmp);
  cp->tmp = NULL;
  *len = got;
  return s;
}

static int convert_args(cmd_t *c, const char *argsig, val_t *a, char **owned, char **err) {
  int na = (int)strlen(argsig);
  if (c->n - 1 != na) { *err = "wrong number of arguments"; return -1; }
  for (int i = 0; i < na; i++) {
    int ti = i + 1;
    owned[i] = NULL;
    switch (argsig[i]) {
      case 'i': case 'k':
        if (parse_int_arg(c, ti, &a[i].i)) { *err = "bad integer argument"; return -1; }
        break;
      case 'd':
        if (parse_int_arg(c, ti, &a[i].i)) { *err = "bad data argument"; return -1; }
        a[i].p = (void *)(intptr_t)a[i].i;
        break;
      case 's':
        owned[i] = arg_dup(c, ti);
        a[i].p = owned[i];
        break;
      case 'r': /* string retained by the library: never freed */
        a[i].p = arg_dup(c, ti);
        break;
      case 'f': {
        int ok;
        a[i].p = (void *)func_lookup(c->v[ti], &ok);
        if (!ok) { *err = "unknown function"; return -1; }
        break;
      }
      case 'm': {
        int ok;
        a[i].p = keymap_lookup(c->v[ti], &ok);
        if (!ok) { *err = "unknown keymap"; return -1; }
        break;
      }
      default: *err = "bad signature"; return -1;
    }
  }
  return 0;
}

static void run_api(cmd_t *c, const api_t *api) {
  char shape[16];
  const char *colon = strchr(api->sig, ':');
  const char *argsig = colon + 1;
  char rk = api->sig[0];
  val_t a[4], r;
  char *owned[4] = {0};
  char *err = NULL;
  memset(a, 0, sizeof a);
  memset(&r, 0, sizeof r);
  if (convert_args(c, argsig, a, owned, &err)) {
    rec_error(c, err);
    for (int i = 0; i < 4; i++) free(owned[i]);
    return;
  }
  int k = 0;
  shape[k++] = rk == 'v' ? 'v' : rk == 'i' ? 'I' : rk == 't' ? 'T' : 'P';
  shape[k++] = ':';
  for (const char *p = argsig; *p; p++) shape[k++] = (*p == 'i' || *p == 'k') ? 'I' : 'P';
  shape[k] = 0;
  cap_t cp;
  if (api->capture) capture_begin(&cp);
  if (call_shape(shape, api->fn, a, &r)) die("no call shape %s", shape);
  size_t outlen = 0;
  char *out = api->capture ? capture_end(&cp, &outlen) : NULL;
  if (rk == 'v' && !out) return; /* void calls produce no record */
  rec_t rec;
  rec_begin(&rec, c->text);
  switch (rk) {
    case 'v': break;
    case 'i': rec_int(&rec, "ret", r.i); break;
    case 't': rec_time(&rec, "ret", r.t); break;
    case 's': rec_str(&rec, "ret", (char *)r.p); break;
    case 'S': rec_str(&rec, "ret", (char *)r.p); free(r.p); break;
    case 'L': {
      char **v = r.p;
      rec_strv(&rec, "ret", v);
      if (v) { for (int i = 0; v[i]; i++) free(v[i]); free(v); }
      break;
    }
    case 'N': rec_strv(&rec, "ret", (char **)r.p); free(r.p); break;
    case 'H': rec_hist_entry(&rec, "ret", r.p); break;
    case 'F': {
      HIST_ENTRY *e = r.p;
      rec_hist_entry(&rec, "ret", e);
      if (e) free_history_entry(e);
      break;
    }
    case 'R': {
      HIST_ENTRY **v = r.p;
      rec_key(&rec, "ret");
      if (!v) buf_puts(&rec.b, "null");
      else {
        buf_putc(&rec.b, '[');
        for (int i = 0; v[i]; i++) {
          if (i) buf_puts(&rec.b, ", ");
          json_hist_entry(&rec.b, v[i]);
          free_history_entry(v[i]);
        }
        buf_putc(&rec.b, ']');
        free(v);
      }
      break;
    }
    case 'f': rec_str(&rec, "ret", func_name((rl_command_func_t *)r.p)); break;
    case 'm': rec_str(&rec, "ret", keymap_name((Keymap)r.p)); break;
  }
  if (out) { rec_strn(&rec, "out", out, outlen); free(out); }
  rec_end(&rec);
  /* keep string arguments that the library may retain (rl_set_prompt copies,
     but e.g. rl_parse_and_bind may keep pointers into variable values) */
  for (int i = 0; i < 4; i++) free(owned[i]);
}

/* ------------------------------------------------------------------ */
/* custom commands                                                     */

#define NEED(nmin, nmax) do { if (c->n - 1 < (nmin) || c->n - 1 > (nmax)) { rec_error(c, "wrong number of arguments"); return; } } while (0)
#define INTARG(idx, var) long var; do { if (parse_int_arg(c, idx, &var)) { rec_error(c, "bad integer argument"); return; } } while (0)
#define KEYMAPARG(idx, var) Keymap var; do { int ok_; var = keymap_lookup(c->v[idx], &ok_); if (!ok_) { rec_error(c, "unknown keymap"); return; } } while (0)

static void free_strv(char **v) {
  if (!v) return;
  for (int i = 0; v[i]; i++) free(v[i]);
  free(v);
}

static void set_strlist(char ***dst, int *n, cmd_t *c, int from) {
  for (int i = 0; i < *n; i++) free((*dst)[i]);
  free(*dst);
  *n = c->n - from;
  *dst = xmalloc(sizeof(char *) * (size_t)(*n + 1));
  for (int i = 0; i < *n; i++) (*dst)[i] = xstrdup(c->v[from + i] ? c->v[from + i] : "");
  (*dst)[*n] = NULL;
}

static void c_echo(cmd_t *c) {
  rec_t r;
  rec_begin(&r, "echo");
  rec_key(&r, "args");
  buf_putc(&r.b, '[');
  for (int i = 1; i < c->n; i++) {
    if (i > 1) buf_puts(&r.b, ", ");
    if (c->v[i]) json_strn(&r.b, c->v[i], c->lens[i]);
    else buf_puts(&r.b, "null");
  }
  buf_putc(&r.b, ']');
  rec_end(&r);
}

static void c_dump(cmd_t *c) {
  NEED(0, 1);
  do_dump(c->n > 1 ? c->v[1] : NULL, strcmp(c->v[0], "dumpfull") == 0);
}

static void c_args(cmd_t *c) {
  NEED(0, 0);
  if (!hctx) { rec_error(c, "not inside a hook"); return; }
  emit_hook_args(hctx);
}

static void c_return(cmd_t *c) {
  NEED(1, 1);
  if (!hctx) { rec_error(c, "not inside a hook"); return; }
  hctx->has_ret = 1;
  free(hctx->ret_s);
  hctx->ret_s = NULL;
  hctx->ret_null = c->v[1] == NULL;
  if (c->v[1]) {
    hctx->ret_s = xstrdup(c->v[1]);
    long v;
    hctx->ret_i = parse_int_arg(c, 1, &v) == 0 ? v : 0;
  }
}

static void c_replace_arg(cmd_t *c) {
  NEED(1, 1);
  if (!hctx || !hctx->strarg) { rec_error(c, "not inside a directory hook"); return; }
  free(*hctx->strarg);
  *hctx->strarg = arg_dup(c, 1);
}

static void c_call(cmd_t *c) {
  NEED(1, 1);
  block_t *b = c->v[1] ? block_find(c->v[1]) : NULL;
  if (!b) { rec_error(c, "no such block"); return; }
  run_block(b);
}

/* repeat N BLOCK: run BLOCK N times (e.g. to build long histories) */
static void c_repeat(cmd_t *c) {
  NEED(2, 2);
  INTARG(1, count);
  block_t *b = c->v[2] ? block_find(c->v[2]) : NULL;
  if (!b) { rec_error(c, "no such block"); return; }
  for (long i = 0; i < count; i++) run_block(b);
}

static void c_set(cmd_t *c) {
  if (c->n < 2 || !c->v[1]) { rec_error(c, "usage: set VAR VALUE"); return; }
  const char *name = c->v[1];
  hook_t *h = hook_find(name);
  if (h) {
    NEED(2, 2);
    const char *e = hook_set(h, c->v[2]);
    if (e) rec_error(c, e);
    return;
  }
  const var_t *v = var_find(name);
  if (!v) { rec_error(c, "unknown variable"); return; }
  if (v->ro) { rec_error(c, "variable is read-only in the driver"); return; }
  if (v->t == V_STRV) {
    char ***dst = (char ***)v->addr;
    /* keep previous arrays alive: the library may hold pointers into them */
    char **nv = xmalloc(sizeof(char *) * (size_t)c->n);
    for (int i = 2; i < c->n; i++) nv[i - 2] = arg_dup(c, i);
    nv[c->n - 2] = NULL;
    *dst = (c->n == 3 && c->v[2] == NULL) ? NULL : nv;
    return;
  }
  NEED(2, 2);
  switch (v->t) {
    case V_INT: {
      INTARG(2, x);
      *(int *)v->addr = (int)x;
      break;
    }
    case V_ULONG: {
      INTARG(2, x);
      *(unsigned long *)v->addr = (unsigned long)x;
      break;
    }
    case V_CHAR: {
      INTARG(2, x);
      *(char *)v->addr = (char)x;
      break;
    }
    case V_STR: *(char **)v->addr = arg_dup(c, 2); break;
    case V_CSTR: *(const char **)v->addr = arg_dup(c, 2); break;
    case V_FUNC: {
      int ok;
      rl_command_func_t *f = func_lookup(c->v[2], &ok);
      if (!ok) { rec_error(c, "unknown function"); return; }
      *(rl_command_func_t **)v->addr = f;
      break;
    }
    case V_FILE: {
      const char *s = c->v[2];
      FILE *f;
      if (!s) f = NULL;
      else if (strcmp(s, "stdin") == 0) f = stdin;
      else if (strcmp(s, "stdout") == 0) f = stdout;
      else if (strcmp(s, "stderr") == 0) f = stderr;
      else if (strncmp(s, "file:", 5) == 0) {
        f = fopen(s + 5, v->addr == (void *)&rl_instream ? "r" : "w");
        if (!f) { rec_error(c, strerror(errno)); return; }
      } else { rec_error(c, "bad stream"); return; }
      *(FILE **)v->addr = f;
      break;
    }
    default: rec_error(c, "cannot set this variable"); return;
  }
}

static void c_get(cmd_t *c) {
  NEED(1, 1);
  const char *name = c->v[1] ? c->v[1] : "";
  rec_t r;
  hook_t *h = hook_find(name);
  const var_t *v = var_find(name);
  if (!h && !v) { rec_error(c, "unknown variable"); return; }
  rec_begin(&r, c->text);
  if (h) rec_str(&r, "ret", hook_get(h));
  else rec_var_value(&r, "ret", v);
  rec_end(&r);
}

static void c_defun(cmd_t *c) {
  NEED(2, 3);
  if (!c->v[1] || !c->v[2] || !block_find(c->v[2])) { rec_error(c, "usage: defun NAME BLOCK [KEY]"); return; }
  if (ndefuns == MAX_DEFUNS) { rec_error(c, "too many defuns"); return; }
  long key = -1;
  if (c->n > 3 && parse_int_arg(c, 3, &key)) { rec_error(c, "bad key"); return; }
  int i = ndefuns++;
  defuns[i].name = xstrdup(c->v[1]);
  defuns[i].block = xstrdup(c->v[2]);
  int ret = rl_add_defun(defuns[i].name, defun_tramps[i], (int)key);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static int sig_by_name(const char *s) {
  if (!s) return -1;
  if (strncmp(s, "SIG", 3) == 0) s += 3;
  static const int sigs[] = {SIGINT, SIGQUIT, SIGWINCH, SIGTSTP, SIGTERM, SIGHUP, SIGALRM, SIGCONT, SIGTTIN, SIGTTOU, SIGUSR1, SIGUSR2};
  for (size_t i = 0; i < sizeof sigs / sizeof sigs[0]; i++)
    if (strcmp(sig_name(sigs[i]), s) == 0) return sigs[i];
  return -1;
}

static void c_signal(cmd_t *c) {
  NEED(2, 2);
  int s = sig_by_name(c->v[1]);
  if (s < 0) { rec_error(c, "unknown signal"); return; }
  const char *m = c->v[2] ? c->v[2] : "";
  int mode = strcmp(m, "record") == 0 ? 0 : strcmp(m, "default") == 0 ? 1 : strcmp(m, "ignore") == 0 ? 2 : -1;
  if (mode < 0) { rec_error(c, "mode must be record|default|ignore"); return; }
  install_sig(s, mode);
}

static void c_raise(cmd_t *c) {
  NEED(1, 1);
  int s = sig_by_name(c->v[1]);
  if (s < 0) { rec_error(c, "unknown signal"); return; }
  kill(getpid(), s);
}

static void c_onsig(cmd_t *c) {
  NEED(1, 1);
  free(onsig_block);
  onsig_block = arg_dup(c, 1);
}

static void c_complete_words(cmd_t *c) {
  int from = 1;
  words_icase = 0;
  if (c->n > 1 && c->v[1] && !c->quoted[1] && strcmp(c->v[1], "-i") == 0) { words_icase = 1; from = 2; }
  set_strlist(&words, &nwords, c, from);
}

static void c_ignore_words(cmd_t *c) { set_strlist(&ign_words, &nign_words, c, 1); }

static void c_tilde_map(cmd_t *c) {
  NEED(2, 2);
  tmap_keys = realloc(tmap_keys, sizeof(char *) * (size_t)(ntmap + 1));
  tmap_vals = realloc(tmap_vals, sizeof(char *) * (size_t)(ntmap + 1));
  tmap_keys[ntmap] = xstrdup(c->v[1] ? c->v[1] : "");
  tmap_vals[ntmap] = xstrdup(c->v[2] ? c->v[2] : "");
  ntmap++;
}

static void c_setenv(cmd_t *c) {
  NEED(2, 2);
  if (!c->v[1] || !c->v[2]) { rec_error(c, "NULL not allowed"); return; }
  setenv(c->v[1], c->v[2], 1);
}
static void c_unsetenv(cmd_t *c) {
  NEED(1, 1);
  if (c->v[1]) unsetenv(c->v[1]);
}
static void c_getenv(cmd_t *c) {
  NEED(1, 1);
  rec_t r;
  rec_begin(&r, c->text);
  rec_str(&r, "ret", c->v[1] ? getenv(c->v[1]) : NULL);
  rec_end(&r);
}
static void c_chdir(cmd_t *c) {
  NEED(1, 1);
  if (!c->v[1] || chdir(c->v[1]) != 0) rec_error(c, strerror(errno));
}

static void c_file_write(cmd_t *c) {
  NEED(2, 2);
  if (!c->v[1] || !c->v[2]) { rec_error(c, "NULL not allowed"); return; }
  FILE *f = fopen(c->v[1], strcmp(c->v[0], "file_append") == 0 ? "ab" : "wb");
  if (!f) { rec_error(c, strerror(errno)); return; }
  fwrite(c->v[2], 1, c->lens[2], f);
  fclose(f);
}

static void c_file_read(cmd_t *c) {
  NEED(1, 1);
  rec_t r;
  rec_begin(&r, c->text);
  FILE *f = c->v[1] ? fopen(c->v[1], "rb") : NULL;
  if (!f) {
    rec_null(&r, "content");
    rec_str(&r, "errno", errno == ENOENT ? "ENOENT" : errno == EACCES ? "EACCES" : "other");
  } else {
    buf_t b = {0};
    char tmp[4096];
    size_t n;
    buf_puts(&b, "");
    while ((n = fread(tmp, 1, sizeof tmp, f)) > 0) buf_putn(&b, tmp, n);
    fclose(f);
    size_t nl;
    char *norm = norm_times(b.p, b.len, &nl);
    rec_strn(&r, "content", norm, nl);
    free(norm);
    free(b.p);
  }
  rec_end(&r);
}

static void c_file_remove(cmd_t *c) {
  NEED(1, 1);
  if (!c->v[1] || unlink(c->v[1]) != 0) rec_error(c, strerror(errno));
}

static void c_file_stat(cmd_t *c) {
  NEED(1, 1);
  struct stat st;
  rec_t r;
  rec_begin(&r, c->text);
  if (!c->v[1] || stat(c->v[1], &st) != 0) rec_bool(&r, "exists", 0);
  else {
    char mode[16];
    snprintf(mode, sizeof mode, "%04o", (unsigned)(st.st_mode & 07777));
    rec_bool(&r, "exists", 1);
    rec_str(&r, "mode", mode);
    if (S_ISREG(st.st_mode)) rec_int(&r, "size", (long)st.st_size);
    rec_str(&r, "type", S_ISDIR(st.st_mode) ? "dir" : S_ISREG(st.st_mode) ? "file" : "other");
  }
  rec_end(&r);
}

static void c_mkdir(cmd_t *c) {
  NEED(1, 1);
  if (!c->v[1] || mkdir(c->v[1], 0755) != 0) rec_error(c, strerror(errno));
}

static void c_chmod(cmd_t *c) {
  NEED(2, 2);
  if (!c->v[1] || !c->v[2]) { rec_error(c, "NULL not allowed"); return; }
  if (chmod(c->v[1], (mode_t)strtol(c->v[2], NULL, 8)) != 0) rec_error(c, strerror(errno));
}

static void c_symlink(cmd_t *c) {
  NEED(2, 2);
  if (!c->v[1] || !c->v[2] || symlink(c->v[1], c->v[2]) != 0) rec_error(c, strerror(errno));
}

/* hardlink TARGET PATH, mkfifo PATH: more file types for completion tests */
static void c_hardlink(cmd_t *c) {
  NEED(2, 2);
  if (!c->v[1] || !c->v[2] || link(c->v[1], c->v[2]) != 0) rec_error(c, strerror(errno));
}

static void c_mkfifo(cmd_t *c) {
  NEED(1, 1);
  if (!c->v[1] || mkfifo(c->v[1], 0644) != 0) rec_error(c, strerror(errno));
}

/* mksocket PATH: a bound (and closed) unix domain socket file */
static void c_mksocket(cmd_t *c) {
  NEED(1, 1);
  struct sockaddr_un sa;
  memset(&sa, 0, sizeof sa);
  sa.sun_family = AF_UNIX;
  if (!c->v[1] || strlen(c->v[1]) >= sizeof sa.sun_path) { rec_error(c, "bad path"); return; }
  strcpy(sa.sun_path, c->v[1]);
  int fd = socket(AF_UNIX, SOCK_STREAM, 0);
  if (fd < 0) { rec_error(c, strerror(errno)); return; }
  if (bind(fd, (struct sockaddr *)&sa, sizeof sa) != 0) rec_error(c, strerror(errno));
  close(fd);
}

/* readline PROMPT [COUNT|all] [addhist] */
static void c_readline(cmd_t *c) {
  NEED(1, 3);
  long count = 1;
  int addhist = 0;
  for (int i = 2; i < c->n; i++) {
    if (c->v[i] && strcmp(c->v[i], "all") == 0) count = -1;
    else if (c->v[i] && strcmp(c->v[i], "addhist") == 0) addhist = 1;
    else if (parse_int_arg(c, i, &count)) { rec_error(c, "bad count"); return; }
  }
  for (long n = 0; count < 0 || n < count; n++) {
    char *line = readline(c->v[1]);
    rec_t r;
    rec_begin(&r, "readline");
    rec_str(&r, "ret", line);
    rec_end(&r);
    if (!line) break;
    if (addhist && *line) add_history(line);
    free(line);
  }
}

/* callback interface */
static int cb_lines, cb_done, cb_addhist;
static long cb_count;

static void cb_handler(char *line) {
  rec_t r;
  rec_begin(&r, "callback");
  rec_str(&r, "ret", line);
  rec_end(&r);
  cb_lines++;
  if (!line) cb_done = 1;
  else {
    if (cb_addhist && *line) add_history(line);
    free(line);
  }
  if (cb_count >= 0 && cb_lines >= cb_count) cb_done = 1;
}

static void cb_read_one(void) {
  in_callback_wait = 1;
  int have = wait_for_input();
  in_callback_wait = 0;
  (void)have;
  rl_callback_read_char();
}

static void c_callback(cmd_t *c) {
  NEED(1, 3);
  cb_count = 1;
  cb_addhist = 0;
  for (int i = 2; i < c->n; i++) {
    if (c->v[i] && strcmp(c->v[i], "all") == 0) cb_count = -1;
    else if (c->v[i] && strcmp(c->v[i], "addhist") == 0) cb_addhist = 1;
    else if (parse_int_arg(c, i, &cb_count)) { rec_error(c, "bad count"); return; }
  }
  cb_lines = 0;
  cb_done = 0;
  rl_callback_handler_install(c->v[1], cb_handler);
  int eof_rounds = 0;
  while (!cb_done) {
    if (eof_flag && !fd_readable(0) && ++eof_rounds > 100) {
      rec_error(c, "callback loop did not finish after EOF");
      break;
    }
    cb_read_one();
  }
  rl_callback_handler_remove();
}

static void c_callback_install(cmd_t *c) {
  NEED(1, 2);
  cb_count = -1;
  cb_lines = 0;
  cb_done = 0;
  cb_addhist = c->n > 2 && c->v[2] && strcmp(c->v[2], "addhist") == 0;
  rl_callback_handler_install(c->v[1], cb_handler);
}

static void c_callback_read_char(cmd_t *c) {
  NEED(0, 1);
  long n = 1;
  if (c->n > 1 && parse_int_arg(c, 1, &n)) { rec_error(c, "bad count"); return; }
  for (long i = 0; i < n; i++) cb_read_one();
}

static void c_capture(cmd_t *c) {
  if (c->n < 2) { rec_error(c, "usage: capture COMMAND ..."); return; }
  cmd_t inner = *c;
  inner.v = c->v + 1;
  inner.lens = c->lens + 1;
  inner.quoted = c->quoted + 1;
  inner.n = c->n - 1;
  inner.text = c->text + strlen("capture");
  while (*inner.text == ' ' || *inner.text == '\t') inner.text++;
  cap_t cp;
  capture_begin(&cp);
  run_cmd(&inner);
  size_t n;
  char *out = capture_end(&cp, &n);
  rec_t r;
  rec_begin(&r, "capture");
  rec_strn(&r, "out", out, n);
  rec_end(&r);
  free(out);
}

static void c_invoke(cmd_t *c) {
  NEED(1, 3);
  int ok;
  rl_command_func_t *f = func_lookup(c->v[1], &ok);
  if (!ok || !f) { rec_error(c, "unknown function"); return; }
  long count = 1, key = 0;
  if (c->n > 2 && parse_int_arg(c, 2, &count)) { rec_error(c, "bad count"); return; }
  if (c->n > 3 && parse_int_arg(c, 3, &key)) { rec_error(c, "bad key"); return; }
  int ret = f((int)count, (int)key);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static void c_history_expand(cmd_t *c) {
  NEED(1, 1);
  char *out = NULL;
  int ret = history_expand(c->v[1], &out);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_str(&r, "out", out);
  rec_end(&r);
  free(out);
}

static void c_history_get_time(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, off);
  rec_t r;
  rec_begin(&r, c->text);
  rec_time(&r, "ret", history_get_time(history_get((int)off)));
  rec_end(&r);
}

static void c_history_dump(cmd_t *c) {
  NEED(0, 0);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "base", history_base);
  rec_int(&r, "length", history_length);
  rec_int(&r, "max", history_max_entries);
  rec_int(&r, "stifled", history_is_stifled());
  rec_int(&r, "pos", where_history());
  rec_key(&r, "entries");
  HIST_ENTRY **l = history_list();
  buf_putc(&r.b, '[');
  for (int i = 0; l && i < history_length && l[i]; i++) {
    if (i) buf_puts(&r.b, ", ");
    json_hist_entry(&r.b, l[i]);
  }
  buf_putc(&r.b, ']');
  rec_end(&r);
}

static void c_history_list(cmd_t *c) {
  NEED(0, 0);
  rec_t r;
  rec_begin(&r, c->text);
  HIST_ENTRY **l = history_list();
  rec_key(&r, "ret");
  if (!l) buf_puts(&r.b, "null");
  else {
    buf_putc(&r.b, '[');
    for (int i = 0; l[i]; i++) {
      if (i) buf_puts(&r.b, ", ");
      json_hist_entry(&r.b, l[i]);
    }
    buf_putc(&r.b, ']');
  }
  rec_end(&r);
}

static void rec_hstate(rec_t *r, HISTORY_STATE *s) {
  rec_int(r, "offset", s->offset);
  rec_int(r, "length", s->length);
  rec_int(r, "size", s->size);
  rec_int(r, "flags", s->flags);
}

static void c_history_get_history_state(cmd_t *c) {
  NEED(0, 0);
  HISTORY_STATE *s = history_get_history_state();
  rec_t r;
  rec_begin(&r, c->text);
  if (s) rec_hstate(&r, s);
  rec_end(&r);
  free(s);
}

static HISTORY_STATE *hslots[8];
static void c_history_state_save(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, slot);
  if (slot < 0 || slot >= 8) { rec_error(c, "slot 0..7"); return; }
  free(hslots[slot]);
  hslots[slot] = history_get_history_state();
}
static void c_history_state_restore(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, slot);
  if (slot < 0 || slot >= 8 || !hslots[slot]) { rec_error(c, "empty slot"); return; }
  history_set_history_state(hslots[slot]);
}

static void c_copy_history_entry(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, off);
  HIST_ENTRY *e = copy_history_entry(history_get((int)off));
  rec_t r;
  rec_begin(&r, c->text);
  rec_hist_entry(&r, "ret", e);
  rec_end(&r);
  if (e) free_history_entry(e);
}

static void c_get_history_event(cmd_t *c) {
  NEED(2, 3);
  INTARG(2, idx);
  long q = 0;
  if (c->n > 3 && parse_int_arg(c, 3, &q)) { rec_error(c, "bad quote char"); return; }
  int index = (int)idx;
  char *ret = get_history_event(c->v[1], &index, (int)q);
  rec_t r;
  rec_begin(&r, c->text);
  rec_str(&r, "ret", ret);
  rec_int(&r, "index", index);
  rec_end(&r);
}

/* tilde_find_word is declared by the public header but the reference
   library does not define it, so the driver does not call it. */

static void c_translate_keyseq(cmd_t *c) {
  NEED(1, 1);
  size_t n = c->v[1] ? strlen(c->v[1]) : 0;
  char *out = xmalloc(2 * n + 2);
  int len = 0;
  int ret = rl_translate_keyseq(c->v[1], out, &len);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  if (ret == 0) rec_strn(&r, "out", out, len > 0 ? (size_t)len : 0);
  rec_int(&r, "len", len);
  rec_end(&r);
  free(out);
}

static void rec_binding(rec_t *r, rl_command_func_t *f, int type) {
  if (!f) { rec_null(r, "type"); rec_null(r, "value"); return; }
  if (type == ISFUNC) { rec_str(r, "type", "func"); rec_str(r, "value", func_name(f)); }
  else if (type == ISKMAP) { rec_str(r, "type", "keymap"); rec_str(r, "value", keymap_name((Keymap)f)); }
  else if (type == ISMACR) { rec_str(r, "type", "macro"); rec_str(r, "value", (char *)f); }
  else rec_int(r, "type", type);
}

static void c_function_of_keyseq(cmd_t *c) {
  int haslen = strcmp(c->v[0], "rl_function_of_keyseq_len") == 0;
  NEED(haslen ? 3 : 2, haslen ? 3 : 2);
  int mi = haslen ? 3 : 2;
  KEYMAPARG(mi, map);
  int type = -1;
  rl_command_func_t *f;
  if (haslen) {
    INTARG(2, len);
    f = rl_function_of_keyseq_len(c->v[1], (size_t)len, map, &type);
  } else f = rl_function_of_keyseq(c->v[1], map, &type);
  rec_t r;
  rec_begin(&r, c->text);
  rec_binding(&r, f, type);
  rec_end(&r);
}

static void c_trim_arg_from_keyseq(cmd_t *c) {
  NEED(3, 3);
  INTARG(2, len);
  KEYMAPARG(3, map);
  int ret = rl_trim_arg_from_keyseq(c->v[1], (size_t)len, map);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static void c_generic_bind(cmd_t *c) {
  NEED(4, 4);
  const char *t = c->v[1] ? c->v[1] : "";
  int type = strcmp(t, "ISFUNC") == 0 ? ISFUNC : strcmp(t, "ISKMAP") == 0 ? ISKMAP : strcmp(t, "ISMACR") == 0 ? ISMACR : -1;
  if (type < 0) { long x; if (parse_int_arg(c, 1, &x)) { rec_error(c, "bad type"); return; } type = (int)x; }
  KEYMAPARG(4, map);
  char *data;
  if (type == ISFUNC) {
    int ok;
    data = (char *)func_lookup(c->v[3], &ok);
    if (!ok) { rec_error(c, "unknown function"); return; }
  } else if (type == ISKMAP) {
    int ok;
    data = (char *)keymap_lookup(c->v[3], &ok);
    if (!ok) { rec_error(c, "unknown keymap"); return; }
  } else data = arg_dup(c, 3); /* macro text: ownership passes to the library */
  int ret = rl_generic_bind(type, c->v[2], data, map);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static void c_keymap_new(cmd_t *c) {
  NEED(2, 3);
  if (!c->v[1]) { rec_error(c, "name required"); return; }
  const char *kind = c->v[2] ? c->v[2] : "";
  Keymap m;
  if (strcmp(kind, "bare") == 0) m = rl_make_bare_keymap();
  else if (strcmp(kind, "full") == 0) m = rl_make_keymap();
  else if (strcmp(kind, "copy") == 0 && c->n == 4) {
    KEYMAPARG(3, src);
    m = rl_copy_keymap(src);
  } else { rec_error(c, "usage: keymap_new NAME bare|full|copy SRC"); return; }
  keymap_register(c->v[1], m);
}

static void c_keymap_free(cmd_t *c) {
  NEED(1, 1);
  KEYMAPARG(1, map);
  if (strcmp(c->v[0], "rl_discard_keymap") == 0) rl_discard_keymap(map);
  else rl_free_keymap(map);
  keymap_forget(map);
}

static void keyname(buf_t *b, int k) {
  if (k == 256) { buf_puts(b, "<any>"); return; }
  if (k >= 128) { buf_puts(b, "\\M-"); k -= 128; }
  if (k == 27) buf_puts(b, "\\e");
  else if (k == 127) buf_puts(b, "\\C-?");
  else if (k < 32) { buf_puts(b, "\\C-"); buf_putc(b, (char)tolower(k + 64)); }
  else if (k == '\\' || k == '"') { buf_putc(b, '\\'); buf_putc(b, (char)k); }
  else buf_putc(b, (char)k);
}

static void keymap_dump_rec(rec_t *r, Keymap m, buf_t *prefix, int *first, int depth) {
  for (int k = 0; k < KEYMAP_SIZE; k++) {
    KEYMAP_ENTRY e = m[k];
    if (e.type == ISFUNC && e.function == NULL) continue;
    size_t plen = prefix->len;
    keyname(prefix, k);
    if (e.type == ISKMAP && depth < 8 && rl_get_keymap_name((Keymap)e.function) == NULL) {
      keymap_dump_rec(r, (Keymap)e.function, prefix, first, depth + 1);
    } else {
      buf_puts(&r->b, *first ? "" : ", ");
      *first = 0;
      buf_puts(&r->b, "[");
      json_str(&r->b, prefix->p);
      buf_puts(&r->b, ", ");
      json_str(&r->b, e.type == ISFUNC ? "func" : e.type == ISKMAP ? "keymap" : e.type == ISMACR ? "macro" : "?");
      buf_puts(&r->b, ", ");
      if (e.type == ISFUNC) json_str(&r->b, func_name(e.function));
      else if (e.type == ISKMAP) json_str(&r->b, keymap_name((Keymap)e.function));
      else if (e.type == ISMACR) json_str(&r->b, (char *)e.function);
      else buf_puts(&r->b, "null");
      buf_puts(&r->b, "]");
    }
    prefix->len = plen;
    prefix->p[plen] = 0;
  }
}

static void c_keymap_dump(cmd_t *c) {
  NEED(1, 1);
  KEYMAPARG(1, map);
  rec_t r;
  rec_begin(&r, c->text);
  rec_key(&r, "ret");
  if (!map) buf_puts(&r.b, "null");
  else {
    buf_t prefix = {0};
    buf_puts(&prefix, "");
    int first = 1;
    buf_putc(&r.b, '[');
    keymap_dump_rec(&r, map, &prefix, &first, 0);
    buf_putc(&r.b, ']');
    free(prefix.p);
  }
  rec_end(&r);
}

static void c_add_undo(cmd_t *c) {
  NEED(4, 4);
  const char *w = c->v[1] ? c->v[1] : "";
  enum undo_code what;
  if (strcmp(w, "delete") == 0) what = UNDO_DELETE;
  else if (strcmp(w, "insert") == 0) what = UNDO_INSERT;
  else if (strcmp(w, "begin") == 0) what = UNDO_BEGIN;
  else if (strcmp(w, "end") == 0) what = UNDO_END;
  else { rec_error(c, "what: delete|insert|begin|end"); return; }
  INTARG(2, start);
  INTARG(3, end);
  rl_add_undo(what, (int)start, (int)end, arg_dup(c, 4));
}

static void c_message(cmd_t *c) {
  NEED(1, 1);
  int ret = rl_message("%s", c->v[1] ? c->v[1] : "(null)");
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static void c_get_screen_size(cmd_t *c) {
  NEED(0, 0);
  int rows = -1, cols = -1;
  rl_get_screen_size(&rows, &cols);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "rows", rows);
  rec_int(&r, "cols", cols);
  rec_end(&r);
}

static void c_timeout_remaining(cmd_t *c) {
  NEED(0, 0);
  unsigned int s = 0, us = 0;
  int ret = rl_timeout_remaining(&s, &us);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static int cmp_strp(const void *a, const void *b) {
  return strcmp(*(char *const *)a, *(char *const *)b);
}

/* trailing "sort" argument: sort results (readdir order is not portable) */
static int want_sort(cmd_t *c, int idx) {
  if (c->n > idx && c->v[idx] && !c->quoted[idx] && strcmp(c->v[idx], "sort") == 0) {
    c->n--; /* hide it from NEED() */
    return 1;
  }
  return 0;
}

static void c_completion_matches(cmd_t *c) {
  int sorted = want_sort(c, 3);
  NEED(2, 2);
  int ok;
  rl_compentry_func_t *g = generator_by_name(c->v[2], &ok);
  if (!ok || !g) { rec_error(c, "unknown generator"); return; }
  char **m = rl_completion_matches(c->v[1], g);
  if (sorted) c->n++;
  if (m && sorted && m[0] && m[1]) {
    int n = 0;
    while (m[n + 1]) n++;
    qsort(m + 1, (size_t)n, sizeof(char *), cmp_strp);
  }
  rec_t r;
  rec_begin(&r, c->text);
  rec_strv(&r, "ret", m);
  rec_end(&r);
  free_strv(m);
}

static void c_gen(cmd_t *c) {
  int sorted = want_sort(c, 3);
  if (c->n - 1 != 2) { if (sorted) c->n++; rec_error(c, "wrong number of arguments"); return; }
  if (sorted) c->n++;
  int ok;
  rl_compentry_func_t *g = generator_by_name(c->v[1], &ok);
  if (!ok || !g) { rec_error(c, "unknown generator"); return; }
  char **v = NULL;
  int n = 0;
  for (int state = 0; state < 10000; state++) {
    char *s = g(c->v[2] ? c->v[2] : "", state);
    if (!s) break;
    v = realloc(v, sizeof(char *) * (size_t)(n + 2));
    v[n++] = s;
  }
  if (v) v[n] = NULL;
  if (sorted && n > 1) qsort(v, (size_t)n, sizeof(char *), cmp_strp);
  rec_t r;
  rec_begin(&r, c->text);
  rec_key(&r, "ret");
  buf_putc(&r.b, '[');
  for (int i = 0; i < n; i++) {
    if (i) buf_puts(&r.b, ", ");
    json_str(&r.b, v[i]);
    free(v[i]);
  }
  free(v);
  buf_putc(&r.b, ']');
  rec_end(&r);
}

static void c_display_match_list(cmd_t *c) {
  if (c->n == 1) {
    if (!compdisp_matches) { rec_error(c, "no matches outside the display hook"); return; }
    rl_display_match_list(compdisp_matches, compdisp_len, compdisp_max);
    return;
  }
  /* display_match_list LCD W1 W2 ... */
  int n = c->n - 1;
  char **m = xmalloc(sizeof(char *) * (size_t)(n + 1));
  int mx = 0;
  for (int i = 0; i < n; i++) {
    m[i] = c->v[i + 1] ? c->v[i + 1] : (char *)"";
    if (i > 0 && (int)strlen(m[i]) > mx) mx = (int)strlen(m[i]);
  }
  m[n] = NULL;
  rl_display_match_list(m, n - 1, mx);
  free(m);
}

static struct readline_state *rlslots[4];
static void c_save_state(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, slot);
  if (slot < 0 || slot >= 4) { rec_error(c, "slot 0..3"); return; }
  if (!rlslots[slot]) rlslots[slot] = xmalloc(sizeof(struct readline_state));
  memset(rlslots[slot], 0, sizeof(struct readline_state));
  int ret = rl_save_state(rlslots[slot]);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}
static void c_restore_state(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, slot);
  if (slot < 0 || slot >= 4 || !rlslots[slot]) { rec_error(c, "empty slot"); return; }
  int ret = rl_restore_state(rlslots[slot]);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_end(&r);
}

static void c_vi_domove(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, key);
  int next = -1;
  int ret = rl_vi_domove((int)key, &next);
  rec_t r;
  rec_begin(&r, c->text);
  rec_int(&r, "ret", ret);
  rec_int(&r, "nextkey", next);
  rec_end(&r);
}

static void c_push_macro_input(cmd_t *c) {
  NEED(1, 1);
  rl_push_macro_input(arg_dup(c, 1)); /* library frees it */
}

static void c_sleep_ms(cmd_t *c) {
  NEED(1, 1);
  INTARG(1, ms);
  struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
  while (nanosleep(&ts, &ts) != 0 && errno == EINTR) ;
}

static void c_exit(cmd_t *c) {
  NEED(0, 1);
  long code = 0;
  if (c->n > 1 && parse_int_arg(c, 1, &code)) { rec_error(c, "bad code"); return; }
  fflush(NULL);
  exit((int)code);
}

static void c_option(cmd_t *c) { (void)c; }

static void c_snap(cmd_t *c) {
  NEED(0, 0);
  request_snapshot();
}

typedef struct {
  const char *name;
  void (*fn)(cmd_t *);
} custom_t;

static const custom_t custom_table[] = {
  {"echo", c_echo}, {"dump", c_dump}, {"dumpfull", c_dump}, {"args", c_args},
  {"return", c_return}, {"replace_arg", c_replace_arg}, {"call", c_call},
  {"repeat", c_repeat},
  {"set", c_set}, {"get", c_get}, {"defun", c_defun}, {"signal", c_signal},
  {"raise", c_raise}, {"onsig", c_onsig}, {"complete_words", c_complete_words},
  {"ignore_words", c_ignore_words}, {"tilde_map", c_tilde_map}, {"setenv", c_setenv},
  {"unsetenv", c_unsetenv}, {"getenv", c_getenv}, {"chdir", c_chdir},
  {"file_write", c_file_write}, {"file_append", c_file_write}, {"file_read", c_file_read},
  {"file_remove", c_file_remove}, {"file_stat", c_file_stat}, {"mkdir", c_mkdir},
  {"chmod", c_chmod}, {"symlink", c_symlink}, {"hardlink", c_hardlink}, {"mkfifo", c_mkfifo},
  {"mksocket", c_mksocket},
  {"readline", c_readline}, {"callback", c_callback}, {"callback_install", c_callback_install},
  {"rl_callback_handler_install", c_callback_install},
  {"callback_read_char", c_callback_read_char}, {"rl_callback_read_char", c_callback_read_char},
  {"capture", c_capture}, {"invoke", c_invoke},
  {"history_expand", c_history_expand}, {"history_get_time", c_history_get_time},
  {"history_dump", c_history_dump}, {"history_list", c_history_list},
  {"history_get_history_state", c_history_get_history_state},
  {"history_state_save", c_history_state_save}, {"history_state_restore", c_history_state_restore},
  {"copy_history_entry", c_copy_history_entry}, {"get_history_event", c_get_history_event},
  {"rl_translate_keyseq", c_translate_keyseq},
  {"rl_function_of_keyseq", c_function_of_keyseq}, {"rl_function_of_keyseq_len", c_function_of_keyseq},
  {"rl_trim_arg_from_keyseq", c_trim_arg_from_keyseq}, {"rl_generic_bind", c_generic_bind},
  {"keymap_new", c_keymap_new}, {"rl_discard_keymap", c_keymap_free}, {"rl_free_keymap", c_keymap_free},
  {"keymap_dump", c_keymap_dump}, {"rl_add_undo", c_add_undo}, {"rl_message", c_message},
  {"rl_get_screen_size", c_get_screen_size}, {"rl_timeout_remaining", c_timeout_remaining},
  {"rl_completion_matches", c_completion_matches}, {"gen", c_gen},
  {"rl_display_match_list", c_display_match_list}, {"display_match_list", c_display_match_list},
  {"rl_save_state", c_save_state}, {"rl_restore_state", c_restore_state},
  {"rl_vi_domove", c_vi_domove}, {"rl_push_macro_input", c_push_macro_input},
  {"sleep_ms", c_sleep_ms}, {"snap", c_snap}, {"exit", c_exit}, {"option", c_option},
};

static void run_cmd(cmd_t *c) {
  const char *name = c->v[0];
  if (!name) { rec_error(c, "NULL command"); return; }
  for (size_t i = 0; i < sizeof custom_table / sizeof custom_table[0]; i++)
    if (strcmp(custom_table[i].name, name) == 0) { custom_table[i].fn(c); return; }
  const api_t *api = api_find(name);
  if (api) { run_api(c, api); return; }
  rec_error(c, "unknown command");
}

/* ------------------------------------------------------------------ */
/* main                                                                */

static int has_option(const char *opt) {
  block_t *m = &blocks[0];
  for (int i = 0; i < m->n; i++)
    if (m->cmds[i].n == 2 && m->cmds[i].v[0] && strcmp(m->cmds[i].v[0], "option") == 0 &&
        m->cmds[i].v[1] && strcmp(m->cmds[i].v[1], opt) == 0)
      return 1;
  return 0;
}

static int env_fd(const char *name) {
  const char *s = getenv(name);
  if (!s || !*s) return -1;
  int fd = atoi(s);
  unsetenv(name);
  return fd;
}

static void list_commands(void) {
  printf("# driver commands (see docs/TESTING.md)\n");
  for (size_t i = 0; i < sizeof custom_table / sizeof custom_table[0]; i++)
    printf("%s\n", custom_table[i].name);
  printf("# generic API calls: NAME SIGNATURE (R:ARGS, see api_t in driver/rl-driver.c)\n");
  for (size_t i = 0; i < sizeof api_table / sizeof api_table[0]; i++)
    printf("%s %s%s\n", api_table[i].name, api_table[i].sig, api_table[i].capture ? " (captures output)" : "");
  printf("# variables for set/get\n");
  for (size_t i = 0; i < sizeof var_table / sizeof var_table[0]; i++)
    printf("%s%s\n", var_table[i].name, var_table[i].ro ? " (read-only)" : "");
  printf("# hooks for set/get\n");
  hooks_init();
  for (int i = 0; i < H_COUNT; i++) printf("%s\n", hooks[i].name);
}

int main(int argc, char **argv) {
  if (argc == 2 && strcmp(argv[1], "--commands") == 0) {
    list_commands();
    return 0;
  }
  if (argc != 2) {
    fprintf(stderr, "usage: rl-driver SCRIPT | --commands\n");
    return 64;
  }
  const char *res = getenv("RLD_RESULTS");
  if (res && *res) {
    results_fp = fopen(res, "w");
    if (!results_fp) die("cannot open %s: %s", res, strerror(errno));
  } else results_fp = stderr;
  unsetenv("RLD_RESULTS");
  sync_fd = env_fd("RLD_SYNC_FD");
  ctl_fd = env_fd("RLD_CTL_FD");
  if (sync_fd >= 0) fcntl(sync_fd, F_SETFD, FD_CLOEXEC);
  if (ctl_fd >= 0) fcntl(ctl_fd, F_SETFD, FD_CLOEXEC);

  parse_script(argv[1]);

  if (!has_option("nosetlocale")) setlocale(LC_ALL, "");

  if (pipe(sigpipe_fds) != 0) die("pipe: %s", strerror(errno));
  for (int i = 0; i < 2; i++) {
    fcntl(sigpipe_fds[i], F_SETFL, fcntl(sigpipe_fds[i], F_GETFL) | O_NONBLOCK);
    fcntl(sigpipe_fds[i], F_SETFD, FD_CLOEXEC);
  }
  install_sig(SIGINT, 0);
  install_sig(SIGQUIT, 0);
  install_sig(SIGWINCH, 0);
  install_sig(SIGTSTP, 0);
  install_sig(SIGTTIN, 0);
  install_sig(SIGTTOU, 0);
  install_sig(SIGCONT, 0);

  hooks_init();
  rl_getc_function = drv_getc;
  if (!has_option("nodefuns")) {
    rl_add_defun("driver-dump", fn_driver_dump, -1);
    rl_add_defun("driver-dump-full", fn_driver_dump_full, -1);
    rl_add_defun("driver-snap", fn_driver_snap, -1);
  }

  run_block(&blocks[0]);

  fflush(NULL);
  if (results_fp != stderr) fclose(results_fp);
  return 0;
}
