"""Keystroke notation -> list of runner actions.

Actions:
    ("send", [items])     items are bytes or ("key", name) resolved at send
                          time (cursor keys depend on DECCKM)
    ("sig", bytes)        a byte the tty turns into a signal (^C ^\\ ^Z)
    ("pause",)            chunk boundary (wait until the driver is idle)
    ("snap",)             screen snapshot
    ("dump",)/("dumpfull",)  driver state dump between keys
    ("resize", cols, rows)
    ("sleep", ms)
"""

from __future__ import annotations

import re


class KeyError_(Exception):
    pass


# Keys whose encoding depends on DECCKM: (normal, application)
CURSOR_KEYS = {
    "up": (b"\x1b[A", b"\x1bOA"),
    "down": (b"\x1b[B", b"\x1bOB"),
    "right": (b"\x1b[C", b"\x1bOC"),
    "left": (b"\x1b[D", b"\x1bOD"),
    "home": (b"\x1b[H", b"\x1bOH"),
    "end": (b"\x1b[F", b"\x1bOF"),
}

FIXED_KEYS = {
    "ins": b"\x1b[2~", "insert": b"\x1b[2~",
    "del": b"\x1b[3~", "delete": b"\x1b[3~",
    "pgup": b"\x1b[5~", "pgdn": b"\x1b[6~",
    "f1": b"\x1bOP", "f2": b"\x1bOQ", "f3": b"\x1bOR", "f4": b"\x1bOS",
    "f5": b"\x1b[15~", "f6": b"\x1b[17~", "f7": b"\x1b[18~", "f8": b"\x1b[19~",
    "f9": b"\x1b[20~", "f10": b"\x1b[21~", "f11": b"\x1b[23~", "f12": b"\x1b[24~",
    "btab": b"\x1b[Z",
    "c-up": b"\x1b[1;5A", "c-down": b"\x1b[1;5B",
    "c-right": b"\x1b[1;5C", "c-left": b"\x1b[1;5D",
    "m-up": b"\x1b[1;3A", "m-down": b"\x1b[1;3B",
    "m-right": b"\x1b[1;3C", "m-left": b"\x1b[1;3D",
    "s-up": b"\x1b[1;2A", "s-down": b"\x1b[1;2B",
    "s-right": b"\x1b[1;2C", "s-left": b"\x1b[1;2D",
    "bs": b"\x7f", "tab": b"\t", "ret": b"\r", "nl": b"\n", "esc": b"\x1b",
    "sp": b" ", "space": b" ", "lt": b"<", "gt": b">",
    "paste-start": b"\x1b[200~", "paste-end": b"\x1b[201~",
}

# Bytes the runner's termios turns into signals.
SIGNAL_BYTES = {0x03: "INT", 0x1c: "QUIT", 0x1a: "TSTP"}


def _ctrl(c: int) -> int:
    if c == ord("?"):
        return 0x7F
    return ord(chr(c).upper()) & 0x1F if chr(c).isalpha() else c & 0x1F


def _parse_char(s: str, i: int) -> tuple[bytes, int]:
    """Parse one (possibly escaped) character at s[i]; returns (bytes, next)."""
    if s[i] != "\\":
        return s[i].encode("utf-8"), i + 1
    if i + 1 >= len(s):
        raise KeyError_("trailing backslash")
    c = s[i + 1]
    if s.startswith("\\C-", i):
        b, j = _parse_char(s, i + 3)
        if len(b) != 1:
            raise KeyError_(f"\\C- needs a single byte at {i}")
        return bytes([_ctrl(b[0])]), j
    if s.startswith("\\M-", i):
        b, j = _parse_char(s, i + 3)
        return b"\x1b" + b, j
    simple = {"e": b"\x1b", "t": b"\t", "r": b"\r", "n": b"\n", "a": b"\x07",
              "b": b"\x08", "d": b"\x7f", "f": b"\x0c", "v": b"\x0b", "\\": b"\\",
              "<": b"<", "s": b" ", " ": b" ", "0": None}
    if c == "x":
        m = re.match(r"[0-9a-fA-F]{2}", s[i + 2:])
        if not m:
            raise KeyError_(f"\\x needs two hex digits at {i}")
        return bytes([int(m.group(0), 16)]), i + 4
    if c == "u":
        m = re.match(r"\{([0-9a-fA-F]+)\}", s[i + 2:])
        if not m:
            raise KeyError_(f"\\u needs {{HEX}} at {i}")
        return chr(int(m.group(1), 16)).encode("utf-8"), i + 2 + len(m.group(0))
    if c in "01234567":
        m = re.match(r"[0-7]{1,3}", s[i + 1:])
        return bytes([int(m.group(0), 8) & 0xFF]), i + 1 + len(m.group(0))
    if c in simple and simple[c] is not None:
        return simple[c], i + 2
    raise KeyError_(f"unknown escape \\{c} at {i}")


def parse_keys(s: str, signals: bool = True) -> list[tuple]:
    actions: list[tuple] = []
    cur: list = []

    def flush():
        nonlocal cur
        if cur:
            actions.append(("send", cur))
            cur = []

    def add_bytes(b: bytes):
        for byte in b:
            if signals and byte in SIGNAL_BYTES:
                flush()
                actions.append(("sig", bytes([byte])))
            else:
                if cur and isinstance(cur[-1], bytes):
                    cur[-1] = cur[-1] + bytes([byte])
                else:
                    cur.append(bytes([byte]))

    i = 0
    while i < len(s):
        if s[i] == "<":
            j = s.find(">", i)
            if j < 0:
                raise KeyError_(f"unterminated <...> at {i} (use \\< or <lt> for '<')")
            name = s[i + 1:j]
            lname = name.lower()
            i = j + 1
            if lname == "pause":
                flush()
                actions.append(("pause",))
            elif lname == "snap":
                flush()
                actions.append(("snap",))
            elif lname in ("dump", "dumpfull"):
                flush()
                actions.append((lname,))
            elif lname.startswith("resize:"):
                m = re.match(r"^resize:(\d+)x(\d+)$", lname)
                if not m:
                    raise KeyError_(f"bad <{name}>, want <resize:COLSxROWS>")
                flush()
                actions.append(("resize", int(m.group(1)), int(m.group(2))))
            elif lname.startswith("sleep:"):
                flush()
                actions.append(("sleep", int(lname[6:])))
            elif lname in CURSOR_KEYS:
                cur.append(("key", lname))
            elif lname in FIXED_KEYS:
                add_bytes(FIXED_KEYS[lname])
            else:
                raise KeyError_(f"unknown key <{name}>")
            continue
        b, i = _parse_char(s, i)
        add_bytes(b)
    flush()
    return actions


def resolve(items: list, app_cursor: bool) -> bytes:
    out = b""
    for it in items:
        if isinstance(it, bytes):
            out += it
        else:
            out += CURSOR_KEYS[it[1]][1 if app_cursor else 0]
    return out
