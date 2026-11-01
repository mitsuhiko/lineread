"""Expectation files and field-wise comparison."""

from __future__ import annotations

import fnmatch
import json
import os
import re

COMPARED_KEYS = ("exit", "results", "screen", "cursor", "snaps", "sanitizer")


def expected_path(case_file: str, platform: str | None = None) -> str:
    stem = case_file[:-len(".case")] if case_file.endswith(".case") else case_file
    if platform:
        return f"{stem}.expected.{platform}.json"
    return f"{stem}.expected.json"


# Implementations that are the GNU reference (plain, coverage, ASan builds).
REF_IMPLS = ("ref", "cov", "asan")


def _load(path: str) -> dict:
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    return {}


def load_expected(case_file_abs: str, platform: str, impl: str | None = None) -> tuple[dict, dict]:
    """Returns (common, overrides) dicts keyed by case name.

    ``platform`` is the override file key: ``darwin``/``linux`` (genuine
    platform differences, apply to every implementation) or
    ``ref-darwin``/``ref-linux`` (reference quirks, see
    ``load_expected(..., impl)``).  With ``impl`` given, the overrides are
    those of ``platform`` plus, for the reference implementations only, the
    ``ref-<platform>`` quirk overrides: libc quirks of the reference that a
    reimplementation must not copy, so it is compared to the common
    expectation instead."""
    common = _load(expected_path(case_file_abs))
    over = dict(_load(expected_path(case_file_abs, platform)))
    if impl is not None and impl in REF_IMPLS and not platform.startswith("ref-"):
        over.update(_load(expected_path(case_file_abs, "ref-" + platform)))
    return common, over


_CTRL_RE = re.compile("[\x7f-\x9f\u2028\u2029]")


def _dump_value(v) -> str:
    s = json.dumps(v, ensure_ascii=False)
    # keep DEL/C1 controls visible
    s = _CTRL_RE.sub(lambda m: "\\u%04x" % ord(m.group(0)), s)
    # lone surrogates (bytes that are not UTF-8) must stay escaped
    return s.encode("utf-8", "backslashreplace").decode("utf-8")


def format_expected(data: dict) -> str:
    """Stable, diff-friendly JSON: one result record / screen row per line."""
    lines = ["{"]
    names = list(data)
    for ci, name in enumerate(names):
        exp = data[name]
        lines.append(f"  {_dump_value(name)}: {{")
        keys = [k for k in COMPARED_KEYS if k in exp] + sorted(k for k in exp if k not in COMPARED_KEYS)
        for ki, k in enumerate(keys):
            v = exp[k]
            comma = "," if ki < len(keys) - 1 else ""
            if k in ("results", "screen") and isinstance(v, list) and v:
                lines.append(f"    {_dump_value(k)}: [")
                for i, item in enumerate(v):
                    lines.append(f"      {_dump_value(item)}" + ("," if i < len(v) - 1 else ""))
                lines.append(f"    ]{comma}")
            elif k == "snaps" and isinstance(v, list) and v:
                lines.append(f"    {_dump_value(k)}: [")
                for i, snap in enumerate(v):
                    lines.append("      {")
                    lines.append(f"        \"screen\": [")
                    rows = snap.get("screen", [])
                    for j, row in enumerate(rows):
                        lines.append(f"          {_dump_value(row)}" + ("," if j < len(rows) - 1 else ""))
                    lines.append("        ],")
                    lines.append(f"        \"cursor\": {_dump_value(snap.get('cursor'))}")
                    lines.append("      }" + ("," if i < len(v) - 1 else ""))
                lines.append(f"    ]{comma}")
            else:
                lines.append(f"    {_dump_value(k)}: {_dump_value(v)}{comma}")
        lines.append("  }" + ("," if ci < len(names) - 1 else ""))
    lines.append("}")
    return "\n".join(lines) + "\n"


def write_expected(path: str, data: dict) -> None:
    if not data:
        if os.path.exists(path):
            os.unlink(path)
        return
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(format_expected(data))
    os.replace(tmp, path)


# -- ignore patterns -------------------------------------------------------

def _glob(pat: str) -> str:
    # '[' and ']' in field paths are literal index brackets, '*' and '?'
    # stay wildcards: results[*].undo matches results[3].undo
    return "".join("[[]" if ch == "[" else "[]]" if ch == "]" else ch for ch in pat)


def _path_matches(path: str, patterns: list[str]) -> bool:
    for pat in patterns:
        g = _glob(pat)
        if (fnmatch.fnmatchcase(path, g) or fnmatch.fnmatchcase(path, g + ".*")
                or fnmatch.fnmatchcase(path, g + "[[]*")):
            return True
    return False


def apply_ignores(actual: dict, patterns: list[str]) -> dict:
    """Drop ignored fields.  Patterns are glob-matched against field paths
    like ``screen``, ``results[3].ret``, ``results[*].undo``, ``snaps[0].cursor``."""
    if not patterns:
        return actual

    def walk(obj, path):
        if isinstance(obj, dict):
            out = {}
            for k, v in obj.items():
                p = f"{path}.{k}" if path else k
                if _path_matches(p, patterns):
                    continue
                out[k] = walk(v, p)
            return out
        if isinstance(obj, list) and path:
            out = []
            for i, v in enumerate(obj):
                p = f"{path}[{i}]"
                if _path_matches(p, patterns):
                    out.append("<ignored>")
                else:
                    out.append(walk(v, p))
            return out
        return obj

    return walk(actual, "")


def recordable(actual: dict) -> dict:
    return {k: actual[k] for k in COMPARED_KEYS if k in actual}


# -- comparison ----------------------------------------------------------------

def _count_leaves(v) -> int:
    if isinstance(v, dict):
        return sum(_count_leaves(x) for x in v.values()) or 1
    if isinstance(v, list) and v and isinstance(v[0], (dict, list)):
        return sum(_count_leaves(x) for x in v)
    if isinstance(v, list):
        return max(len(v), 1)
    return 1


def compare(expected: dict, actual: dict) -> tuple[list[tuple[str, object, object]], int, int]:
    """Returns (mismatches, fields_passed, fields_total)."""
    mism: list[tuple[str, object, object]] = []
    stats = [0, 0]

    def leaf(path, e, a):
        stats[1] += 1
        if e == a:
            stats[0] += 1
        else:
            mism.append((path, e, a))

    def cmp(path, e, a):
        if isinstance(e, dict) and isinstance(a, dict):
            for k in e:
                cmp(f"{path}.{k}" if path else k, e[k], a.get(k, "<missing>"))
            for k in a:
                if k not in e:
                    stats[1] += 1
                    mism.append((f"{path}.{k}" if path else k, "<missing>", a[k]))
            return
        if isinstance(e, list) and isinstance(a, list) and (path in ("results", "snaps", "screen") or path.endswith(".screen")):
            n = max(len(e), len(a))
            for i in range(n):
                ev = e[i] if i < len(e) else "<missing>"
                av = a[i] if i < len(a) else "<missing>"
                if isinstance(ev, dict) or isinstance(av, dict):
                    if isinstance(ev, dict) and isinstance(av, dict):
                        cmp(f"{path}[{i}]", ev, av)
                    else:
                        stats[1] += _count_leaves(ev if ev != "<missing>" else av)
                        mism.append((f"{path}[{i}]", ev, av))
                else:
                    leaf(f"{path}[{i}]", ev, av)
            return
        leaf(path, e, a)

    for k in COMPARED_KEYS:
        if k == "sanitizer":
            if "sanitizer" in actual or "sanitizer" in expected:
                leaf("sanitizer", expected.get("sanitizer"), actual.get("sanitizer"))
            continue
        if k not in expected:
            if k == "snaps" and actual.get("snaps"):
                leaf("snaps", None, actual["snaps"])
            continue
        cmp(k, expected[k], actual.get(k, "<missing>"))
    return mism, stats[0], stats[1]


def short(v, limit: int = 160) -> str:
    s = _dump_value(v)
    return s if len(s) <= limit else s[:limit - 3] + "..."
