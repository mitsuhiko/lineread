# Agent rules for lineread

Read PLAN.md first.

## Clean-room rules

- Never copy GNU Readline source code, comments or documentation text into this
  repository. Test cases contain only inputs and observed outputs.
- Recreated headers in `include/readline/` contain declarations only (names,
  prototypes, constants, layouts), written fresh.
- The clean-room cut has happened: the GNU Readline source is deleted. Never
  download, read or search GNU Readline source (or projects vendoring it), and
  never run `ref/build.sh`. The reference exists only as compiled binaries in
  `../lineread-ref-bin/` (black-box oracle). Allowed semantics sources: the
  readline(3)/history(3) man pages, the GNU Readline/History info manuals,
  `include/readline/`, and the tests. See docs/IMPLEMENTING.md.

## Working conventions

- Commit early and often with focused messages. Only commit files you own;
  use explicit pathspecs.
- Every test case must pass against the reference implementation before it is
  committed, and be deterministic (run it several times).
- Keep the runner's output machine-readable (`--json`) and human-readable.
