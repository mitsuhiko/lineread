"""Real-program smoke tier: drive readline consumers on a pty.

    ./rlt apps [--impl ref|asan|rust] [--build] [--record] [-k GLOB] [--show]

Programs come from tests/apps/build.sh (build/<platform>/apps/<impl>/):
CPython's readline module (the system python3 with PYTHONPATH pointing at
our build of the module), the sqlite3 shell and the Lua REPL.  Scenarios are
defined in SCENARIOS below: a list of steps, each sending keys (runner key
notation, docs/TESTING.md section 4) and waiting until the emulated screen
matches a regular expression and the program has been quiet for a moment.
The final screen and exit status are compared with
tests/apps/expected.<platform>.json (recorded against the reference with
--record).  The tier is separate from the main score: the programs and their
versions differ per platform, and synchronisation is by screen content and
quiescence instead of the driver protocol.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import fcntl
import termios

import pyte

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "runner"))
from lrt import keys as keymod  # noqa: E402
from lrt.execute import PLATFORM, DEFAULT_LOCALE, _set_termios, _display  # noqa: E402

PLAT_DIR = f"{os.uname().sysname.lower()}-{os.uname().machine}"

PY = ["python3", "-q"]
PROMPT_PY = r">>> $"
PROMPT_SQL = r"sqlite> $"
PROMPT_LUA = r"^> $"

# (name, program, [(keys, wait-regex on the cursor line or None), ...])
SCENARIOS = [
    ("py-basic", "python", [
        ("", PROMPT_PY),
        ("x = 40\r", PROMPT_PY),
        ("x + 2\r", PROMPT_PY),
        ("<up><up>\r", PROMPT_PY),
        ("x * 2\\C-a\\C-d40\r", PROMPT_PY),
        ("exit()\r", None),
    ]),
    ("py-emacs-editing", "python", [
        ("", PROMPT_PY),
        ("print('one two three')\\C-b\\C-b\\M-b\\M-d\r", PROMPT_PY),
        ("'bac'.upper()\\C-a\\C-f\\C-f\\C-t\r", PROMPT_PY),
        ("exit()\r", None),
    ]),
    ("py-complete", "python", [
        ("", PROMPT_PY),
        ("import os\r", PROMPT_PY),
        ("os.getcw\t", r"os\.getcwd"),
        ("\\C-e()\r", PROMPT_PY),
        ("\\C-u", None),
        ("exit()\r", None),
    ]),
    ("py-reverse-search", "python", [
        ("", PROMPT_PY),
        ("a = 'hello'\r", PROMPT_PY),
        ("b = 2\r", PROMPT_PY),
        ("\\C-ra = \r", PROMPT_PY),
        ("a\r", PROMPT_PY),
        ("exit()\r", None),
    ]),
    ("py-history-api", "python", [
        ("", PROMPT_PY),
        ("import readline\r", PROMPT_PY),
        ("readline.clear_history(); readline.add_history('zzz = 7')\r", PROMPT_PY),
        ("<up><up>\r", PROMPT_PY),
        ("zzz, readline.get_current_history_length()\r", PROMPT_PY),
        ("exit()\r", None),
    ]),
    ("sqlite-basic", "sqlite3", [
        ("", PROMPT_SQL),
        ("select 1+1;\r", PROMPT_SQL),
        ("<up>\\C-e\\C-?\\C-?2;\r", PROMPT_SQL),
        (".quit\r", None),
    ]),
    ("sqlite-continuation", "sqlite3", [
        ("", PROMPT_SQL),
        ("select\r", r"\.\.\.> $"),
        ("  6 *\r", r"\.\.\.> $"),
        ("  7;\r", PROMPT_SQL),
        (".quit\r", None),
    ]),
    ("sqlite-kill-yank", "sqlite3", [
        ("", PROMPT_SQL),
        ("select 'ab' || 'cd';\\C-a\\C-k\\C-y\r", PROMPT_SQL),
        ("\\C-y\\C-a\\M-f\\M-d\\M-d\\M-d\\M-d 'x';\r", PROMPT_SQL),
        (".quit\r", None),
    ]),
    ("lua-basic", "lua", [
        ("", PROMPT_LUA),
        ("print(1+1)\r", PROMPT_LUA),
        ("<up>\\C-b\\C-b\\C-?3\r", PROMPT_LUA),
        ("os.exit()\r", None),
    ]),
    ("lua-continuation", "lua", [
        ("", PROMPT_LUA),
        ("for i = 1, 2 do\r", r"^>> $"),
        ("print(i * 10) end\r", PROMPT_LUA),
        ("os.exit()\r", None),
    ]),
    ("lua-word-editing", "lua", [
        ("", PROMPT_LUA),
        ("x = 'alpha beta'\\M-b\\M-u\r", PROMPT_LUA),
        ("print(x)\r", PROMPT_LUA),
        ("os.exit()\r", None),
    ]),
]


def program_cmd(prog: str, appdir: str) -> list[str]:
    if prog == "python":
        return PY
    return [os.path.join(appdir, prog)]


def asan_preload() -> dict:
    cc = os.environ.get("CC", "clang")
    if sys.platform == "darwin":
        lib = subprocess.run([cc, "-print-file-name=libclang_rt.asan_osx_dynamic.dylib"],
                             capture_output=True, text=True).stdout.strip()
        return {"DYLD_INSERT_LIBRARIES": lib}
    arch = os.uname().machine
    lib = subprocess.run([cc, f"-print-file-name=libclang_rt.asan-{arch}.so"],
                         capture_output=True, text=True).stdout.strip()
    return {"LD_PRELOAD": lib}


def cursor_line(screen) -> str:
    return _display(screen)[screen.cursor.y][:screen.cursor.x]


def run_scenario(name, prog, steps, appdir, impl, show=False) -> dict:
    cols, rows = 80, 24
    base = tempfile.mkdtemp(prefix="lrta-", dir="/tmp")
    home = os.path.join(base, "home")
    os.makedirs(home)
    with open(os.path.join(base, "inputrc"), "w") as f:
        f.write("")
    env = {
        "HOME": home, "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "TERM": "xterm-256color",
        "LANG": "C", "LC_CTYPE": DEFAULT_LOCALE, "LC_COLLATE": "C", "INPUTRC": os.path.join(base, "inputrc"),
        "COLUMNS": str(cols), "LINES": str(rows), "TZ": "UTC", "USER": "tester", "LOGNAME": "tester",
        "PYTHONPATH": os.path.join(appdir, "python"), "PYTHON_BASIC_REPL": "1", "PYTHONNOUSERSITE": "1",
        "PYTHON_COLORS": "0", "NO_COLOR": "1", "PYTHONDONTWRITEBYTECODE": "1",
    }
    if impl in ("asan", "rust-asan"):
        env["ASAN_OPTIONS"] = f"log_path={base}/asan:detect_leaks=0"
        env["UBSAN_OPTIONS"] = f"log_path={base}/ubsan"
        if prog == "python":
            # the module is dlopen()ed: the ASan runtime must be preloaded
            env.update(asan_preload())
    cmd = program_cmd(prog, appdir)
    pid, master = pty.fork()
    if pid == 0:
        try:
            _set_termios(0, cols, rows)
            os.chdir(home)
            os.execvpe(cmd[0], cmd, env)
        except BaseException as e:  # noqa: BLE001
            os.write(2, f"exec failed: {e}\n".encode())
        os._exit(127)
    fl = fcntl.fcntl(master, fcntl.F_GETFL)
    fcntl.fcntl(master, fcntl.F_SETFL, fl | os.O_NONBLOCK)
    screen = pyte.Screen(cols, rows)
    stream = pyte.ByteStream(screen)
    alive = True
    status = None

    def pump(timeout):
        nonlocal alive
        if not alive:
            time.sleep(timeout)
            return False
        r, _, _ = select.select([master], [], [], timeout)
        if not r:
            return False
        try:
            data = os.read(master, 65536)
        except OSError:
            alive = False
            return False
        if not data:
            alive = False
            return False
        stream.feed(data)
        return True

    def settle(regex, limit=10.0, quiet=0.25):
        end = time.monotonic() + limit
        last = time.monotonic()
        while time.monotonic() < end:
            if pump(0.02):
                last = time.monotonic()
                continue
            ok = regex is None or re.search(regex, cursor_line(screen))
            if ok and time.monotonic() - last >= quiet:
                return True
            if not alive and regex is None:
                return True
        return False

    timeouts = []
    try:
        for i, (keys, wait) in enumerate(steps):
            if keys:
                for act in keymod.parse_keys(keys, signals=False):
                    if act[0] == "send" and alive:
                        app = (1 << 5) in screen.mode
                        try:
                            os.write(master, keymod.resolve(act[1], app))
                        except OSError:
                            alive = False
                    elif act[0] == "pause":
                        settle(None, 2.0, 0.1)
            if not settle(wait):
                timeouts.append(i)
        end = time.monotonic() + 10
        while time.monotonic() < end:
            p, st = os.waitpid(pid, os.WNOHANG)
            if p:
                status = st
                break
            pump(0.05)
        while pump(0.05):
            pass
    finally:
        if status is None:
            try:
                os.kill(pid, signal.SIGKILL)
                _, status = os.waitpid(pid, 0)
            except (ProcessLookupError, ChildProcessError):
                pass
        os.close(master)
    rows_out = [r.rstrip() for r in _display(screen)]
    while rows_out and not rows_out[-1]:
        rows_out.pop()
    repl = [(os.path.realpath(home), "$HOME"), (home, "$HOME"), (os.path.realpath(base), "$BASE"), (base, "$BASE")]
    norm = []
    for r in rows_out:
        for a, b in repl:
            r = r.replace(a, b)
        norm.append(r)
    res = {"screen": norm}
    if status is None:
        res["exit"] = "timeout"
    elif os.WIFSIGNALED(status):
        res["exit"] = "signal:" + signal.Signals(os.WTERMSIG(status)).name
    else:
        res["exit"] = os.WEXITSTATUS(status)
    if timeouts:
        res["step_timeouts"] = timeouts
    san = [open(os.path.join(base, fn), errors="replace").read()
           for fn in sorted(os.listdir(base)) if fn.startswith(("asan.", "ubsan."))]
    if san:
        res["sanitizer"] = "\n".join(san)
    shutil.rmtree(base, ignore_errors=True)
    if show:
        print(f"--- {name} exit={res['exit']}" + (f" step timeouts {timeouts}" if timeouts else ""))
        for r in res["screen"]:
            print(f"   |{r:<{cols}}|")
    return res


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="rlt apps", description=__doc__.split("\n\n")[0])
    ap.add_argument("--impl", default="ref")
    ap.add_argument("--build", action="store_true", help="run tests/apps/build.sh IMPL first")
    ap.add_argument("--record", action="store_true", help="record expectations (reference only)")
    ap.add_argument("-k", "--name", action="append", help="scenario name glob")
    ap.add_argument("--show", action="store_true")
    ap.add_argument("--repeat", type=int, default=1)
    opts = ap.parse_args(argv)
    appdir = os.path.join(ROOT, "build", PLAT_DIR, "apps", opts.impl)
    if opts.build or not os.path.isdir(appdir):
        subprocess.run([os.path.join(ROOT, "tests", "apps", "build.sh"), opts.impl], check=True)
    exp_path = os.path.join(ROOT, "tests", "apps", f"expected.{PLATFORM}.json")
    expected = {}
    if os.path.exists(exp_path):
        with open(exp_path) as f:
            expected = json.load(f)
    ok = bad = 0
    for name, prog, steps in SCENARIOS:
        if opts.name and not any(fnmatch.fnmatchcase(name, g) for g in opts.name):
            continue
        if prog == "python" and opts.impl.endswith("asan") and sys.platform == "darwin":
            # macOS python re-executes itself and drops DYLD_INSERT_LIBRARIES,
            # so an ASan-instrumented extension module cannot be loaded
            print(f"SKIP     {name}: ASan python module not supported on macOS")
            continue
        runs = [run_scenario(name, prog, steps, appdir, opts.impl, opts.show) for _ in range(opts.repeat)]
        res = runs[0]
        if any(r != res for r in runs[1:]):
            print(f"FLAKY    {name}")
            bad += 1
            continue
        if opts.record:
            if "step_timeouts" in res or "sanitizer" in res:
                print(f"ERROR    {name}: not recorded ({res.get('step_timeouts')}, sanitizer={'sanitizer' in res})")
                bad += 1
                continue
            expected[name] = res
            ok += 1
            continue
        exp = expected.get(name)
        if exp is None:
            print(f"ERROR    {name}: no expectation (record with ./rlt apps --record)")
            bad += 1
        elif exp != res:
            print(f"FAIL     {name}")
            for k in sorted(set(exp) | set(res)):
                if exp.get(k) != res.get(k):
                    if k == "screen":
                        for i in range(max(len(exp[k]), len(res.get(k, [])))):
                            e = exp[k][i] if i < len(exp[k]) else "<missing>"
                            a = res[k][i] if i < len(res.get(k, [])) else "<missing>"
                            if e != a:
                                print(f"         screen[{i}]: expected {e!r}\n                    actual   {a!r}")
                    else:
                        print(f"         {k}: expected {str(exp.get(k))[:200]!r} actual {str(res.get(k))[:200]!r}")
            bad += 1
        else:
            ok += 1
    if opts.record:
        with open(exp_path, "w") as f:
            json.dump(expected, f, indent=1, ensure_ascii=False, sort_keys=True)
            f.write("\n")
    print(f"apps: {ok}/{ok + bad} ok  impl={opts.impl} platform={PLATFORM}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
