# Real-program smoke tier

Separate from the main score (`./rlt`): real readline consumers built
against our headers (`include/`) and an implementation's library, driven on
a pty.

```sh
tests/apps/build.sh ref            # or asan, rust (downloads sources once)
./rlt apps --impl ref              # compare with tests/apps/expected.<platform>.json
./rlt apps --impl rust --show      # print the final screens
./rlt apps --record --repeat 3     # re-record (reference only)
docker/run.sh sh -c 'tests/apps/build.sh rust && ./rlt apps --impl rust'
```

Programs (built into `build/<platform>/apps/<impl>/`):

| program | source | notes |
|---|---|---|
| CPython `readline` module | `Modules/readline.c` + `Modules/clinic/readline.c.h` of the running `python3`'s exact version, from GitHub | loaded via `PYTHONPATH` in front of the stdlib copy; the REPL runs with `PYTHON_BASIC_REPL=1` (3.13+) so it uses readline; needs Python headers (`python3-dev` on Linux, internal headers for 3.13+) |
| sqlite3 shell | amalgamation 3.46.1, `-DHAVE_READLINE=1` | |
| Lua 5.4.7 REPL | `lua.c` with `-DLUA_USE_READLINE` | |

The scenarios live in `tests/apps/run.py` (`SCENARIOS`): each step sends
keys (runner key notation) and waits until the line under the cursor matches
a regular expression and the program has been quiet for 250 ms. The final
screen and exit status are compared. Expectations are per platform because
the programs differ (Python 3.14 on the macOS host, 3.11 on Debian).

Linking: the library must contain the history functions too (one archive,
like GNU's `libreadline.a`), plus `-lncurses`. The Python module is a shared
object, so the library must be position independent (Rust staticlibs are;
the reference libraries are built with `-fPIC`).

Limitations: under ASan the Python module cannot be loaded on macOS (the
framework python re-executes itself and drops `DYLD_INSERT_LIBRARIES`), so
those scenarios are skipped there; on Linux the ASan runtime is preloaded.
