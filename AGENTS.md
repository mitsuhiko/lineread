# Agent rules for lineread

Read PLAN.md first.

## Clean-room rules

- Never copy GNU Readline source code, comments or documentation text into this
  repository. Test cases contain only inputs and observed outputs.
- Recreated headers in `include/readline/` contain declarations only (names,
  prototypes, constants, layouts), written fresh.
- GNU Readline source lives outside the repo in `../lineread-ref/` and must
  never be committed. During phase 5 (implementation) nobody may read it.

## Working conventions

- Commit early and often with focused messages. Only commit files you own;
  use explicit pathspecs.
- Every test case must pass against the reference implementation before it is
  committed, and be deterministic (run it several times).
- Keep the runner's output machine-readable (`--json`) and human-readable.
