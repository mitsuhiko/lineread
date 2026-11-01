#!/bin/sh
# Create the runner virtualenv (per platform, gitignored) and install deps.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
PLAT="$(uname -s | tr A-Z a-z)-$(uname -m)"
VENV="$HERE/.venv-$PLAT"
PY=${PYTHON:-python3}
if [ ! -x "$VENV/bin/python" ]; then
  "$PY" -m venv "$VENV"
fi
"$VENV/bin/python" -m pip install -q --disable-pip-version-check -r "$HERE/requirements.txt"
echo "$VENV"
