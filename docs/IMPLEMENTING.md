# Implementing lineread (phase 5)

You are writing a Rust library that exports the GNU Readline 8.3 C API/ABI
(readline, history and tilde) and hill-climbing the behavioural test suite
recorded from the reference. Read `PLAN.md`, `AGENTS.md` and
`docs/TESTING.md` first.

## Clean-room rules

* Never look at GNU Readline source code (not in this repo, not online, not
  in other projects that vendor it). There is none in this repository; the
  reference exists only as compiled libraries/drivers in
  `../lineread-ref-bin/` (a black-box oracle).
* Allowed sources of semantics: the public documentation — the
  `readline(3)` and `history(3)` man pages and the GNU Readline / GNU
  History info manuals ("GNU Readline Library", "GNU History Library"),
  the recreated headers in `include/readline/` (declarations only), and the
  tests. **Behaviour is defined by the tests**; where the documentation and
  the tests disagree, the tests win.
* You may probe the reference: write a case, run it with `./rlt --impl ref
  --show` (or `--driver ../lineread-ref-bin/<platform>/rl-driver`), and
  record it (`--record`) to pin down a behaviour. New cases follow
  docs/TESTING.md (deterministic, `--repeat 5`, Linux check).
* Do not edit recorded expectations by hand.

## Layout and build

```
impl/                       cargo package (you create it)
  Cargo.toml                package "lineread", [lib] crate-type = ["staticlib", "cdylib"]
  build.rs                  compiles the C shim (see below)
  c/shim.c                  rl_message and anything else Rust cannot express
  src/lib.rs ...
impl/target/release/liblineread.a   what the driver links (IMPL=rust)
```

* One crate exports everything: the readline functions/variables, the
  history library (`add_history`, `history_expand`, `read_history`, ...)
  and the tilde library (`tilde_expand`, ...). GNU ships `libreadline`
  (which contains the history functions too) and `libhistory`; a single
  archive with all symbols is fine for the tests and the apps tier. For
  distribution you can later produce `libreadline.so` and `libhistory.so`
  from the same code (cdylib).
* `make IMPL=rust` runs `cargo build --release --manifest-path
  impl/Cargo.toml` and links `build/<platform>/rust/rl-driver` (our headers
  + the driver + `liblineread.a` + `-lncurses`, and `-lpthread -ldl -lm` on
  Linux). To link another archive set the environment variable
  `RUST_LIB=/abs/path/lib.a` (cargo is then skipped).
* The driver references every public function and variable, so it only
  links once all symbols exist. Start by stubbing the whole surface: the
  list of declarations with their C types is in
  `conformance/abi.<platform>.json` (`surface`: `FunctionDecl` /
  `VarDecl`), and `./rlt --impl rust` reports missing symbols in the
  conformance tier (`symbol:NAME`, `link`).
* Toolchain: stable Rust, clang, ncurses (the `terminfo` crate is planned
  for capabilities; `-lncurses` stays on the link line for now). Platforms:
  macOS arm64 and Linux aarch64 (`docker/run.sh` provides the Linux
  environment with the repo and `../lineread-ref-bin` mounted, clang and a
  stable Rust toolchain; use a separate target directory there, e.g.
  `docker/run.sh sh -c 'CARGO_TARGET_DIR=build/linux-target RUST_LIB=...'`
  or just `docker/run.sh ./rlt --impl rust`, which builds into
  `impl/target` — run `cargo clean` when switching platforms, or set
  `CARGO_TARGET_DIR` and `RUST_LIB` accordingly).
* The wiring was verified with a stand-in crate that bundles the reference
  library into `liblineread.a`: it scores 100 on Linux and 99.99 on macOS
  (the two vi ref-quirk cases, see below).

## Running the score

```sh
./rlt --impl rust                 # whole suite + conformance tier, failures listed
./rlt --impl rust --score         # one line: SCORE 87.123 cases=... fields=...
./rlt --impl rust --json > s.json # per-area / per-tag / per-case details
./rlt --impl rust tests/emacs     # one area
./rlt --impl rust tests/emacs/movement.case::forward-char-count --show
./rlt --impl rust -t vi-delete-to # by tag
./rlt --impl rust --max-diff 50   # more mismatching fields per case
./rlt --impl rust --trace FILE::NAME   # protocol + raw terminal bytes
./rlt --impl rust-asan            # driver under ASan/UBSan: ownership bugs
./rlt --impl ref --score          # the reference: 100 on every platform
docker/run.sh ./rlt --impl rust   # Linux
```

Reading a failure:

```
FAIL     tests/emacs/movement.case::forward-char-count (14/17 fields)
         results[1].point:
           expected 6
           actual   5
```

`results[i]` is the i-th record the driver wrote (see the case's script and
`<dump>` items), `screen[r]` a row of the emulated terminal at exit,
`snaps[i]` a `<snap>`, `exit` the exit status, `sanitizer` an ASan report.
`--show` prints the actual records and screens; the expected ones are in
the `.expected.json` next to the case file. The score is the mean per-case
credit (a failing case contributes the fraction of its matching fields), so
partial progress counts.

Areas (directory under `tests/`): history, histexpand, tilde, bind,
inputrc, complete, emacs, kill, undo, search, vi, display, utf8, callback,
signals, hooks, input, format, plus `conformance` (headers/ABI) and
`fuzz` (fuzzer regressions). `docs/AUDIT.txt` lists which cases exercise
each public name — use it to find the tests for the function you are
working on (`./rlt -t NAME` works for most names: cases are tagged with the
functions/variables they test).

## Differential fuzzing

```sh
./rlt fuzz --a ref --b rust -n 2000 --seed 1
```

Random cases (emacs/vi, inputrc variables, prompts, history, completion,
UTF-8, screen sizes) run against the reference and your library; on a
difference the case is minimised and written to `tests/fuzz/`. Record it
against the reference (`./rlt tests/fuzz/fuzz-1-42.case --record`), give
it a meaningful name/area if it shows a real behaviour, and fix the
implementation. Use new seeds over time. See docs/TESTING.md §16.

## Real programs

`tests/apps/build.sh rust && ./rlt apps --impl rust` builds the CPython
readline module, the sqlite3 shell and the Lua REPL against our headers and
your library and replays recorded pty sessions (tests/apps/README.md). Not
part of the score; run it once the core works. The Python module is a
shared object: the staticlib must be position independent (Rust's is).

## C ABI notes

* **Headers are fixed.** `include/readline/*.h` match GNU's declarations,
  struct layouts and constants (checked by the conformance tier). Mirror
  them exactly with `#[repr(C)]` types: `KEYMAP_ENTRY { char type;
  rl_command_func_t *function; }`, `KEYMAP_SIZE` 257, `UNDO_LIST`,
  `FUNMAP`, `HIST_ENTRY { char *line; char *timestamp; histdata_t data; }`,
  `HISTORY_STATE`, `struct readline_state` (with its reserved bytes).
* **Exported mutable globals.** Applications read and write variables
  directly (`rl_point`, `rl_end`, `rl_mark`, `rl_done`, `rl_line_buffer`,
  `rl_prompt`, `rl_instream`, `history_length`, `history_base`, every hook
  like `rl_attempted_completion_function`, ...). Export them as
  `#[no_mangle] pub static mut NAME: T` with the C type (hooks as
  `Option<unsafe extern "C" fn(..) -> ..>`, which has the nullable function
  pointer layout). Never cache them across calls into application code:
  re-read after every hook/callback/bound function, and write them back
  before calling out. Keymaps (`emacs_standard_keymap`,
  `vi_movement_keymap`, ...) are exported arrays of 257 entries;
  `rl_executing_keymap`, `funmap` (a `FUNMAP **` array) and friends are
  pointers into your data that applications may inspect.
* **Memory ownership.** Anything the caller frees must come from libc
  `malloc` (use `libc::malloc`/`strdup`, never Rust's allocator for memory
  that crosses the ABI): `readline()` results, `rl_copy_text`,
  `tilde_expand`, `history_tokenize`/`history_arg_extract`,
  `rl_completion_matches` arrays and their strings, `HIST_ENTRY`s returned
  by `remove_history`/`replace_history_entry` (freed with
  `free_history_entry`), `rl_invoking_keyseqs`, ... Conversely you free
  what the application hands over: strings returned by completion
  generators and by hooks such as the filename quoting/dequoting/rewrite
  functions, the matches array returned by `rl_attempted_completion_function`,
  the copy passed to `rl_push_macro_input`. The driver frees exactly what
  the documentation says the caller owns, so `--impl rust-asan` (and the
  ASan fuzzer self-check) find double frees and leaks of ownership.
  `rl_free` must be `free`.
* **Variadic `rl_message(const char *fmt, ...)`.** Stable Rust cannot
  define C variadic functions: compile a small C shim in `build.rs`
  (invoke `cc`/`clang` and `ar`, or the `cc` crate if you vendor it) that
  formats with `vsnprintf` and calls a Rust `extern "C"` function with the
  resulting string, and link it into the staticlib
  (`cargo:rustc-link-lib=static=...` with `cargo:rustc-link-search`; rustc
  bundles static native libraries into the staticlib).
* **FILE\* streams.** `rl_instream`/`rl_outstream` are stdio `FILE *`
  (NULL means stdin/stdout at `readline()`/`rl_initialize()` time). Write
  output through the FILE (`fwrite`/`fputs`/`fflush` from libc), because
  applications and tests interleave their own stdio with yours (e.g. the
  driver redirects `rl_outstream` to a temporary file to capture dumper
  output). Read through `rl_getc_function` (below); `rl_instream` may also
  be a regular file.
* **Input.** Every byte must be read through `rl_getc_function` (default
  `rl_getc`): the test driver installs its own function to synchronise with
  the runner, and also calls `rl_getc` itself. `rl_getc` must handle EINTR,
  `rl_set_timeout`, `rl_signal_event_hook` (see tests/input/*).
* **Signals.** With `rl_catch_signals`, readline installs handlers for
  INT, QUIT, TERM, HUP, ALRM, TSTP, TTIN, TTOU (and WINCH with
  `rl_catch_sigwinch`), cleans up, restores the terminal, re-sends the
  signal to the application's handler and then resets (tests/signals).
* **Terminal.** Capabilities come from terminfo for `$TERM` (tests use
  xterm-256color mostly, also vt100, vt102, ansi, linux, dumb); screen size
  from the tty (`TIOCGWINSZ`), `COLUMNS`/`LINES`, `rl_prefer_env_winsize`.
  Only the resulting screen is compared, not the bytes: redraw however you
  like as long as the terminal ends up in the same state.
* **Locale.** The driver calls `setlocale(LC_ALL, "")`; tests run with a
  UTF-8 `LC_CTYPE` (some with `C`). Multibyte handling follows `MB_CUR_MAX`
  / `mbrtowc`; display widths follow the platform's `wcwidth` (use
  `libc::wcwidth` to agree with the reference on CJK).
* **Symbols that are declared but not defined by the reference** (only
  `tilde_find_word`) are not required.

## Reference behaviours worth knowing

The tests encode the reference, including some odd corners. Notable ones:

* vi word motions (`w b e`, `dw`, ...) classify bytes, not characters: in
  the common expectations bytes >= 0x80 are not word characters (glibc
  behaviour). The macOS reference differs (Latin-1 classification); that is
  recorded as a reference-only quirk (`*.expected.ref-darwin.json`) and does
  not apply to you — implement the common behaviour on every platform.
* Bindable function names are matched case-insensitively and the first
  funmap entry wins, so `vi-eword` binds the same function as `vi-eWord`.
* With `rl_event_hook` set, readline calls the hook before reading input
  (tests only rely on the first call; see tests/input/timeout.case).
* The tests avoid a reference bug where a NUL byte arriving in the same
  read as typed-ahead text is dropped; you need not reproduce it.
* `docs/UNTESTED.md` lists the few names without behavioural cases and why.

## Workflow suggestion

1. Stub the full surface (conformance tier green: symbols + link).
2. History library and history expansion (pure functions, no terminal):
   tests/history, tests/histexpand, tests/tilde.
3. Keymaps, funmap, binding API, inputrc parser and variables: tests/bind,
   tests/inputrc.
4. The editing loop: input, dispatch, line buffer, redisplay for simple
   lines; then emacs commands, kill ring, undo, numeric arguments, macros,
   search; then completion; vi mode; callback interface; signals; display
   corner cases (wrapping, multibyte, prompts with invisible characters,
   horizontal scrolling).
5. Keep `./rlt --impl rust-asan` and the fuzzer clean; run the Linux suite
   and the apps tier regularly.
