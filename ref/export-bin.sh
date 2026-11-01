#!/bin/sh
# Export the compiled reference (no source) for the clean-room phase.
#
#   ref/export-bin.sh            # host platform
#   docker/run.sh ref/export-bin.sh   # Linux
#
# Creates $LINEREAD_REF_BIN/<platform>/ (default ../lineread-ref-bin):
#   lib/plain/libreadline.a libhistory.a   GNU Readline 8.3, -O1, debug info stripped
#   lib/asan/libreadline.a  libhistory.a   the same with ASan+UBSan
#   rl-driver, rl-driver-asan              prebuilt test drivers (black-box oracle)
#   MANIFEST                               provenance and checksums
#
# The Makefile links IMPL=ref / IMPL=asan against these libraries when the
# reference build directories ($LINEREAD_REF/build-*) are absent, so the
# driver can keep evolving after the GNU source is deleted.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
REF_DIR=${LINEREAD_REF:-$REPO/../lineread-ref}
BIN_DIR=${LINEREAD_REF_BIN:-$REPO/../lineread-ref-bin}
PLAT="$(uname -s | tr A-Z a-z)-$(uname -m)"
case "$(uname -s)" in Darwin) SUF="";; *) SUF="-$PLAT";; esac
mkdir -p "$BIN_DIR"
BIN_DIR=$(cd "$BIN_DIR" && pwd)
OUT="$BIN_DIR/$PLAT"

case "$BIN_DIR" in
  "$REPO"|"$REPO"/*) echo "refusing to put reference binaries inside the repo" >&2; exit 1;;
esac

strip_lib() {  # strip debug info (keeps the symbol table needed for linking)
  if [ "$(uname -s)" = Darwin ]; then strip -S "$1" 2>/dev/null || true
  else strip --strip-debug "$1"; fi
}

mkdir -p "$OUT/lib/plain" "$OUT/lib/asan"
for kind in plain asan; do
  src="$REF_DIR/build-$kind$SUF"
  for l in libreadline.a libhistory.a; do
    if [ ! -f "$src/$l" ]; then echo "missing $src/$l (run ref/build.sh)" >&2; exit 1; fi
    cp "$src/$l" "$OUT/lib/$kind/$l"
    strip_lib "$OUT/lib/$kind/$l"
  done
done

# prebuilt drivers linked against the exported libraries
rm -f "$REPO/build/$PLAT/ref/rl-driver" "$REPO/build/$PLAT/asan/rl-driver"
make -s -C "$REPO" IMPL=ref REF_FROM_BIN=1 LINEREAD_REF_BIN="$BIN_DIR"
make -s -C "$REPO" IMPL=asan REF_FROM_BIN=1 LINEREAD_REF_BIN="$BIN_DIR"
cp "$REPO/build/$PLAT/ref/rl-driver" "$OUT/rl-driver"
cp "$REPO/build/$PLAT/asan/rl-driver" "$OUT/rl-driver-asan"

{
  echo "GNU Readline 8.3 reference, compiled ($PLAT)"
  echo "exported: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "lineread commit: $(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)"
  echo "readline source: $(git -C "$REF_DIR/src" describe --tags 2>/dev/null || echo readline-8.3)"
  echo
  (cd "$OUT" && find . -type f ! -name MANIFEST | sort | while read -r f; do
     if command -v sha256sum >/dev/null; then sha256sum "$f"; else shasum -a 256 "$f"; fi
   done)
} > "$OUT/MANIFEST"
echo "exported to $OUT"
ls -l "$OUT" "$OUT/lib/plain" "$OUT/lib/asan"
