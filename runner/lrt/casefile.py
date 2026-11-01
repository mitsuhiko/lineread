"""Parser for *.case files.

A case file holds many test cases.  Syntax (see docs/TESTING.md):

    # comment (only outside sections)
    tags: history                 <- preamble: defaults for every case
    --- inputrc
    set bell-style none

    === case-name
    tags: api
    env: FOO=bar
    --- script
    add_history "x"
    history_dump
    --- keys
    abc\\r

Header lines are ``key: value``.  Sections start with ``--- name [args]`` and
run until the next ``---``/``===`` line.  The preamble (before the first
``===``) provides defaults for every case in the file.
"""

from __future__ import annotations

import dataclasses
import os
import re

HEADER_KEYS = {
    "tags", "screen", "env", "timeout", "ignore", "skip", "platforms",
    "keys", "script", "eof", "signals", "description",
}
SECTION_KEYS = {"inputrc", "script", "keys", "file", "dir", "symlink"}

NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.+-]*$")


class CaseError(Exception):
    pass


@dataclasses.dataclass
class FileSpec:
    path: str
    kind: str = "file"          # file | dir | symlink
    content: str = ""           # file content or symlink target
    mode: int | None = None


@dataclasses.dataclass
class Case:
    file: str                   # path of the .case file, relative to repo
    name: str
    line: int
    tags: list[str]
    script: str = ""
    keys: str = ""
    inputrc: str | None = None
    env: dict[str, str | None] = dataclasses.field(default_factory=dict)
    files: list[FileSpec] = dataclasses.field(default_factory=list)
    screen: tuple[int, int] = (80, 24)   # cols, rows
    timeout: float = 10.0
    ignore: list[str] = dataclasses.field(default_factory=list)
    skip: str | None = None
    platforms: list[str] | None = None
    eof: bool = True
    signals: bool = True
    description: str = ""

    @property
    def id(self) -> str:
        return f"{self.file}::{self.name}"

    @property
    def area(self) -> str:
        parts = self.file.split("/")
        # tests/<area>/... -> area
        if len(parts) >= 3 and parts[0] == "tests":
            return parts[1]
        return parts[0] if len(parts) > 1 else "root"


class _Block:
    """Raw collected data for the preamble or one case."""

    def __init__(self, name: str | None, line: int):
        self.name = name
        self.line = line
        self.headers: list[tuple[str, str, int]] = []
        self.sections: list[tuple[str, list[str], list[str], int]] = []


def _parse_blocks(text: str, path: str) -> list[_Block]:
    blocks = [_Block(None, 1)]
    cur_section = None
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    for i, raw in enumerate(lines, 1):
        line = raw.rstrip("\r")
        if line.startswith("=== "):
            name = line[4:].strip()
            if not NAME_RE.match(name):
                raise CaseError(f"{path}:{i}: bad case name {name!r}")
            blocks.append(_Block(name, i))
            cur_section = None
            continue
        if line.startswith("--- "):
            parts = line[4:].split()
            if not parts or parts[0] not in SECTION_KEYS:
                raise CaseError(f"{path}:{i}: unknown section {line!r}")
            cur_section = (parts[0], parts[1:], [], i)
            blocks[-1].sections.append(cur_section)
            continue
        if cur_section is not None:
            cur_section[2].append(line)
            continue
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = re.match(r"^([a-z_]+):\s?(.*)$", line)
        if not m or m.group(1) not in HEADER_KEYS:
            raise CaseError(f"{path}:{i}: expected 'key: value' header, got {line!r}")
        blocks[-1].headers.append((m.group(1), m.group(2), i))
    return blocks


def _body(lines: list[str]) -> str:
    lines = list(lines)
    while lines and not lines[-1].strip():
        lines.pop()
    return "".join(l + "\n" for l in lines)


def _apply(case: Case, blk: _Block, path: str) -> None:
    for key, value, ln in blk.headers:
        where = f"{path}:{ln}"
        if key == "tags":
            case.tags.extend(value.split())
        elif key == "screen":
            m = re.match(r"^\s*(\d+)x(\d+)\s*$", value)
            if not m:
                raise CaseError(f"{where}: screen must be COLSxROWS")
            case.screen = (int(m.group(1)), int(m.group(2)))
        elif key == "env":
            if "=" in value:
                k, v = value.split("=", 1)
                case.env[k.strip()] = v
            else:
                case.env[value.strip()] = None
        elif key == "timeout":
            case.timeout = float(value)
        elif key == "ignore":
            case.ignore.extend(value.split())
        elif key == "skip":
            case.skip = value.strip() or "skipped"
        elif key == "platforms":
            case.platforms = value.split()
        elif key == "keys":
            case.keys = value
        elif key == "script":
            case.script = value + "\n"
        elif key in ("eof", "signals"):
            v = value.strip().lower()
            if v not in ("on", "off"):
                raise CaseError(f"{where}: {key} must be on|off")
            setattr(case, key, v == "on")
        elif key == "description":
            case.description = value.strip()
    for name, args, lines, ln in blk.sections:
        where = f"{path}:{ln}"
        if name == "inputrc":
            case.inputrc = _body(lines)
        elif name == "script":
            case.script = _body(lines)
        elif name == "keys":
            case.keys = "".join(lines)
        elif name in ("file", "dir", "symlink"):
            if not args:
                raise CaseError(f"{where}: --- {name} needs a path")
            fpath = args[0]
            if fpath.startswith("/") or ".." in fpath.split("/"):
                raise CaseError(f"{where}: fixture paths must be relative")
            spec = FileSpec(path=fpath, kind=name)
            opts = args[1:]
            if name == "file":
                body = _body(lines)
                if "noeol" in opts:
                    body = body[:-1] if body.endswith("\n") else body
                spec.content = body
            elif name == "symlink":
                if len(args) < 2:
                    raise CaseError(f"{where}: --- symlink PATH TARGET")
                spec.content = args[1]
                opts = args[2:]
            for o in opts:
                if o.startswith("mode="):
                    spec.mode = int(o[5:], 8)
                elif o != "noeol":
                    raise CaseError(f"{where}: unknown option {o!r}")
            case.files = [f for f in case.files if f.path != fpath] + [spec]


def parse_case_file(path: str, relpath: str | None = None) -> list[Case]:
    relpath = relpath or path
    with open(path, encoding="utf-8") as f:
        text = f.read()
    blocks = _parse_blocks(text, relpath)
    pre = blocks[0]
    cases = []
    seen = set()
    for blk in blocks[1:]:
        if blk.name in seen:
            raise CaseError(f"{relpath}:{blk.line}: duplicate case name {blk.name}")
        seen.add(blk.name)
        case = Case(file=relpath, name=blk.name, line=blk.line, tags=[])
        _apply(case, pre, relpath)
        _apply(case, blk, relpath)
        if not case.script.strip():
            raise CaseError(f"{relpath}:{blk.line}: case {blk.name} has no script")
        cases.append(case)
    return cases


def find_case_files(paths: list[str], root: str) -> list[str]:
    out = []
    for p in paths:
        full = os.path.join(root, p) if not os.path.isabs(p) else p
        if os.path.isdir(full):
            for dp, dns, fns in os.walk(full):
                dns.sort()
                for fn in sorted(fns):
                    if fn.endswith(".case"):
                        out.append(os.path.join(dp, fn))
        elif os.path.isfile(full):
            out.append(full)
        else:
            raise CaseError(f"no such file or directory: {p}")
    seen = set()
    res = []
    for f in out:
        f = os.path.abspath(f)
        if f not in seen:
            seen.add(f)
            res.append(f)
    return res
