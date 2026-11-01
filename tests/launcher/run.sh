#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
case $(uname -s) in
  Darwin) prefix=${READLINE_PREFIX:-/opt/homebrew/opt/readline}; cc -I"$prefix/include" "$root/tests/launcher/consumer.c" -L"$prefix/lib" -Wl,-rpath,"$prefix/lib" -lreadline -lhistory -o "$tmp/consumer"; cc -I"$prefix/include" "$root/tests/launcher/history.c" -L"$prefix/lib" -lhistory -o "$tmp/history" ;;
  Linux) cc -I"$root/include" "$root/tests/launcher/consumer.c" -Wl,--no-as-needed -l:libreadline.so.8 -l:libhistory.so.8 -ldl -o "$tmp/consumer"; cc -I"$root/include" "$root/tests/launcher/history.c" -l:libhistory.so.8 -ldl -o "$tmp/history" ;;
  *) exit 77 ;;
esac
# Assert an ordinary dynamic dependency exists and the unwrapped program does
# not resolve these addresses to lineread.
if printf 'hello\n' | "$tmp/consumer" 'space arg' ''; then
  echo 'unexpected unwrapped provider' >&2; exit 1
fi
printf 'hello\n' | "$root/lineread" -- "$tmp/consumer" 'space arg' ''
printf 'hello\n' | "$root/lineread" -- "$tmp/consumer" 'space arg' ''
"$root/lineread" -- "$tmp/history"
echo 'launcher provider and forwarding tests passed'
