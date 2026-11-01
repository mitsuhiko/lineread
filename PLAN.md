# lineread: clean-room reimplementation of GNU Readline in Rust

Goal: a Rust library exporting the full GNU Readline 8.3 C API/ABI (readline +
history + tilde), developed purely against (a) recreated public headers and
(b) a behavioral test suite recorded from the reference implementation.

Platforms: macOS and Linux. Behavior is what matters; byte-exact terminal output
is not a goal (we may do better than readline where it is clearly a bug).

## Phases

1. **Infrastructure** (subagent `infra`)
   - Reference build of GNU readline `readline-8.3` from savannah git, *outside*
     the repo (`../lineread-ref/`): a plain build and a coverage build
     (clang `-fprofile-instr-generate -fcoverage-mapping`).
   - Recreated headers in `include/readline/` (readline.h, history.h,
     keymaps.h, tilde.h, rltypedefs.h, chardefs.h, rlstdc.h, rlconf.h).
     Same names, prototypes, constants, struct layouts; no copied comments/text.
   - `driver/rl-driver.c`: a test driver compiled against *our* headers that
     only uses the public API. It interprets a small command script (API calls,
     state dumps, `readline()` calls against a pty) and emits JSON.
   - `runner/`: Python runner. Spawns the driver per case on a pty, feeds
     keystrokes, emulates the terminal (pyte) for screen snapshots, compares
     recorded expectations field by field, outputs per-area scores (`--json`).
     `--record` regenerates expectations from the reference.
   - Header conformance tier: generated static asserts (sizeof/offsetof,
     constant values) + exported symbol checks.
   - Linux via Docker (`docker/`), same suite, same expectations.

2. **Test suite authoring** (two subagents in parallel worktrees)
   - `tests-a`: history API, history expansion, history files, tilde,
     keymaps/bind API, inputrc parsing + `$if/$include`, variables, dumpers,
     funmap, completion (generators, hooks, display of matches, menu-complete).
   - `tests-b`: editing commands (emacs), vi mode, kill ring/yank, undo,
     numeric args, keyboard macros, isearch/non-incremental search, history
     navigation, display (prompt invisible chars, wrapping, multiline,
     horizontal-scroll-mode, multibyte/UTF-8), callback interface, signals,
     hooks/re-entrancy, application-facing globals.
   - Every bindable function (`rl_funmap_names`) and every variable gets tests.

3. **Coverage closing + stabilization** (subagent)
   - Line/branch coverage of the reference per source file; target >=85% lines
     outside terminal/signal glue. Write tests for gaps.
   - Each case recorded 3x on macOS and on Linux; flaky cases fixed or removed.
   - Differential fuzzer (random key sequences, emacs/vi) comparing
     implementation vs. reference driver binary.

4. **Clean-room cut**
   - Delete the GNU readline source. Keep only the compiled reference driver
     binary (black-box oracle for fuzzing) and recorded expectations.
   - From here on, nobody reads GNU readline source.

5. **Hill climbing** (subagents) in `impl/` (Rust crate, cdylib + staticlib
   exporting the C ABI; small C shim only where Rust cannot express the ABI,
   e.g. variadic `rl_message`). Uses the `terminfo` crate. Score = test pass
   rate per area; iterate until green on macOS and Linux, then real-program
   smoke tests (CPython readline module, sqlite3 shell, Lua REPL).

## Test design principles

- Tests are data (case files), never C code. One process per case, fresh state.
- Many small cases, each testing one behavior; tagged by area/function.
- Deterministic: pinned `TERM`, `LANG/LC_ALL`, `HOME`, `INPUTRC`, screen size,
  `keyseq-timeout`; no wall-clock timing dependencies.
- Expectations are field-wise (line buffer, point, mark, return value, history,
  kill ring, screen rows, cursor, ...) so partial progress is visible.
- Screen is compared as emulated terminal state, never as raw bytes.
- Memory ownership checked: the driver frees what the API says the caller owns;
  ASan runs are part of the suite.
