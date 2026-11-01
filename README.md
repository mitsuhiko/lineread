# lineread

A clean-room Rust reimplementation of the GNU Readline 8.3 C interface, accompanied by a behavioral reference test suite. The goal is to make existing C programs that use Readline and the History library work with lineread, while allowing the implementation to improve where exact terminal output is not important.

**Status: experimental.** This is not yet a proven drop-in replacement. The suite exercises editing in Emacs and vi modes, history and history expansion, completion, terminal display, callbacks, signals, key bindings, inputrc, and the public C API. A small number of cases still differ from the reference, including terminal redisplay and signal handling; see [known divergences](docs/UNTESTED.md). A passing test suite is evidence of compatibility, not a guarantee of it.

## Build

You need a Rust toolchain, a C compiler, and the system terminal libraries. The test driver also links against ncurses. On macOS, Xcode command line tools provide the compiler; on Linux, install the development packages for ncurses and a C compiler.

```sh
cargo build --release --manifest-path impl/Cargo.toml
```

This builds a static library and a dynamic library in `impl/target/release/`. The implementation exports the Readline, History, and tilde interfaces from one library. Recreated C declarations are in `include/readline/`. The implementation uses `terminfo` and a small C shim where the C ABI cannot be expressed directly in stable Rust.

## Test

The runner needs Python 3, a C compiler, Rust, and ncurses. On its first run it creates a virtual environment and installs its Python dependency, `pyte`.

```sh
./rlt --impl rust                    # full suite and ABI checks
./rlt --impl rust --score            # concise score
./rlt --impl rust tests/history      # one area
./rlt --impl rust tests/vi --show    # include observed results
./rlt --impl rust --json > score.json
```

Tests launch a fresh driver per case and compare observations—including editing state, return values, and an emulated terminal screen—with expectations recorded from GNU Readline 8.3. Scores give partial credit for matching fields; the overall number includes header and ABI conformance checks, so it is not a percentage of feature parity.

The recorded ABI data covers **macOS arm64 and Linux aarch64**. Other architectures are not yet validated. The Linux container runs the suite in its own environment:

```sh
docker/run.sh ./rlt --impl rust
```

There is also a separate smoke tier which builds and drives CPython's `readline` module, the SQLite shell, and the Lua REPL. It downloads their sources on first build:

```sh
./rlt apps --impl rust --build
```

See [testing](docs/TESTING.md) for case format, filtering, recording, sanitizers, and platform-specific expectations; and [the app test guide](tests/apps/README.md) for smoke-test prerequisites.

## The reference and clean-room boundary

The committed tests and expected results can be used to test lineread without a GNU Readline source tree. The reference binaries used during development are **not included in this repository**. To run `./rlt --impl ref`, record new expectations, or fuzz against the oracle, provide compatible prebuilt reference libraries in `../lineread-ref-bin/<platform>/` (or point `LINEREAD_REF_BIN` at their parent); see [testing](docs/TESTING.md). Reference-source coverage and re-recording ABI facts require a separate reference build and are not available from the recorded data alone.

The project was developed against public documentation, recreated API declarations, and black-box observations of GNU Readline. Implementation contributors must follow the clean-room rules in [AGENTS.md](AGENTS.md) and [the implementation guide](docs/IMPLEMENTING.md): do not consult GNU Readline implementation source or projects that vendor it. The historical reference-build scripts are retained for test infrastructure, but must not be used to obtain source for implementation work.

The differential fuzzer can compare the Rust implementation with a separately supplied reference binary:

```sh
./rlt fuzz --help
./rlt fuzz --a ref --b rust --seed 1 -n 1000
```

It can minimize mismatches into reproducible cases. More detail: [implementation guide](docs/IMPLEMENTING.md) and [test guide](docs/TESTING.md).

## Project layout

| Path | Purpose |
| --- | --- |
| `impl/` | Rust library and C ABI shim |
| `include/readline/` | Recreated public C headers |
| `tests/` | Behavioral cases and recorded expectations |
| `driver/`, `runner/`, `rlt` | C test driver and Python test runner |
| `conformance/` | Recorded ABI facts and conformance checks |
| `docs/ARCHITECTURE.md` | Implementation architecture and module ownership |
| `docs/COVERAGE.md` | Reference coverage measured before the clean-room cut |
| `docs/UNTESTED.md` | Known untested behavior and divergences |

## License

The Rust package metadata declares `MIT OR Apache-2.0`. License text files have not yet been added to this repository.
