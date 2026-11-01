# lineread implementation architecture (impl/)

How the Rust implementation is organised, the conventions every module
follows, and who owns what.  Read PLAN.md, AGENTS.md, docs/IMPLEMENTING.md
and docs/TESTING.md first.  Clean-room rules apply: no GNU Readline source,
ever.  Behaviour comes from the man pages / info manuals, our headers and
the tests; probe the black-box reference (§10) when in doubt.

## 1. Build

```
impl/
  Cargo.toml        crate "lineread": staticlib + cdylib, deps: libc, terminfo
  Cargo.lock        committed (offline builds)
  build.rs          compiles c/shim.c into liblineread_shim.a (bundled)
  c/shim.c          rl_message (variadic), MB_CUR_MAX, stdin/stdout/stderr,
                    ucontext coroutines for the callback interface
  src/              the library (module map below)
  tools/            gen_stubs.py, gen_tables.py (one-shot generators, kept for
                    reference), observed/ (reference observations used by
                    gen_tables.py), compare.py (diff two --json runs)
```

`./rlt --impl rust` builds with cargo (`impl/target/<platform>/`) and links
the driver.  Linux: `docker/run.sh ./rlt --impl rust` (cargo home persisted
in `impl/target/cargo-home-linux`).  `panic = "abort"`: a Rust panic never
unwinds into C.

## 2. Module map and ownership

Every public symbol is defined (`#[no_mangle]`) in the module that owns its
behaviour — never in a central file.  Workstreams (§12) own whole files;
cross-module changes are limited to calling functions the other module
exports (add a small `pub fn` in *your* module rather than editing theirs).

| module (impl/src/) | contents | test areas | workstream |
|---|---|---|---|
| `lib.rs` `prelude.rs` `cvar.rs` `types.rs` `util.rs` `mb.rs` | crate root, `CVar`, `#[repr(C)]` types, C string/alloc/stdio helpers, multibyte helpers | conformance | core (coordinate) |
| `history/mod.rs` | history list API and variables | history | **H** |
| `history/file.rs` | read/write/append/truncate history files | history | **H** |
| `history/search.rs` | `history_search*` | history | **H** |
| `history/expand.rs` | `history_expand`, `history_tokenize`, `history_arg_extract`, `get_history_event`, expansion variables | histexpand | **H** |
| `tilde.rs` | tilde library | tilde | **H** |
| `histcmds.rs` | previous/next/beginning/end-of-history, saved line, per-entry undo lists, operate-and-get-next, fetch-history, yank-nth-arg, yank-last-arg | emacs (history tags), kill | **H** |
| `search.rs` | incremental search, non-incremental search, history-search-*, substring search | search | **H** |
| `keymap/mod.rs` | keymap API, names, default key sequences bound at init | bind | **B** |
| `keymap/defaults.rs` | GENERATED default keymaps (gen_tables.py) | bind | **B** |
| `funmap.rs` `funmap_table.rs` | `funmap`, name lookup; GENERATED default order | bind | **B** |
| `bind.rs` | binding API, keyseq (un)translation, function_of_keyseq, invoking keyseqs, dumpers | bind | **B** |
| `inputrc.rs` | init file parser, `$if/$include`, error messages | inputrc | **B** |
| `vars.rs` | every inputrc variable (crate-visible globals), `rl_variable_*` | inputrc | **B** |
| `input.rs` | `rl_getc`, `rl_read_key`, pending/stuffed input, event hook, timeouts | input, hooks | **B** |
| `dispatch.rs` | key dispatch, prefix keymaps, keyseq-timeout, `rl_executing_*` | bind, emacs dispatch | **B** |
| `readline.rs` | `readline()`, `rl_initialize`, line setup/step/teardown, accept-line, editing mode, save/restore state | everything | **B** (core: keep changes small, announce them) |
| `tty.rs` | prep/deprep terminal, tty special chars | signals, display | **B** |
| `signals.rs` | signal handling | signals | **B** |
| `callback.rs` | callback interface (coroutine) | callback | **B** |
| `buffer.rs` | the line buffer primitives (insert/delete with undo) | emacs | **E** |
| `undo.rs` | undo list | undo | **E** |
| `kill.rs` | kill ring, kill/yank commands | kill | **E** |
| `numarg.rs` | numeric arguments | emacs (numarg) | **E** |
| `kbdmacro.rs` | keyboard macros, macro input stack | emacs (macro) | **E** |
| `commands/movement.rs` `commands/text.rs` `commands/misc.rs` | emacs commands | emacs | **E** |
| `vi.rs` | vi mode | vi | **E** (or a separate **V** stream) |
| `display.rs` | redisplay, prompts, messages, clear-screen | display, utf8 | **D** |
| `terminal.rs` | terminfo, screen size, output buffer, bell | display | **D** |
| `complete/mod.rs` (split into submodules freely) | completion, generators, match display, colors | complete | **D** |

## 3. Global state

Readline's API *is* global mutable state, and C code reads/writes the
exported variables directly, so:

* **Exported variables are the source of truth.**  Each is
  `#[no_mangle] pub static NAME: CVar<T> = CVar::new(default);` where
  `CVar<T>` (`cvar.rs`) is `#[repr(transparent)]` over `UnsafeCell<T>`, i.e.
  exactly the C object `T NAME;`.  Read with `.get()`, write with `.set()`,
  every time — never cache across a call that can run application code
  (hooks, bound functions, `rl_getc_function`, completion generators,
  `rl_redisplay_function`): the application may change `rl_point`,
  `rl_line_buffer`, `rl_end`, ... from there.
* Defaults match the reference (probed with `get VAR` before and after
  `rl_initialize`; see the generator defaults in tools/gen_stubs.py).
  Function-pointer variables are `CVar<Option<fn_t>>` (nullable fn pointer
  layout).  Defaults like `rl_getc_function = rl_getc`,
  `rl_redisplay_function = rl_redisplay` are set statically.
* **Private state** is `static NAME: CVar<T>` in the owning module (not
  exported).  Use `.get_mut()` for non-`Copy` data, keep the borrow short
  and never hold it across calls into application code or into another
  module that might touch the same static.
* Keymaps: the five exported arrays are `CVar<[KEYMAP_ENTRY; 257]>`; the
  entry's `function` field is a `union KmData { func, map, macro_, raw }`
  (same layout as the C pointer) so static tables can reference other
  keymaps.  User keymaps are malloc'd arrays.

## 4. Unsafe, FFI and errors

* Exported functions are `pub unsafe extern "C" fn` with the exact C
  signature (`types.rs` has every typedef; `T *` function pointers are
  `Option<T>`).  The C-facing shell converts arguments (`cstr(p)` →
  `&[u8]`) and calls safe-ish Rust helpers that work on byte slices.
* NULL arguments: accept them where the reference does (check with a
  probe); never dereference blindly.
* No panics across the boundary (`panic = "abort"`).  Prefer saturating
  arithmetic and clamping (`buffer::point()` clamps to `rl_end`).
* Errors are reported the way the reference does: return codes, `rl_ding()`,
  messages on stderr (`inputrc::report_error` adds `FILE: line N:` while an
  init file is being read).

## 5. Memory ownership

* Anything handed to C that C may free comes from libc malloc:
  `util::c_strdup`, `xmalloc`, `xrealloc`, `c_strvec`; free C memory with
  `util::cfree`.  Rust `Vec`/`String` are fine for private data.
* We free what the API says the library owns (macro text pushed with
  `rl_push_macro_input`, generator results, ...).  `./rlt --impl rust-asan`
  (driver under ASan) must stay clean — run it for your areas.
* Strings returned as "borrowed" (e.g. `rl_get_keymap_name`,
  `rl_variable_value`, `rl_untranslate_keyseq`, `rl_get_termcap`) live in
  module-owned storage.

## 6. Text: bytes and characters

* The line buffer and all strings are **bytes** (`&[u8]` / `Vec<u8>`), never
  Rust `String`.
* Character boundaries and widths follow the C locale like the reference:
  `mb.rs` wraps `mbrtowc`/`wcwidth` (`decode_at`, `next_char`, `prev_char`,
  `forward_chars`, `backward_chars`, `wc_width`, `is_word_char`).  When
  `MB_CUR_MAX == 1` or `byte-oriented` is on, a byte is a character.
  Invalid sequences count as single bytes.
* Word characters: ASCII alphanumerics and multibyte characters for which
  `iswalnum` holds; single bytes >= 0x80 are *not* word characters (the
  common, glibc expectation — also on macOS).

## 7. Terminal and output

* `terminal.rs` loads terminfo with the `terminfo` crate (`$TERM`, or
  `rl_terminal_name`); `cap("el")`, `cap_param("cuf", n)`; padding is
  stripped.  `rl_get_termcap` maps termcap names to terminfo names.
* All output goes through `terminal::out*()` into a buffer and is written to
  `rl_outstream` (a `FILE *`, via `fwrite`) by `terminal::flush()`.  Never
  write to a file descriptor directly; flush before calling application code
  that may print.
* Screen size: `TIOCGWINSZ`, `COLUMNS`/`LINES` (`rl_prefer_env_winsize`),
  terminfo, 80x24.  `usable_width()` is one less than the columns unless the
  terminal has `am` and `xenl`.

## 8. Input, dispatch and the callback interface

* All keys are read with `input::rl_read_key()`.  It returns pending input
  (`rl_execute_next`), macro text, stuffed characters, else calls
  `rl_getc_function(rl_instream)` (after the event hook).  Never call
  `rl_getc_function` yourself.
* Commands are written in **blocking style**: a command that needs more
  keys (quoted-insert, isearch, vi motions, the completion pager...) just
  calls `rl_read_key()` in a loop and sets the appropriate `RL_STATE_*`
  flags (`input::set_state` / `unset_state` / `is_state`) while doing so.
* The callback interface runs the same loop (`readline::line_setup`,
  `line_step`, `line_teardown`) on a coroutine (`callback.rs`, ucontext in
  c/shim.c, same thread, 8 MiB malloc'd stack).  `rl_read_key` suspends the
  coroutine when no input is available, so `rl_callback_read_char` returns
  to the application with all state (flags, partial line, prefix keymap)
  preserved.  The handler runs on the application's stack.  GNU's callback
  mode exposes slightly different state between calls (e.g. it returns from
  quoted-insert with `last_func = quoted-insert` and no MOREINPUT flag);
  emulate such details in `callback.rs` when suspending (save/clear flags,
  restore on resume), not in the commands.
* `dispatch::dispatch_key(key, map)` resolves one key: functions are called
  via `dispatch::call_command` (sets `rl_executing_key/keymap`,
  `RL_STATE_DISPATCHING`, `rl_last_func`), prefix keymaps read the next key
  (with `keyseq-timeout` when the prefix has an ANYOTHERKEY binding), unbound
  sequences ding and are discarded, macros are pushed as input.
* A bindable command is `#[no_mangle] pub unsafe extern "C" fn rl_xxx(count:
  c_int, key: c_int) -> c_int`; the count already includes the numeric
  argument and its sign (`numarg::count()`), `rl_explicit_arg` tells whether
  one was given.  Return 0 on success; ding on failure like the reference.
* After every command the key loop calls `readline::after_command()`
  (kill/yank sequencing, active region).  Add your module's hook there.
* `funmap_table.rs` and `keymap/defaults.rs` are generated by
  `impl/tools/gen_tables.py`, which finds each function by grepping
  `impl/src` for its definition: if you move a bindable function to another
  module, re-run `python3 impl/tools/gen_tables.py`.

## 9. Display model

`display.rs` keeps a model of the screen (`SCREEN.rows`: cells with their
output bytes and widths, rows relative to the first row of the prompt's
last line, and the physical cursor).  `rl_redisplay` lays out the prompt's
last line (invisible `\001..\002` sequences attach to the next cell) and the
line (control characters as `^X`, tabs expanded, wide characters via
`wcwidth`, wrapping at `usable_width()`), rewrites the differing tail of each
changed row and moves the cursor to point.  Lines of a multi-line prompt
before its last line are printed once (`print_prompt_prefix`) when a line
starts.  `rl_on_new_line` resets the model (cursor assumed at column 0 of an
empty line).  Only the final terminal state is compared, so the byte
stream is free-form.  Missing: horizontal-scroll-mode, dumb terminals,
lines taller than the screen, resize re-layout, mark-modified-lines,
active-region highlighting, paren blinking, `rl_message` with newlines.

## 10. Probing the reference

Write a case file outside the repo and run it against the reference:

```sh
cat > /tmp/probe.case <<'EOF'
=== x
keys: abc\C-a<dump>\r
--- script
readline "> "
EOF
./rlt /tmp/probe.case --show --impl ref      # reference
./rlt /tmp/probe.case --show --impl rust     # ours
```

(Headers like `keys:` must come before the first section.)  Useful script
commands: `get VAR`, `dumpfull`, `keymap_dump MAP`, `capture rl_xxx`.
`../lineread-ref-bin/<platform>/rl-driver --bindable` lists C name -> funmap
name.  New behaviours worth pinning down become real test cases (record with
`--record`, see docs/TESTING.md).

## 11. Stubs and workflow

* Unimplemented functions call `todo_stub!("name")` and return a neutral
  value (`grep -rn 'todo_stub!' impl/src`).  `LINEREAD_TRACE_STUBS=1` logs
  every stub call to stderr — handy with `--show`.
* Work per area: `./rlt --impl rust tests/AREA`, `-t TAG`, `FILE::CASE
  --show`.  Before committing run your areas and the whole suite and diff:
  `./rlt --impl rust --json > /tmp/after.json; python3 impl/tools/compare.py
  /tmp/before.json /tmp/after.json` — no regressions outside your area.
  Check Linux (`docker/run.sh ./rlt --impl rust tests/AREA`) and
  `--impl rust-asan` regularly.
* Commit often with explicit pathspecs (only files you own); rebase on main
  before committing (`git pull --rebase`); keep commits small to make
  conflicts trivial.

## 12. Workstreams

| stream | owns (files) | target areas (cases) |
|---|---|---|
| **H** history | `history/*`, `tilde.rs`, `histcmds.rs`, `search.rs` | history 234, histexpand 225, tilde 41, search 182; history-related cases in emacs/kill (yank-nth-arg, yank-last-arg, operate-and-get-next) |
| **B** bindings & input | `keymap/*`, `funmap*.rs`, `bind.rs`, `inputrc.rs`, `vars.rs`, `input.rs`, `dispatch.rs`, `readline.rs`, `tty.rs`, `signals.rs`, `callback.rs` | bind 198, inputrc 246, input 39, hooks 85, signals 68, callback 60, format 17 |
| **E** editing | `buffer.rs`, `undo.rs`, `kill.rs`, `numarg.rs`, `kbdmacro.rs`, `commands/*`, `vi.rs` | emacs 539, kill 91, undo 46, vi 437 |
| **D** display & completion | `display.rs`, `terminal.rs`, `complete/*` | display 229, utf8 85, complete 381 |

With a fifth agent, split **V** (vi.rs, area vi) off **E**.  Shared
helpers that several streams need (e.g. `mb.rs`) are extended additively;
announce changes to `readline.rs`, `dispatch.rs`, `buffer.rs`, `display.rs`
signatures to the other streams.
