#!/bin/sh
# Build the GNU Readline 8.3 reference implementation OUTSIDE the repository.
#
#   ref/build.sh [plain|cov|asan|all]     (default: all)
#
# Layout (REF_DIR defaults to <repo>/../lineread-ref, override with LINEREAD_REF):
#   $REF_DIR/src          git checkout of tag readline-8.3 (or unpacked tarball)
#   $REF_DIR/build-plain  static libs, -O1 -g
#   $REF_DIR/build-cov    static libs with clang source-based coverage
#   $REF_DIR/build-asan   static libs with AddressSanitizer
#   $REF_DIR/gnu-include  <readline/*.h> symlinks to the GNU headers
# Each build dir contains libreadline.a and libhistory.a.
#
# On Linux the build directories get a platform suffix (build-plain-linux-x86_64
# etc.) so a shared checkout can be used from Docker and the host at once.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
REF_DIR=${LINEREAD_REF:-$REPO/../lineread-ref}
mkdir -p "$REF_DIR"
REF_DIR=$(cd "$REF_DIR" && pwd)
SRC="$REF_DIR/src"
TAG=readline-8.3
GIT_URL=https://git.savannah.gnu.org/git/readline.git
TARBALL_URL=https://ftp.gnu.org/gnu/readline/readline-8.3.tar.gz

case "$REF_DIR" in
  "$REPO"|"$REPO"/*) echo "refusing to put reference source inside the repo" >&2; exit 1;;
esac

fetch() {
  if [ -f "$SRC/readline.c" ]; then return; fi
  echo "==> fetching $TAG into $SRC"
  if git clone --depth 1 --branch "$TAG" "$GIT_URL" "$SRC"; then return; fi
  echo "git clone failed, falling back to tarball" >&2
  rm -rf "$SRC"
  tmp="$REF_DIR/readline-8.3.tar.gz"
  curl -fL --retry 3 -o "$tmp" "$TARBALL_URL"
  mkdir -p "$SRC"
  tar -xzf "$tmp" -C "$SRC" --strip-components=1
}

suffix() {
  case "$(uname -s)" in
    Darwin) echo "";;
    *) echo "-$(uname -s | tr A-Z a-z)-$(uname -m)";;
  esac
}

build_one() {
  kind=$1; cflags=$2; ldflags=$3
  dir="$REF_DIR/build-$kind$(suffix)"
  if [ -f "$dir/libreadline.a" ] && [ "${FORCE:-0}" != 1 ]; then
    echo "==> $dir up to date (FORCE=1 to rebuild)"; return
  fi
  echo "==> building $kind in $dir"
  rm -rf "$dir"; mkdir -p "$dir"
  ( cd "$dir" && \
    CC=${CC:-clang} CFLAGS="$cflags" LDFLAGS="$ldflags" \
      "$SRC/configure" --disable-shared --disable-install-examples \
        --with-curses >configure.log 2>&1 && \
    make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" static >make.log 2>&1 ) || {
      echo "build $kind failed, see $dir/*.log" >&2; exit 1; }
  ls -l "$dir/libreadline.a" "$dir/libhistory.a"
}

gnu_headers() {
  # <readline/...> include tree pointing at the GNU headers (used only to
  # record the header conformance data)
  mkdir -p "$REF_DIR/gnu-include/readline"
  for h in readline history keymaps tilde rltypedefs chardefs rlstdc rlconf; do
    ln -sf "../../src/$h.h" "$REF_DIR/gnu-include/readline/$h.h"
  done
}

fetch
gnu_headers
what=${1:-all}
case "$what" in
  plain|all) build_one plain "-g -O1" "";;
esac
case "$what" in
  cov|all) build_one cov "-g -O0 -fprofile-instr-generate -fcoverage-mapping" "-fprofile-instr-generate";;
esac
case "$what" in
  asan|all) build_one asan "-g -O1 -fno-omit-frame-pointer -fsanitize=address,undefined" "-fsanitize=address,undefined";;
esac
