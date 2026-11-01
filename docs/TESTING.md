# lineread behavioural test suite

This document is the reference for writing, recording and running test cases
against GNU Readline 8.3 (the *reference*) and, later, the Rust
reimplementation. Read `PLAN.md` and `AGENTS.md` first.

Contents:

1. [Layout](#1-layout)
2. [Quick start](#2-quick-start)
3. [Case files](#3-case-files)
4. [Keystroke notation](#4-keystroke-notation)
5. [How a case is executed](#5-how-a-case-is-executed)
6. [Expectations, recording and comparison](#6-expectations-recording-and-comparison)
7. [Driver script language](#7-driver-script-language)
8. [Driver command reference](#8-driver-command-reference)
9. [Runner command line](#9-runner-command-line)
10. [Coverage](#10-coverage)
11. [Header/ABI conformance tier](#11-headerabi-conformance-tier)
12. [Linux (Docker)](#12-linux-docker)
13. [Pitfalls and tips](#13-pitfalls-and-tips)
14. [Extending the driver](#14-extending-the-driver)

---

## 1. Layout

```
include/readline/*.h     recreated public headers (declarations only)
driver/rl-driver.c       test driver; uses only the public API, compiled against include/
runner/lrt/              Python runner (casefile, keys, execute, compare, coverage, cli)
rlt                      runner entry point (./rlt --help)
tests/<area>/*.case      test cases (hand written)
tests/<area>/*.expected.json            recorded expectations (generated)
tests/<area>/*.expected.<platform>.json per-platform overrides (generated, rare)
conformance/check.py     header/ABI conformance tier
conformance/abi.<platform>.json         recorded header facts from GNU's headers
ref/build.sh             builds the reference outside the repo (../lineread-ref)
docker/                  Linux image + docker/run.sh
Makefile                 builds build/<platform>/<impl>/rl-driver
```

The reference lives in `../lineread-ref/` (override with `LINEREAD_REF`):
`src/` (git checkout of tag `readline-8.3`), `build-plain/`, `build-cov/`,
`build-asan/` (static `libreadline.a`), `gnu-include/` (symlinks used only to
record the conformance data). On Linux the build directories carry a
platform suffix (`build-plain-linux-aarch64`), so one checkout serves the host
and Docker. Never copy anything from `src/` into the repo.

Areas (first directory below `tests/`) are the unit of scoring:
`history`, `histexpand`, `tilde`, `bind`, `emacs`, `vi`, `complete`,
`display`, `callback`, `format` (format demo), plus `conformance`. Add new
areas freely (e.g. `kill`, `undo`, `isearch`, `signals`, `vars`); tags give a
second, finer grouping.

## 2. Quick start

```sh
ref/build.sh                 # once: fetch + build plain/cov/asan reference libs
./rlt                        # whole suite + conformance against the reference
./rlt tests/emacs            # one area
./rlt tests/emacs/editing.case::undo --show     # one case, print actual output
./rlt tests/vi --record --repeat 3              # (re)record expectations
./rlt --repeat 10 tests      # flakiness check
./rlt --impl asan tests      # reference + driver under ASan/UBSan
./rlt --coverage tests       # line/branch coverage of the reference sources
docker/run.sh                # the same suite on Linux
./rlt --json > summary.json  # machine readable per-area/tag/case scores
```

`./rlt` builds the driver (`make IMPL=...`) before running and creates its
virtualenv (`runner/.venv-<platform>`, needs `pyte`) on first use
(`runner/setup.sh`). The whole seed suite runs in about a second.

Workflow for a new case: write it, run `./rlt FILE::NAME --show` until the
output demonstrates the behaviour you want, then
`./rlt FILE --record --repeat 3` (refuses to record runs that differ), then
`./rlt FILE --repeat 5`, check the Linux run (`docker/run.sh ./rlt FILE`), and
commit the `.case` and `.expected.json` files together.

## 3. Case files

A `.case` file holds many cases. It is line oriented:

```
# comment (only outside sections)
tags: history api                  <- preamble: defaults for every case
--- file hist.txt                  <- preamble fixture, every case gets it
first
second

=== read-history                   <- a case: "=== NAME"
tags: histfile
env: HISTSIZE=5
--- script
read_history "hist.txt"
history_dump
--- keys
abc\r
```

* `=== NAME` starts a case. Names match `[A-Za-z0-9][A-Za-z0-9_.+-]*` and are
  unique per file. A case id is `PATH::NAME`, e.g.
  `tests/history/files.case::read-history`.
* Before the first `===` is the **preamble**. Its headers and sections are
  defaults for every case in the file: `tags` are concatenated, `env` and
  fixture files are merged (case wins), every other header/section is
  replaced if the case sets it.
* **Header lines** are `key: value` (outside sections). Blank lines and lines
  starting with `#` are ignored there.
* **Sections** start with `--- NAME [args]` and run until the next `---` or
  `===` line. Their bodies are verbatim (no comments, no escapes). Trailing
  blank lines of a body are dropped; every body line ends with `\n`.

### Headers

| header | meaning |
|---|---|
| `tags: a b c` | tags (whitespace separated, accumulate with the preamble) |
| `description: text` | free text, informational |
| `screen: COLSxROWS` | terminal size, default `80x24` |
| `env: NAME=value` | environment override (repeatable); `$HOME` and `$BASE` in the value are replaced by the case directories |
| `env: NAME` | remove NAME from the environment |
| `timeout: SECONDS` | per-case timeout, default 10 |
| `ignore: PATTERN...` | field paths not recorded/compared, glob syntax (`*` and `?` are wildcards, brackets are literal): `screen`, `cursor`, `snaps`, `results[*].undo`, `results[2]`, `results[*].size` |
| `skip: reason` | do not run |
| `platforms: linux darwin` | run only on these platforms |
| `eof: on\|off` | send EOF after the last key (default on, see §5) |
| `signals: on\|off` | treat `^C ^\ ^Z` in keys as signal keys (default on, see §5) |
| `keys: ...` | single-line form of the keys section |
| `script: ...` | single-line form of the script section |

### Sections

| section | meaning |
|---|---|
| `--- script` | driver script (required, may come from the preamble), §7 |
| `--- keys` | keystrokes (§4). Newlines in the body are ignored, so long input can be split over lines; all other characters (including spaces) count |
| `--- inputrc` | contents of the init file; `INPUTRC` always points at this file (empty if absent), so `/etc/inputrc` and `~/.inputrc` never leak in |
| `--- file PATH [noeol] [mode=0644]` | fixture file relative to `$HOME`; `noeol` drops the final newline |
| `--- dir PATH [mode=0755]` | fixture directory |
| `--- symlink PATH TARGET` | fixture symlink |

Fixture paths must be relative and must not contain `..`. Content that has
lines starting with `--- `/`=== ` or needs exact bytes can be written by the
script instead (`file_write` with a heredoc and escapes).

## 4. Keystroke notation

Keys are sent to the pty exactly as a terminal would send them.

| notation | bytes |
|---|---|
| any UTF-8 character | itself |
| `\C-x` | control: `x & 0x1f` (`\C-?` = DEL 0x7f, `\C-@` = NUL) |
| `\M-x` | meta = ESC prefix: `\e x` (`\M-\C-x` = ESC ^X) |
| `\e` `\t` `\r` `\n` `\a` `\b` `\d` `\f` `\v` | ESC, TAB, CR (Enter), LF, BEL, BS, DEL, FF, VT |
| `\\` `\<` `\s` `\ ` | backslash, `<`, space, space |
| `\xHH` `\NNN` `\u{HEX}` | byte (hex/octal), Unicode code point as UTF-8 |
| `<up> <down> <left> <right> <home> <end>` | cursor keys; `\e[A` etc., or `\eOA` etc. when the terminal is in application cursor mode (DECCKM, e.g. after `enable-keypad`) |
| `<ins> <del> <pgup> <pgdn> <f1>`..`<f12> <btab>` | `\e[2~ \e[3~ \e[5~ \e[6~ \eOP`.. `\e[24~ \e[Z` |
| `<c-left> <m-right> <s-up>` ... | xterm modified cursor keys (`\e[1;5D`, `\e[1;3C`, `\e[1;2A`) |
| `<bs> <tab> <ret> <nl> <esc> <sp> <lt> <gt>` | DEL, TAB, CR, LF, ESC, space, `<`, `>` |
| `<paste-start> <paste-end>` | bracketed paste markers `\e[200~` / `\e[201~` |

Control items (they also split the input into chunks, see §5):

| item | meaning |
|---|---|
| `<pause>` | chunk boundary: wait until the driver has consumed everything and blocks for input |
| `<snap>` | take a screen snapshot (`snaps[i]` in the expectation) |
| `<dump>` / `<dumpfull>` | record the editing state (see `dump` in §8) between keys, independent of key bindings |
| `<resize:COLSxROWS>` | resize the pty (SIGWINCH) and the emulated screen; waits until readline handled it |
| `<sleep:MS>` | sleep (rarely needed; e.g. to let a readline timeout expire) |

`Enter` is `\r` (the runner's termios has ICRNL, readline turns it off and
binds both). Backspace on a terminal is DEL: use `<bs>` or `\C-?`.

## 5. How a case is executed

For every case (and every `--repeat`), the runner:

1. creates `/tmp/lrt-XXXXXXXX/` (`$BASE`, fixed length on every platform) with
   `home/` (`$HOME`, also the cwd), `inputrc`, `script`, `results.jsonl`, and
   writes the fixtures;
2. starts `rl-driver script` on a fresh pty as session leader with a pinned
   termios (`ISIG ICANON ECHO ECHOE ECHOK ECHOCTL ECHOKE IEXTEN`, `ICRNL`,
   **no `IXON`**, `OPOST ONLCR`, `CS8`; intr `^C`, quit `^\`, susp `^Z`,
   erase DEL, kill `^U`, werase `^W`, lnext `^V`, eof `^D`, reprint `^R`;
   dsusp/status/discard disabled) and a clean environment:
   `HOME`, `TERM=xterm-256color`, `LANG=C`, `LC_CTYPE=en_US.UTF-8` (macOS) /
   `C.UTF-8` (Linux), `LC_COLLATE=C`, `INPUTRC=$BASE/inputrc`, `COLUMNS`,
   `LINES`, `TZ=UTC`, `USER=LOGNAME=tester`, `SHELL=/bin/sh`, `TMPDIR=$BASE`,
   `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, then the case's `env:` lines;
3. feeds the keys chunk by chunk (below), emulating the terminal with `pyte`;
4. sends EOF when the keys are exhausted (unless `eof: off`), waits for the
   driver to exit (or the timeout), and collects results, screen and exit
   status. Strings in results and screens have the case paths replaced by
   `$HOME` / `$BASE`.

The driver calls `setlocale(LC_ALL, "")` at start (script `option
nosetlocale` disables that), so readline runs with UTF-8 ctype and C
collation unless the case changes `LC_*`/`LANG` via `env:`.

### Synchronisation: no timing in the protocol

The driver installs `rl_getc_function`. Whenever readline wants a byte and
none is pending, the driver reports "idle, N bytes consumed" on a pipe and
waits in `select()` for terminal input, control commands or signals. The
runner writes a chunk only after it saw an idle report covering all bytes
sent so far. Therefore:

* a **chunk** (the keys between control items) is written with a single
  `write()` and arrives at once: an escape sequence or `\eb` inside one chunk
  is never split by readline's `keyseq-timeout`, and typeahead that readline
  batches (e.g. self-insert of consecutive characters) is deterministic;
* `<pause>` forces readline to see the input up to that point *before* the
  rest exists. Use it after a lone `\e` that must act on its own (vi
  command mode, `\e` as a prefix in emacs). Readline then waits
  `keyseq-timeout` ms (default 500) before acting; set e.g.
  `set keyseq-timeout 50` in the inputrc to keep the case fast — the value
  does not influence the result, only the wall time;
* `<dump>`, `<snap>` and `<resize>` happen exactly between two keys;
* paren blinking (`blink-matching-paren`) and other readline-internal waits
  just take their time before the next idle report.

**Signal keys.** With `signals: on` (default), `^C`, `^\` and `^Z` in the
keys are sent as separate chunks and not counted as input (the tty turns them
into SIGINT/SIGQUIT/SIGTSTP and flushes its input queue); the runner waits
until the driver has processed the signal (a `signal` record appears in the
results). The driver handles INT, QUIT, WINCH, TSTP, TTIN, TTOU and CONT
("record" mode) so the process does not die or stop; `signal INT default`
etc. changes that. When readline is not catching signals or a key is read
with signals disabled (quoted-insert), use `signals: off` and a `<pause>`
before the control character (see `tests/format/features.case`).

**EOF.** After the last chunk the runner tells the driver that input is
exhausted; from then on the driver's `rl_getc_function` returns `EOF`, as a
real end of file would. `readline` then returns NULL (possibly after first
treating EOF like `^D` on a non-empty line, which is readline behaviour).
With `eof: off` the script must stop reading by itself, otherwise the case
times out.

While the driver is not reading (API calls, results output), no keys are
sent: keys are only consumed by `readline`, the callback loop, `rl_read_key`
and `rl_getc`. A script without any reading command ignores its keys.

### What is recorded

* `results`: the JSON records the driver wrote, in order (§7);
* `screen`: rows of the emulated terminal after the driver exited, trailing
  whitespace stripped, trailing empty rows dropped;
* `cursor`: `[row, col]` of the terminal cursor at exit;
* `snaps`: `{"screen", "cursor"}` per `<snap>` / `snap` command;
* `exit`: exit status (`0`, `3`, `"signal:SIGSEGV"`, `"timeout"`);
* `sanitizer`: ASan/UBSan report, only present when one was produced (always
  a failure).

The screen is compared, never raw bytes: any output that produces the same
terminal state is equivalent.

## 6. Expectations, recording and comparison

`tests/a/b.case` has its expectations in `tests/a/b.expected.json`, a JSON
object keyed by case name, written in a stable diff-friendly layout (one
result record or screen row per line). Do not edit it by hand; regenerate
with `--record`.

* `./rlt PATHS --record [--repeat 3]` runs the selected cases against the
  reference (`--impl ref` unless given) and rewrites their entries. Cases
  whose repeated runs differ are reported as `FLAKY` and not recorded.
  Entries of cases that no longer exist in the `.case` file are removed;
  entries of cases not selected are kept.
* `--record-platform` writes `b.expected.<platform>.json` (`darwin` or
  `linux`) containing only the cases whose result on this platform differs
  from the common file, and removes overrides that became identical. When
  checking, an override replaces the common entry for that case. Use this
  only for genuine platform differences (e.g. `~root` is `/var/root` on
  macOS); prefer making the case portable.
* Fields matched by `ignore:` are dropped before recording and comparing.

Comparison is field-wise: every key of every result record, every screen
row, the cursor, the exit status and every snapshot row is one *field*. A case
passes when all fields match; `fields_passed/fields_total` gives partial
credit in the reports and the per-area percentages. Failures print the first
mismatching field paths with expected and actual values (`--max-diff N`).

## 7. Driver script language

One command per line; leading whitespace is ignored; `#` at the start of a
line is a comment. Tokens are separated by whitespace:

| token | value |
|---|---|
| `bare` | the characters up to the next whitespace |
| `NULL` | a NULL pointer (for string args); `"NULL"` is the string |
| `"double"` | C escapes: `\n \t \r \e \a \b \f \v \\ \" \xHH \NNN`; any other backslash sequence is kept verbatim (`"\C-a"` stays `\C-a`, handy for readline key notation) |
| `'single'` | raw, no escapes, cannot contain `'` |
| `<<TAG` (last token) | heredoc: the following lines up to a line `TAG`, each with `\n` |

Integer arguments accept decimal, `0x` hex, `0` octal, or a **quoted key**
that denotes exactly one byte: `"a"`, `"\C-a"`, `"\M-x"` (= `x|0x80`),
`"\e"`, `"^X"`. Key sequence *strings* given to readline functions are
passed through unchanged: `rl_bind_keyseq '\C-x\C-r' ...` lets readline
translate the escapes, while functions that take raw bytes
(`rl_function_of_keyseq`) need real bytes: `"\x18\x12"`.

Blocks define sub-scripts used by hooks, bindable functions and `call`:

```
def NAME
  commands...
end
```

### Result records

Each command that produces a value writes one JSON object to the results:
`{"op": "<command line as written>", ...}`. Most return `"ret"`; some add
fields (`out` for captured output, `index`, `len`, ...). Commands without a
value (void API calls, `set`, `file_write`, ...) write nothing unless they
fail. Errors never abort the script: `{"op": ..., "error": "unknown
function"}`.

Special records: `{"op": "readline", "ret": ...}`, `{"op": "callback",
"ret": ...}`, `{"op": "dump", ...}`, `{"op": "hook", "hook": NAME,
...args}`, `{"op": "signal", "sig": "INT"}`, `{"op": "echo", "args": [...]}`,
`{"op": "capture", "out": ...}`.

Strings are byte exact: valid UTF-8 as is, control characters escaped,
invalid bytes as lone surrogates `\udcXX`. `HIST_ENTRY` values are
`{"line", "timestamp", "data"}` (`data` as an integer). Timestamps and
`time_t` values within two days of the current time are shown as `<now>`
(readline stamps entries with the wall clock); this applies to `file_read`
contents too.

## 8. Driver command reference

`build/<platform>/ref/rl-driver --commands` prints the authoritative list of
commands, API signatures, variables and hooks.

### Reading input

| command | description |
|---|---|
| `readline PROMPT [COUNT\|all] [addhist]` | call `readline(PROMPT)` COUNT times (default 1) or until it returns NULL (`all`); records each return value, frees it; `addhist` adds non-empty lines to the history |
| `callback PROMPT [COUNT\|all] [addhist]` | install `rl_callback_handler_install(PROMPT, handler)`, loop `select` + `rl_callback_read_char()` until the handler got COUNT lines (default 1) or NULL, then `rl_callback_handler_remove()`. The handler records `{"op":"callback","ret":line}` and frees the line |
| `callback_install PROMPT [addhist]` (alias `rl_callback_handler_install`) | install only |
| `callback_read_char [N]` (alias `rl_callback_read_char`) | wait for input, call `rl_callback_read_char()` N times |
| `onsig BLOCK` | in the callback loop, run BLOCK when a signal arrives (e.g. `rl_callback_sigcleanup`) |
| `rl_read_key`, `rl_getc` | read through the synchronised input path |

### State, output and control

| command | description |
|---|---|
| `dump [LABEL]` | record `line point end mark mode keymap explicit_arg numeric_arg insert_mode` |
| `dumpfull [LABEL]` | additionally `state` (RL_STATE_* names), `undo` (`rl_undo_list` as `[what,start,end,text]`), `prompt display_prompt last_func done eof_found pending_input mark_active executing_key executing_keyseq executing_macro history_length history_base history_pos` |
| `snap` | ask the runner for a screen snapshot now (usable inside hooks/bound functions) |
| `capture COMMAND...` | run COMMAND with `rl_outstream` redirected to a temp file, then record `{"op":"capture","out":...}` |
| `echo ARGS...` | record the arguments (markers) |
| `get VAR` / `set VAR VALUE...` | read/write an exported variable or hook (below) |
| `call BLOCK` | run a block |
| `repeat N BLOCK` | run a block N times (long histories, big files, many funmap entries) |
| `defun NAME BLOCK [KEY]` | `rl_add_defun(NAME, f, KEY)` where `f` runs BLOCK (up to 32) |
| `invoke FUNCTION [COUNT] [KEY]` | call a bindable function by name (`rl_named_function`) |
| `args` | inside a hook/defun: record its arguments |
| `return VALUE` | inside a hook/defun: set its return value (int, string or `NULL`) |
| `replace_arg STRING` | inside a directory hook: replace `*dirname` |
| `signal SIG record\|default\|ignore` | driver disposition for INT QUIT WINCH TSTP TERM HUP ALRM CONT TTIN TTOU USR1 USR2 |
| `raise SIG` | `kill(getpid(), SIG)` |
| `option nodefuns` / `option nosetlocale` | (anywhere in the main script) skip registering the driver's bindable functions / the `setlocale` call |
| `sleep_ms N`, `exit [CODE]` | |

Bindable functions registered by the driver (bind them in the inputrc or
with `rl_bind_keyseq`): `driver-dump`, `driver-dump-full`, `driver-snap`.
They show up in `rl_funmap_names` and the function dumper.

### Files and environment

`file_write PATH CONTENT`, `file_append PATH CONTENT`, `file_read PATH`
(→ `content` or `null` + `errno`), `file_remove PATH`, `file_stat PATH`
(→ `exists mode type [size]`), `mkdir PATH`, `chmod PATH OCTAL`,
`symlink TARGET PATH`, `hardlink TARGET PATH`, `mkfifo PATH`, `mksocket PATH`
(a bound unix socket file, for file type indicators), `setenv NAME VALUE`, `unsetenv NAME`, `getenv NAME`,
`chdir PATH`.

### Completion helpers

| command | description |
|---|---|
| `complete_words [-i] W...` | word list for the `words` generator (`-i`: match case-insensitively) |
| `ignore_words W...` | words removed by the `ignore_words` value of `rl_ignore_some_completions_function` |
| `tilde_map WORD VALUE` | table for the `table` value of the tilde hooks |
| `gen GENERATOR TEXT [sort]` | call a generator with state 0,1,... until NULL; record all results |
| `rl_completion_matches TEXT GENERATOR [sort]` | record the match vector |
| `display_match_list [LCD W...]` | `rl_display_match_list`; without args inside the display hook it uses the hook's matches |

Generators: `words`, `filename` (`rl_filename_completion_function`),
`username` (`rl_username_completion_function`), `empty`. `sort` sorts the
results (readdir order is filesystem specific).

### Hook values (`set HOOK VALUE`)

Every hook accepts `NULL`, `default` (value at startup), `log` (record
`{"op":"hook",...args}` and behave neutrally), `script:BLOCK` (run BLOCK; it
can use `args`, `return`, `replace_arg`, any command). Neutral behaviour /
default return when the block does not `return`:

| hook | args | default behaviour |
|---|---|---|
| `rl_startup_hook rl_pre_input_hook rl_signal_event_hook rl_timeout_event_hook rl_input_available_hook rl_event_hook` | – | return 0 (`rl_event_hook` also keeps the runner protocol alive) |
| `rl_redisplay_function` | – | call `rl_redisplay()` unless a block is set |
| `rl_prep_term_function` / `rl_deprep_term_function` | `meta` | call `rl_prep_terminal` / `rl_deprep_terminal` unless a block is set |
| `rl_macro_display_hook` | `keyseq value readable prefix` | – |
| `rl_completion_entry_function`, `rl_menu_completion_entry_function` | values are generator names: `words filename username empty` | |
| `rl_attempted_completion_function` (`words`, `log`, `script:`) | `text start end` | return `rl_completion_matches(text, words)`; `return NULL` → NULL, `return filename` etc. → that generator. Set completion variables (e.g. `rl_completion_append_character`) *inside* the block: readline resets them before each completion |
| `rl_ignore_some_completions_function` (`ignore_words`) | `matches` | drop `ignore_words` entries, return 0 |
| `rl_completion_display_matches_hook` | `matches num_matches max_length` | nothing is displayed unless the block does (`display_match_list`, `rl_forced_update_display`) |
| `rl_completion_word_break_hook` | – | NULL (default breaks) |
| `rl_directory_completion_hook rl_directory_rewrite_hook rl_filename_stat_hook` | `dirname` | return 0 |
| `rl_filename_rewrite_hook rl_completion_rewrite_hook` | `text len` | return the argument unchanged |
| `rl_filename_quoting_function` | `text match_type quote_char` | return a copy of text |
| `rl_filename_dequoting_function` | `text quote_char` | remove backslashes |
| `rl_char_is_quoted_p` (`backslash`) | `text index` | quoted iff preceded by a backslash |
| `tilde_expansion_preexpansion_hook tilde_expansion_failure_hook` (`table`) | `word` | look up `tilde_map`, else NULL |
| `history_inhibit_expansion_function` | `string index` | return 0 |

`rl_getc_function` is owned by the driver and cannot be set.

### Variables (`get VAR`, `set VAR VALUE`)

All exported variables of the headers are available (see `--commands`).
Integers take numbers or quoted keys (`set rl_completion_append_character
"/"`); `char` variables likewise (`set history_comment_char "#"`, `0` for
NUL); strings are copied (`NULL` allowed); `tilde_additional_prefixes` /
`_suffixes` take several strings; `rl_instream`/`rl_outstream` take `stdin
stdout stderr NULL file:PATH`; `rl_last_func` a function name. Read-only in
the driver: `rl_line_buffer rl_prompt rl_display_prompt rl_library_version
rl_executing_keymap rl_binding_keymap rl_executing_keyseq rl_executing_macro
rl_undo_list funmap`. `get rl_readline_state` returns flag names; `get
funmap` the function names in funmap order.

### Generic API calls

Most public functions are callable directly by name with a typed signature
`R:ARGS` (printed by `--commands`):

* ARGS: `i` int, `k` key (int or quoted key), `s` string (copied, freed
  after the call), `r` string retained by the library, `f` bindable function
  name (`NULL` allowed), `m` keymap name, `d` history data (integer).
* R (what is recorded as `ret`, and what the driver frees): `v` nothing (no
  record), `i` int, `t` time_t, `s` borrowed string, `S` string freed by the
  driver, `L` string vector freed (elements and array), `N` string vector
  (array freed only, e.g. `rl_funmap_names`), `H` borrowed `HIST_ENTRY`, `F`
  `HIST_ENTRY` freed with `free_history_entry`, `R` vector of entries freed,
  `f` function name, `m` keymap name. Dumper-style functions capture
  `rl_outstream` into `out`.

Every bindable command is callable as `rl_<name> COUNT KEY`
(e.g. `rl_forward_word 1 0`) — generated from the header. Calling editing
functions outside `readline()` requires `rl_initialize` first (the line
buffer does not exist before).

Keymap names: `emacs emacs-standard emacs-meta emacs-ctlx vi vi-move
vi-command vi-insert`, `current` (`rl_get_keymap()`), names created with
`keymap_new` and names set with `rl_set_keymap_name`. Keymaps without a name
are reported as `?anon`, unknown function pointers as `?unknown`.

Custom (non-generic) API commands:

| command | records |
|---|---|
| `history_expand STRING` | `ret`, `out` |
| `history_dump` | `base length max stifled pos entries` |
| `history_list` | `ret`: entries up to the NULL terminator |
| `history_get_time OFFSET` | `history_get_time(history_get(OFFSET))` |
| `history_get_history_state` | `offset length size flags` |
| `history_state_save SLOT` / `history_state_restore SLOT` | get/set_history_state (slots 0..7) |
| `copy_history_entry OFFSET` | copy of `history_get(OFFSET)` |
| `get_history_event STRING INDEX [QCHAR]` | `ret`, `index` |
| `rl_translate_keyseq SEQ` | `ret`, `out`, `len` |
| `rl_function_of_keyseq BYTES MAP`, `rl_function_of_keyseq_len BYTES LEN MAP` | `type` (`func keymap macro` or null), `value` |
| `rl_trim_arg_from_keyseq BYTES LEN MAP` | `ret` |
| `rl_generic_bind ISFUNC\|ISKMAP\|ISMACR KEYSEQ DATA MAP` | DATA is a function name, keymap name or macro text |
| `keymap_new NAME bare\|full\|copy [SRC]` | `rl_make_bare_keymap` / `rl_make_keymap` / `rl_copy_keymap` |
| `rl_discard_keymap MAP`, `rl_free_keymap MAP` | |
| `keymap_dump MAP` | all non-empty entries `[keyseq, type, value]`, unnamed submaps expanded |
| `rl_add_undo delete\|insert\|begin\|end START END TEXT` | |
| `rl_message TEXT` | `rl_message("%s", TEXT)` |
| `rl_get_screen_size` | `rows cols` |
| `rl_timeout_remaining` | `ret` |
| `rl_save_state SLOT` / `rl_restore_state SLOT` | slots 0..3 |
| `rl_vi_domove KEY` | `ret nextkey` |
| `rl_push_macro_input TEXT` | (the library frees the copy) |

`tilde_find_word` is declared in `tilde.h` but not defined by the reference
library, so the driver does not call it.

## 9. Runner command line

```
./rlt [PATH|PATH::NAME ...] [options]
```

| option | |
|---|---|
| `--impl ref\|cov\|asan\|rust` | which build to link the driver against (default `ref`) |
| `-t TAG` / `-T TAG` | only / exclude cases with TAG (repeatable) |
| `-k GLOB` | case name or id glob (repeatable) |
| `-j N` | parallel jobs (default: CPU count) |
| `--repeat N` | run every case N times; differing results are `FLAKY` |
| `--record`, `--record-platform` | write expectations (§6) |
| `--show` | print actual results, snapshots and screen of every selected case |
| `--trace` | log the runner/driver protocol and raw output (implies `-j1`) |
| `--keep` | keep the per-case directories |
| `--list` | list selected cases |
| `--json` | JSON summary on stdout: counts, per-area and per-tag `passed/total/fields_passed/fields_total`, per-case status and mismatches |
| `-v` | list passing cases too |
| `--coverage [--coverage-html DIR]` | coverage run (§10) |
| `--conformance` / `--no-conformance` | include the conformance tier (default: included when no paths/filters are given) |
| `--driver PATH`, `--no-build`, `--timeout-scale F`, `--max-diff N` | |

Exit status is 1 if any case failed, errored (e.g. missing expectation) or was
flaky. The driver binary is `build/<platform>/<impl>/rl-driver`; `IMPL=rust`
links `impl/target/release/liblineread.a` (override `RUST_LIB=`).

## 10. Coverage

`./rlt --coverage [PATHS]` builds the driver against
`../lineread-ref/build-cov` (clang `-fprofile-instr-generate
-fcoverage-mapping`), runs the cases with `LLVM_PROFILE_FILE` pointing into
`build/<platform>/coverage/raw/`, merges with `llvm-profdata` and prints per
file line/branch/function coverage of the reference sources
(`build/<platform>/coverage/summary.json`). `--coverage-html DIR` writes an
annotated HTML report (`DIR/index.html`) — use it to find untested code. On
macOS the tools come from `xcrun`, on Linux from the `llvm` package.

## 11. Header/ABI conformance tier

`conformance/check.py` compares our headers with facts recorded from GNU's
headers (`conformance/abi.<platform>.json`, committed):

* `decl:NAME` – every function, variable, typedef, struct/union and enum with
  its full type, from clang's AST;
* `sizeof(T)`, `alignof(T)`, `offsetof(T,field)`, `enum:NAME` – evaluated by a
  generated probe program compiled against our headers;
* `macro:NAME` – value of integer/string macros, presence of empty macros,
  token text of the others; `macro:NAME(c)` – character macros evaluated for
  c = -1..256;
* `symbol:NAME` – every declared function/variable is defined in the
  implementation's static library (`nm`), unless the reference does not
  define it either; `link` – a program taking the address of every symbol
  links against the library.

`python3 conformance/check.py --impl rust` runs it alone; `./rlt` includes it
as area `conformance`. `--record` regenerates the facts (needs
`../lineread-ref/gnu-include`, i.e. only before the clean-room cut); record on
each platform (Linux: `docker/run.sh python3 conformance/check.py --record`).

## 12. Linux (Docker)

`docker/run.sh [COMMAND...]` builds a Debian bookworm image (clang, llvm,
compiler-rt, ncurses, python + pyte) and runs COMMAND with the repo mounted
at `/work/lineread` and `../lineread-ref` at `/work/lineread-ref`. Without a
command it runs `ref/build.sh && ./rlt`. Examples:

```sh
docker/run.sh ref/build.sh            # once: Linux reference builds
docker/run.sh ./rlt --repeat 3
docker/run.sh ./rlt tests/tilde --record-platform
docker/run.sh sh                      # shell
```

The same expectations must pass on macOS and Linux.

## 13. Pitfalls and tips

* **One behaviour per case**, many small cases; name them after the
  behaviour. Group related cases in a file with a shared preamble.
* Call `using_history` before history string searches (`!str`, `!?str?`,
  `history_search`): `add_history` does not move `history_offset`.
* `history_comment_char` is NUL by default, so `history_get_time` of a
  `"#123"` timestamp is 0 until you `set history_comment_char "#"`.
* Do not depend on machine specifics: user names/home directories (`~root`
  needs a platform override), readdir order (`gen ... sort`), the real home
  directory, the current time (normalised to `<now>`), absolute paths on the
  screen (the case directory is `/tmp/lrt-XXXXXXXX`, same length everywhere,
  but a path split across a line wrap is not normalised).
* Byte-level vs. UTF-8 behaviour: the default ctype is UTF-8; use
  `env: LC_CTYPE=C` for byte semantics. Wide character widths come from the
  platform's `wcwidth`; keep to common CJK/accented characters.
* Readline functions that need an initialised line buffer (`rl_insert_text`,
  `rl_delete_text`, bindable commands) must not be called before
  `rl_initialize` or `readline`.
* Typeahead: readline may read several queued bytes at once (e.g. inserting
  "abc" in one go, or `rl_callback_read_char` consuming more than one byte).
  This is deterministic thanks to chunking; insert `<pause>` to force
  one-key-at-a-time behaviour when that is what you test.
* Use `<dump>` for state between keys and `driver-dump` (bound key) or
  `dump` inside a block for state in the middle of a command or hook.
* The kill ring is not part of the public API: observe it through yanking
  (`\C-y`, `\M-y`).
* A signal key flushes the tty input queue: keys after `^C` in the same
  chunk would be lost, hence the automatic chunking. Keys sent while the tty
  is in cooked mode (after `rl_deprep_terminal` or `rl_cleanup_after_signal`)
  are echoed by the tty and only delivered after a newline.
* `rl_attempted_completion_function` blocks are where per-completion
  variables must be set (`rl_completion_append_character`,
  `rl_completion_suppress_quote`, ...). Application-global settings that
  readline consults *before* calling that function
  (`rl_completer_quote_characters`, `rl_completer_word_break_characters`,
  `rl_filename_quote_characters`, `rl_special_prefixes`) must be set before
  `readline`.
* The second `TAB` only lists matches if the first one did not change the
  line (inserting a common prefix counts as a change); use `\M-?`
  (possible-completions) to list unconditionally.
* Colors are not visible in screen snapshots (only characters are
  compared). Test colored listings with
  `capture display_match_list LCD MATCH...` after `rl_initialize` and
  `set rl_filename_completion_desired 1`; `rl_crlf` output still goes to the
  terminal, the match text goes to the capture.
* `gen` and `rl_completion_matches` called outside `readline` see init file
  settings (`completion-ignore-case` etc.) only after `rl_initialize`.
* Error messages of the init file parser, LS_COLORS parsing and history
  expansion go to stderr, i.e. onto the emulated screen; widen the screen
  (`screen: 160x24`) so long paths do not wrap.
* The driver `tcdrain`s the terminal before exiting: on macOS a pty master
  can otherwise lose output still queued when the slave closes.
* Prompt escapes: `"\001...\002"` mark invisible characters
  (`RL_PROMPT_START_IGNORE`/`END_IGNORE`).
* Check a new case with `--repeat 5` and on Linux before committing.

## 14. Extending the driver

* A plain function with a supported signature: add `API(name, "R:ARGS")` (or
  `APIC` to capture `rl_outstream`) to `api_table` in `driver/rl-driver.c`.
  If no call shape exists for the argument pattern, extend the generated
  `call_shape` (`I`/`P` per argument, up to three arguments).
* Anything with out-parameters, structs or ownership subtleties: write a
  `c_NAME(cmd_t *c)` handler (use `NEED`, `INTARG`, `KEYMAPARG`, `rec_*`) and
  add it to `custom_table`.
* A new variable: add it to `var_table`; a new hook: an `H_*` id, a
  trampoline and a `HOOKDEF` line.
* Free exactly what the API documentation says the caller owns, so the ASan
  run (`--impl asan`) catches ownership bugs in implementations.
* Keep the driver on the public API only and compile it against
  `include/readline` (the Makefile does).
