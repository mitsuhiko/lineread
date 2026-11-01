# Opt-in dynamic launcher

From the repository, run `./lineread -- command arg ...`. This builds the
native release cdylib with Cargo (and therefore requires a Rust toolchain,
Cargo dependencies, and a writable build directory) and then replaces the
launcher process with the command. Arguments, standard streams, working
folder, exit status and environment are forwarded; Cargo output goes to
stderr. The launcher does not modify system libraries. Its injected loader
settings are inherited by child processes as well. Use only with trusted
applications and trusted existing loader environment variables.

Linux uses `LD_PRELOAD`, putting liblineread before any existing entries.
It exports both readline and history symbols. Other preloads remain in the
list, but ordering changes which definition wins; mixing implementations
can be unsafe. In secure-execution mode (including typical setuid programs),
the loader restricts or ignores preload settings. Static executables and
statically linked readline cannot be replaced. Symbol visibility, symbol
versions, copy relocations, `RTLD_DEEPBIND`, separate loader namespaces,
internal bindings and explicit `dlopen`/`dlsym` can also defeat substitution.

On macOS the launcher prepends a private directory to `DYLD_LIBRARY_PATH`.
Its symlinks map the common `libreadline` and `libhistory` dylib basenames
(unversioned, versions 4 through 9, and 8.0 through 8.3) to the same Rust
library. Existing `DYLD_LIBRARY_PATH` entries follow ours and existing
`DYLD_INSERT_LIBRARIES` is left intact. This targets normal dynamically
linked dependencies with those install-name basenames, not arbitrary names,
frameworks, or libraries loaded by other mechanisms. A two-level namespace
binds references to a particular library; simply inserting a library is not
a general interposition mechanism. Flat namespaces, explicit lookups, shared
cache behavior, code signing/library validation, and hardened runtime policy
can affect loading. SIP and protected processes may strip or ignore DYLD
variables; for example, system binaries may not be injectable. Static links
cannot be replaced. Both the application and the cdylib must have compatible
architectures and ABI. On either OS, repository paths containing loader list
separators (colon, whitespace) are rejected.

**Launch is not proof of substitution.** Use `./lineread --trace -- command`
to request loader diagnostics on stderr (`LD_DEBUG=bindings` on Linux;
`DYLD_PRINT_LIBRARIES` and `DYLD_PRINT_BINDINGS` on macOS). Inspect actual
bindings and loaded paths, not just the environment or successful startup.
Diagnostics themselves may be suppressed by security policy, and a loaded
library does not prove every reference binds to it. The executable must be
compatible with this evolving implementation; replacing an ABI does not
ensure all behavior is implemented.

For a concrete provider check, `./tests/launcher/run.sh` compiles dynamically
linked test consumers and checks the actual addresses of `readline`,
`add_history`, and `history_length` with `dladdr`, also exercising history,
readline input and argument forwarding. The separate history-only consumer
checks a dependency on libhistory without libreadline. On macOS it expects
Homebrew readline under `/opt/homebrew/opt/readline`; override with
`READLINE_PREFIX`. On Linux it needs readline/history ABI 8 libraries and a C
compiler; `docker/run.sh ./tests/launcher/run.sh` runs it in the project
Linux container. This verifies these test consumers only, not an arbitrary
application.
