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
