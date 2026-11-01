# Public names without (enough) behavioural cases

`./rlt --audit` (runner/lrt/audit.py) counts, for every bindable function,
inputrc variable and public function/variable/hook declared in
`include/readline/*.h`, how many cases exercise it, and reports anything
below three cases as a gap unless it is listed here. Each entry says why the
name cannot be tested (meaningfully) through the public API. The list items
are parsed by the audit: the backticked name must come first.

- `tilde_find_word` — declared in `tilde.h` but not defined by the GNU
  Readline 8.3 library (a leftover declaration). The driver does not call
  it; an implementation may define it, but nothing can be compared.
- `rl_vi_domove` — only meaningful while readline itself is reading the
  motion of a vi operator (`d`, `c`, `y`): it uses readline's internal
  motion context. Called from an application command at any other time the
  reference dereferences a NULL context; called from a key bound as a motion
  it corrupts the line buffer. The behaviour of vi operators and motions is
  covered by tests/vi/*.

## Known display divergences

- `tests/display/terminals.case::wrap-edit-ansi`: on `TERM=ansi` (`am`
  without `xenl`), the reference explicitly writes `\r\n\r` at each
  full physical row; our renderer relies on auto-wrap while tracking a
  narrower logical row. Probes at widths 10, 20 and 30 show cursor drift
  when moving back up and editing. The same probes with `TERM=xterm`
  (`am` and `xenl`) use implicit wrapping in the reference; `TERM=dumb`
  scrolls horizontally instead. Correcting this needs a consistent model
  of the logical and physical margins, not a case-specific escape sequence.
- `tests/emacs/bindable-extra.case::byte-motion-then-insert-utf8`: inserting
  inside a multibyte character gives the correct line buffer but different
  terminal output. For example, after inserting `Z` in `äb`, the reference
  writes an isolated `\xc3` followed by cursor-right; our renderer inserts
  columns and writes `\xc3Z\xa4`. Three-byte and longer suffix probes show
  the same distinction. The runner feeds bytes into `pyte.ByteStream`, whose
  incremental UTF-8 decoder renders the interrupted `\xc3` as a replacement
  character when it sees the escape sequence. This is a real difference in
  emitted bytes, though the exact invalid-byte appearance is emulator-dependent;
  modelling the reference's malformed-character update generally remains open.

## Known signal-hook divergence

- `tests/complete/interrupt.case::sigint-at-more-pager-signal-event-hook`:
  after SIGINT at the completion pager, the reference calls
  `rl_signal_event_hook` again after the driver's `<dump>` and before
  `readline` returns NULL; our implementation only reports the first call.
  Probes with a pause before or after SIGINT and with a screen snapshot
  before it still showed the second call. With EOF immediately after SIGINT
  and no subsequent dump, the reference showed only the first call. The
  driver's normal `rl_getc_function` wrapper processes control messages and
  invokes the hook when its `pselect` is interrupted; the interaction with
  the interrupted pager and subsequent input remains unresolved. A general
  explanation is needed rather than adding a pager- or test-specific hook call.

## Covered indirectly

These are counted by the audit through the driver, but are worth knowing:

- `rl_getc_function` is owned by the driver: it installs a wrapper that
  synchronises with the runner and then calls the library's `rl_getc`
  (see docs/TESTING.md §5). Every reading case therefore exercises both
  `rl_getc_function` (it must be honoured for all input) and `rl_getc`.
  `option libgetc` cases (tests/input/timeout.case) let `rl_getc` block in
  its own `select` to cover timeouts and EINTR handling.
- `rl_do_lowercase_version` is never called by the reference's dispatcher
  (it re-dispatches the lower-case key instead); the audit counts upper-case
  meta keys (`\M-F`) and explicit bindings as uses.
- `vi-eword`, `vi-bword`, `vi-fword`: bindable names are looked up
  case-insensitively and the first match wins, so binding `vi-eword` binds
  `vi-eWord` etc. (tests/vi/bindable-extra.case::eword-name-binds-eWord).
  The C functions are called directly in `eword-c-function-*`.
