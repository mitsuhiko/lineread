#!/bin/sh
# Build real readline consumers against OUR headers (include/) and an
# implementation's library:
#
#   tests/apps/build.sh [IMPL]        IMPL = ref (default) | asan | rust
#
# Output: build/<platform>/apps/<impl>/
#   python/readline<EXT_SUFFIX>   CPython's readline extension module, built
#                                 from the CPython source of the running
#                                 python3 (use with PYTHONPATH=.../python)
#   sqlite3                       sqlite3 shell (amalgamation, HAVE_READLINE)
#   lua                           Lua 5.4 REPL (LUA_USE_READLINE)
#
# Sources are downloaded once into build/apps-src/ (needs network access).
# This tier is separate from the main score: see tests/apps/README.md.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
IMPL=${1:-ref}
PLAT="$(uname -s | tr A-Z a-z)-$(uname -m)"
OUT="$ROOT/build/$PLAT/apps/$IMPL"
SRC="$ROOT/build/apps-src"
CC=${CC:-clang}
mkdir -p "$OUT/python" "$SRC"

SQLITE_ZIP=sqlite-amalgamation-3460100
SQLITE_URL=https://www.sqlite.org/2024/$SQLITE_ZIP.zip
LUA=lua-5.4.7
LUA_URL=https://www.lua.org/ftp/$LUA.tar.gz

LIBS=$(make -s --no-print-directory -C "$ROOT" IMPL="$IMPL" print-libs)
case "$IMPL" in
  rust|rust-asan) make -s -C "$ROOT" IMPL="$IMPL" rust-lib >/dev/null 2>&1 || true;;
esac
for l in $LIBS; do
  [ -f "$l" ] || { echo "library $l missing (IMPL=$IMPL)" >&2; exit 1; }
done
EXTRA=""
SAN=""
case "$IMPL" in
  asan|rust-asan) SAN="-fsanitize=address,undefined";;
esac
case "$(uname -s)" in
  Linux) EXTRA="-lpthread -ldl -lm";;
esac
TERMLIB=${TERMLIB:--lncurses}
CFLAGS="-O1 -g -I$ROOT/include $SAN"

fetch() {  # URL FILE
  [ -f "$SRC/$2" ] || curl -fsSL --retry 3 -o "$SRC/$2.tmp" "$1" && mv "$SRC/$2.tmp" "$SRC/$2" 2>/dev/null || true
  [ -f "$SRC/$2" ] || { echo "download of $1 failed" >&2; exit 1; }
}

# --- sqlite3 shell ---------------------------------------------------------
fetch "$SQLITE_URL" "$SQLITE_ZIP.zip"
[ -d "$SRC/$SQLITE_ZIP" ] || (cd "$SRC" && unzip -q "$SQLITE_ZIP.zip")
echo "==> sqlite3 ($IMPL)"
$CC $CFLAGS -w -DHAVE_READLINE=1 -DSQLITE_OMIT_LOAD_EXTENSION -DSQLITE_THREADSAFE=0 \
  -o "$OUT/sqlite3" "$SRC/$SQLITE_ZIP/shell.c" "$SRC/$SQLITE_ZIP/sqlite3.c" $LIBS $TERMLIB $EXTRA

# --- Lua REPL --------------------------------------------------------------
fetch "$LUA_URL" "$LUA.tar.gz"
[ -d "$SRC/$LUA" ] || (cd "$SRC" && tar xzf "$LUA.tar.gz")
echo "==> lua ($IMPL)"
case "$(uname -s)" in Darwin) LUAOS=-DLUA_USE_MACOSX;; *) LUAOS=-DLUA_USE_LINUX;; esac
LUASRC=$(ls "$SRC/$LUA/src"/*.c | grep -v -e '/luac.c$')
$CC $CFLAGS -w $LUAOS -DLUA_USE_READLINE -o "$OUT/lua" $LUASRC $LIBS $TERMLIB $EXTRA -lm

# --- CPython readline module -------------------------------------------------
PY=${PYTHON:-python3}
PYVER=$($PY -c 'import sys; print("v%d.%d.%d" % sys.version_info[:3])')
PYINC=$($PY -c 'import sysconfig; print(sysconfig.get_paths()["include"])')
EXT=$($PY -c 'import sysconfig; print(sysconfig.get_config_var("EXT_SUFFIX"))')
PYDIR="$SRC/cpython-$PYVER"
mkdir -p "$PYDIR/clinic"
fetch "https://raw.githubusercontent.com/python/cpython/$PYVER/Modules/readline.c" "cpython-$PYVER/readline.c"
fetch "https://raw.githubusercontent.com/python/cpython/$PYVER/Modules/clinic/readline.c.h" "cpython-$PYVER/clinic/readline.c.h"
echo "==> python readline module $PYVER ($IMPL)"
case "$(uname -s)" in
  Darwin) SHARED="-bundle -undefined dynamic_lookup";;
  *) SHARED="-shared -fPIC";;
esac
# HAVE_RL_* feature macros as CPython's configure would detect them for
# GNU Readline 8.x
PYDEFS="-DHAVE_RL_APPEND_HISTORY -DHAVE_RL_CATCH_SIGNAL -DHAVE_RL_COMPDISP_FUNC_T \
  -DHAVE_RL_COMPLETION_APPEND_CHARACTER -DHAVE_RL_COMPLETION_DISPLAY_MATCHES_HOOK \
  -DHAVE_RL_COMPLETION_MATCHES -DHAVE_RL_COMPLETION_SUPPRESS_APPEND -DHAVE_RL_PRE_INPUT_HOOK \
  -DHAVE_RL_RESIZE_TERMINAL"
$CC $CFLAGS -w -fPIC $PYDEFS -I"$PYINC" -I"$PYINC/internal" -I"$PYDIR" $SHARED \
  -o "$OUT/python/readline$EXT" "$PYDIR/readline.c" $LIBS $TERMLIB $EXTRA
ls -l "$OUT" "$OUT/python"
