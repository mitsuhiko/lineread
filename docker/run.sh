#!/bin/sh
# Run a command in the Linux test container with the repo, the reference
# checkout (if any) and the exported reference binaries mounted.
#
#   docker/run.sh                     # ./rlt (whole suite)
#   docker/run.sh ./rlt tests/history --repeat 3
#   docker/run.sh ./rlt --record-platform tests/display
#   docker/run.sh sh                  # interactive shell
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
REF_DIR=${LINEREAD_REF:-$REPO/../lineread-ref}
mkdir -p "$REF_DIR"
REF_DIR=$(cd "$REF_DIR" && pwd)
REF_BIN=${LINEREAD_REF_BIN:-$REPO/../lineread-ref-bin}
mkdir -p "$REF_BIN"
REF_BIN=$(cd "$REF_BIN" && pwd)
IMAGE=${LINEREAD_IMAGE:-lineread-test}
cp "$REPO/runner/requirements.txt" "$HERE/requirements.txt"
docker build -q -t "$IMAGE" "$HERE" >/dev/null
rm -f "$HERE/requirements.txt"
TTY=""
[ -t 0 ] && [ -t 1 ] && TTY="-it"
if [ $# -eq 0 ]; then
  set -- ./rlt
fi
# run as the invoking user: root would ignore file permissions in tests
exec docker run --rm $TTY --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$REPO:/work/lineread" -v "$REF_DIR:/work/lineread-ref" \
  -e LINEREAD_REF=/work/lineread-ref \
  -v "$REF_BIN:/work/lineread-ref-bin" -e LINEREAD_REF_BIN=/work/lineread-ref-bin \
  -w /work/lineread "$IMAGE" "$@"
