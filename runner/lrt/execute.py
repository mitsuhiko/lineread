"""Run one case: spawn the driver on a pty, feed keys, emulate the screen."""

from __future__ import annotations

import errno
import fcntl
import json
import os
import pty
import select
import shutil
import signal
import struct
import sys
import tempfile
import termios
import time

import pyte

from . import keys as keymod
from .casefile import Case

PLATFORM = "darwin" if sys.platform == "darwin" else "linux"
DEFAULT_LOCALE = "en_US.UTF-8" if PLATFORM == "darwin" else "C.UTF-8"
DECCKM = 1 << 5  # pyte stores private mode N as N << 5


class Trace:
    def __init__(self, enabled: bool):
        self.enabled = enabled
        self.t0 = time.monotonic()

    def __call__(self, msg: str) -> None:
        if self.enabled:
            sys.stderr.write(f"[{time.monotonic() - self.t0:7.3f}] {msg}\n")


def _set_termios(fd: int, cols: int, rows: int) -> None:
    attrs = termios.tcgetattr(fd)
    iflag = termios.ICRNL | termios.IXON
    if hasattr(termios, "IUTF8"):
        iflag |= termios.IUTF8
    oflag = termios.OPOST | termios.ONLCR
    cflag = termios.CREAD | termios.CS8 | termios.HUPCL
    lflag = (termios.ISIG | termios.ICANON | termios.ECHO | termios.ECHOE |
             termios.ECHOK | termios.ECHOCTL | termios.ECHOKE | termios.IEXTEN)
    try:
        vdis = os.fpathconf(fd, "PC_VDISABLE")
    except (ValueError, OSError):
        vdis = 0xFF if PLATFORM == "darwin" else 0
    cc = list(attrs[6])
    cc = [vdis if not isinstance(x, bytes) else bytes([vdis]) for x in cc]

    def setcc(name, val):
        idx = getattr(termios, name, None)
        if idx is not None and idx < len(cc):
            cc[idx] = bytes([val]) if isinstance(attrs[6][idx], bytes) else val

    for name, val in [("VINTR", 0x03), ("VQUIT", 0x1C), ("VERASE", 0x7F),
                      ("VKILL", 0x15), ("VEOF", 0x04), ("VSTART", 0x11),
                      ("VSTOP", 0x13), ("VSUSP", 0x1A), ("VREPRINT", 0x12),
                      ("VWERASE", 0x17), ("VLNEXT", 0x16)]:
        setcc(name, val)
    # VMIN/VTIME may alias VEOF/VEOL on some systems; ICANON is on, so
    # they're irrelevant until readline sets its own.
    speed = termios.B38400
    termios.tcsetattr(fd, termios.TCSANOW,
                      [iflag, oflag, cflag, lflag, speed, speed, cc])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def _normalize(obj, repl):
    if isinstance(obj, str):
        for a, b in repl:
            obj = obj.replace(a, b)
        return obj
    if isinstance(obj, list):
        return [_normalize(x, repl) for x in obj]
    if isinstance(obj, dict):
        return {k: _normalize(v, repl) for k, v in obj.items()}
    return obj


def _screen_state(screen: pyte.Screen) -> dict:
    rows = [line.rstrip() for line in screen.display]
    while rows and not rows[-1]:
        rows.pop()
    return {"screen": rows, "cursor": [screen.cursor.y, screen.cursor.x]}


def build_env(case: Case, home: str, base: str, impl: str, extra: dict) -> dict:
    cols, rows = case.screen
    env = {
        "HOME": home,
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "TERM": "xterm-256color",
        "LANG": DEFAULT_LOCALE,
        "LC_ALL": DEFAULT_LOCALE,
        "INPUTRC": os.path.join(base, "inputrc"),
        "COLUMNS": str(cols),
        "LINES": str(rows),
        "TZ": "UTC",
        "USER": "tester",
        "LOGNAME": "tester",
        "SHELL": "/bin/sh",
        "TMPDIR": base,
    }
    env.update(extra)
    for k, v in case.env.items():
        if v is None:
            env.pop(k, None)
        else:
            env[k] = v.replace("$HOME", home).replace("$BASE", base)
    return env


def _write_fixtures(case: Case, base: str, home: str) -> None:
    with open(os.path.join(base, "inputrc"), "w", encoding="utf-8") as f:
        f.write(case.inputrc or "")
    with open(os.path.join(base, "script"), "w", encoding="utf-8") as f:
        f.write(case.script)
    for spec in case.files:
        p = os.path.join(home, spec.path)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        if spec.kind == "dir":
            os.makedirs(p, exist_ok=True)
        elif spec.kind == "symlink":
            os.symlink(spec.content, p)
        else:
            with open(p, "w", encoding="utf-8", newline="") as f:
                f.write(spec.content)
        if spec.mode is not None:
            os.chmod(p, spec.mode)


class Session:
    """One driver process on a pty plus the runner side of the protocol."""

    def __init__(self, case: Case, driver: str, env: dict, base: str, trace: Trace):
        self.case = case
        self.trace = trace
        self.cols, self.rows = case.screen
        self.screen = pyte.Screen(self.cols, self.rows)
        self.stream = pyte.ByteStream(self.screen)
        self.snaps: list[dict] = []
        self.consumed = -1  # bytes consumed as of the latest idle message
        self.sent = 0
        self.acks: set[str] = set()
        self.sigs: list[int] = []
        self.sync_buf = b""
        self.nonce = 0
        self.exit_status = None
        self.raw = bytearray()
        self.deadline = time.monotonic() + case.timeout
        self.timed_out = False

        sync_r, sync_w = os.pipe()
        ctl_r, ctl_w = os.pipe()
        pid, master = pty.fork()
        if pid == 0:  # child
            try:
                _set_termios(0, self.cols, self.rows)
                os.dup2(sync_w, 3)
                os.dup2(ctl_r, 4)
                os.closerange(5, 256)
                os.chdir(env["HOME"])
                env2 = dict(env, RLD_SYNC_FD="3", RLD_CTL_FD="4")
                os.execve(driver, [driver, os.path.join(base, "script")], env2)
            except BaseException as e:  # noqa: BLE001
                os.write(2, f"exec failed: {e}\n".encode())
            os._exit(127)
        os.close(sync_w)
        os.close(ctl_r)
        self.pid = pid
        self.master = master
        self.sync_r = sync_r
        self.ctl_w = ctl_w
        for fd in (master, sync_r):
            fl = fcntl.fcntl(fd, fcntl.F_GETFL)
            fcntl.fcntl(fd, fcntl.F_SETFL, fl | os.O_NONBLOCK)
        self.master_open = True
        self.sync_open = True

    # -- low level -------------------------------------------------------
    def _read_master(self) -> bool:
        try:
            data = os.read(self.master, 65536)
        except OSError as e:
            if e.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                return False
            self.master_open = False  # EIO: slave closed
            return False
        if not data:
            self.master_open = False
            return False
        self.raw += data
        self.stream.feed(data)
        return True

    def drain(self) -> None:
        while self.master_open and self._read_master():
            pass

    def _handle_sync_line(self, line: str) -> None:
        self.trace(f"sync <- {line}")
        parts = line.split()
        if not parts:
            return
        if parts[0] == "I":
            self.consumed = max(self.consumed, int(parts[1]))
        elif parts[0] == "A":
            self.acks.add(parts[1])
        elif parts[0] == "G":
            self.sigs.append(int(parts[1]))
        elif parts[0] == "P":
            self.drain()
            self.snaps.append(_screen_state(self.screen))
            self._ctl(f"{parts[1]} ack")

    def _read_sync(self) -> None:
        try:
            data = os.read(self.sync_r, 65536)
        except OSError as e:
            if e.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
                return
            data = b""
        if not data:
            self.sync_open = False
            return
        self.sync_buf += data
        while b"\n" in self.sync_buf:
            line, self.sync_buf = self.sync_buf.split(b"\n", 1)
            self._handle_sync_line(line.decode("ascii", "replace"))

    def _ctl(self, msg: str) -> None:
        self.trace(f"ctl -> {msg}")
        try:
            os.write(self.ctl_w, (msg + "\n").encode())
        except OSError:
            pass

    def _check_exit(self) -> bool:
        if self.exit_status is not None:
            return True
        pid, status = os.waitpid(self.pid, os.WNOHANG)
        if pid == 0:
            return False
        self.exit_status = status
        return True

    def poll(self, timeout: float) -> None:
        """Wait up to TIMEOUT for activity, process everything available."""
        fds = []
        if self.master_open:
            fds.append(self.master)
        if self.sync_open:
            fds.append(self.sync_r)
        if not fds:
            time.sleep(min(timeout, 0.01))
            return
        try:
            r, _, _ = select.select(fds, [], [], timeout)
        except InterruptedError:
            return
        if self.master in r:
            self._read_master()
        if self.sync_r in r:
            self._read_sync()

    def wait_until(self, cond, limit: float | None = None) -> bool:
        """Process events until COND() is true, the driver exits, or time runs
        out.  Returns COND()."""
        end = self.deadline if limit is None else min(self.deadline, time.monotonic() + limit)
        while True:
            if cond():
                return True
            if self._check_exit() and not self.sync_open:
                return cond()
            now = time.monotonic()
            if now >= end:
                if now >= self.deadline:
                    self.timed_out = True
                return cond()
            self.poll(min(0.05, end - now))

    def exited(self) -> bool:
        return self._check_exit()

    # -- actions -------------------------------------------------------------
    def wait_idle(self) -> bool:
        return self.wait_until(lambda: self.consumed >= self.sent)

    def send(self, data: bytes, counts: bool = True) -> None:
        self.trace(f"send {data!r}")
        if counts:
            self.sent += len(data)
        view = memoryview(data)
        while view:
            if self.exited() and not self.master_open:
                return
            try:
                n = os.write(self.master, view)
                view = view[n:]
            except BlockingIOError:
                self.poll(0.01)
            except OSError:
                return
            self.drain()

    def ctl_and_wait(self, cmd: str) -> None:
        self.nonce += 1
        tag = f"n{self.nonce}"
        self._ctl(f"{tag} {cmd}")
        self.wait_until(lambda: tag in self.acks)

    def resize(self, cols: int, rows: int) -> None:
        nsig = len(self.sigs)
        self.cols, self.rows = cols, rows
        self.screen.resize(rows, cols)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.wait_until(lambda: len(self.sigs) > nsig, limit=2.0)

    def signal_byte(self, b: bytes) -> None:
        nsig = len(self.sigs)
        self.send(b, counts=False)
        self.wait_until(lambda: len(self.sigs) > nsig or self.exited(), limit=2.0)

    def finish(self) -> None:
        try:
            if self.exit_status is None:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass
        for fd in (self.master, self.sync_r, self.ctl_w):
            try:
                os.close(fd)
            except OSError:
                pass


def run_case(case: Case, driver: str, impl: str, trace_enabled: bool = False,
             keep: bool = False, extra_env: dict | None = None) -> dict:
    """Run CASE with DRIVER; returns the actual result dict."""
    trace = Trace(trace_enabled)
    base = os.path.realpath(tempfile.mkdtemp(prefix="lrt-"))
    home = os.path.join(base, "home")
    os.makedirs(home)
    result: dict = {}
    try:
        _write_fixtures(case, base, home)
        extra = dict(extra_env or {})
        extra["RLD_RESULTS"] = os.path.join(base, "results.jsonl")
        if impl in ("asan", "rust-asan"):
            extra["ASAN_OPTIONS"] = f"log_path={base}/asan:detect_leaks=0:abort_on_error=0"
            extra["UBSAN_OPTIONS"] = f"log_path={base}/ubsan:print_stacktrace=1"
        env = build_env(case, home, base, impl, extra)
        actions = keymod.parse_keys(case.keys, signals=case.signals)
        sess = Session(case, driver, env, base, trace)
        try:
            for act in actions:
                if sess.timed_out:
                    break
                sess.wait_idle()
                if sess.exited() and not sess.sync_open:
                    break
                sess.drain()
                kind = act[0]
                if kind == "send":
                    app = DECCKM in sess.screen.mode
                    sess.send(keymod.resolve(act[1], app))
                elif kind == "sig":
                    sess.signal_byte(act[1])
                elif kind == "snap":
                    sess.snaps.append(_screen_state(sess.screen))
                elif kind in ("dump", "dumpfull"):
                    sess.ctl_and_wait(kind)
                elif kind == "resize":
                    sess.resize(act[1], act[2])
                elif kind == "sleep":
                    time.sleep(act[1] / 1000.0)
                elif kind == "pause":
                    pass
            if not sess.timed_out:
                sess.wait_idle()
                if case.eof:
                    sess.drain()
                    sess.ctl_and_wait("eof")
            sess.wait_until(lambda: sess.exited() and not sess.sync_open)
            # collect the remaining output
            end = time.monotonic() + 1.0
            while sess.master_open and time.monotonic() < end:
                sess.poll(0.02)
                if not sess.master_open:
                    break
                if sess.exited():
                    sess.drain()
                    break
        finally:
            sess.finish()

        status = sess.exit_status
        if sess.timed_out or status is None:
            result["exit"] = "timeout"
        elif os.WIFSIGNALED(status):
            result["exit"] = "signal:" + signal.Signals(os.WTERMSIG(status)).name
        else:
            result["exit"] = os.WEXITSTATUS(status)

        records = []
        rpath = os.path.join(base, "results.jsonl")
        if os.path.exists(rpath):
            with open(rpath, encoding="utf-8", errors="surrogateescape") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        records.append(json.loads(line))
                    except json.JSONDecodeError:
                        records.append({"op": "?", "error": "unparsable record", "raw": line})
        result["results"] = records
        result.update(_screen_state(sess.screen))
        if sess.snaps:
            result["snaps"] = sess.snaps
        san = []
        for fn in sorted(os.listdir(base)):
            if fn.startswith("asan.") or fn.startswith("ubsan."):
                with open(os.path.join(base, fn), errors="replace") as f:
                    san.append(f.read())
        if san:
            result["sanitizer"] = "\n".join(san)
        if trace_enabled:
            result["_raw"] = bytes(sess.raw)
        repl = [(home, "$HOME"), (base, "$BASE")]
        if base.startswith("/private/"):
            repl += [(home[len("/private"):], "$HOME"), (base[len("/private"):], "$BASE")]
        result = _normalize(result, repl)
    finally:
        if keep:
            sys.stderr.write(f"kept case directory {base}\n")
        else:
            shutil.rmtree(base, ignore_errors=True)
    return result
