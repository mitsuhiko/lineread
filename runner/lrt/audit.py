"""Completeness audit: which public names are exercised by how many cases.

`./rlt --audit` runs the selected cases (default: all) with the driver's usage
trace enabled (RLD_TRACE) and counts, per case, which

* bindable functions (rl_funmap_names) were dispatched (key binding, named
  command, `invoke`, `rl_xxx COUNT KEY`),
* inputrc variables (rl_variable_dumper) were set, and to which values
  (static scan of inputrc sections, fixture files and scripts),
* functions and variables declared in include/readline/*.h were called or
  read/written (driver commands, `set`/`get`, hooks installed and called).

Names listed in docs/UNTESTED.md (list items starting with `name`) are
reported as documented exceptions.  The result is a text report (and JSON
with --json) that is committed as docs/AUDIT.txt.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MIN_CASES = 3

# driver commands -> public functions/variables they exercise
CUSTOM_MAP = {
    "readline": ["readline"],
    "callback": ["rl_callback_handler_install", "rl_callback_read_char", "rl_callback_handler_remove"],
    "callback_install": ["rl_callback_handler_install"],
    "callback_read_char": ["rl_callback_read_char"],
    "history_expand": ["history_expand"],
    "history_get_time": ["history_get_time", "history_get"],
    "history_dump": ["history_base", "history_length", "history_max_entries", "history_is_stifled",
                     "where_history", "history_list"],
    "history_list": ["history_list"],
    "history_get_history_state": ["history_get_history_state"],
    "history_state_save": ["history_get_history_state"],
    "history_state_restore": ["history_set_history_state"],
    "copy_history_entry": ["copy_history_entry", "free_history_entry"],
    "get_history_event": ["get_history_event"],
    "rl_discard_keymap": ["rl_discard_keymap"],
    "rl_free_keymap": ["rl_free_keymap"],
    "keymap_dump": [],
    "display_match_list": ["rl_display_match_list"],
    "rl_add_undo": ["rl_add_undo"],
    "defun": ["rl_add_defun"],
    "invoke": ["rl_named_function"],
    "capture": ["rl_outstream"],
    "dump": ["rl_line_buffer", "rl_point", "rl_end", "rl_mark", "rl_editing_mode", "rl_get_keymap",
             "rl_explicit_arg", "rl_numeric_arg", "rl_insert_mode"],
    "dumpfull": ["rl_line_buffer", "rl_point", "rl_end", "rl_mark", "rl_editing_mode", "rl_get_keymap",
                 "rl_explicit_arg", "rl_numeric_arg", "rl_insert_mode", "rl_readline_state",
                 "rl_undo_list", "rl_prompt", "rl_display_prompt", "rl_last_func", "rl_done",
                 "rl_eof_found", "rl_pending_input", "rl_mark_active_p", "rl_executing_key",
                 "rl_executing_keyseq", "rl_key_sequence_length", "rl_executing_macro",
                 "history_length", "history_base", "where_history"],
}
# what every reading case exercises through the driver's input path
READING = {"readline", "callback", "callback_read_char", "rl_callback_read_char", "rl_read_key", "rl_getc"}

KEYMAP_VARS = {
    "emacs_standard_keymap": ["emacs", "emacs-standard"],
    "emacs_meta_keymap": ["emacs-meta"],
    "emacs_ctlx_keymap": ["emacs-ctlx"],
    "vi_movement_keymap": ["vi", "vi-move", "vi-command"],
    "vi_insertion_keymap": ["vi-insert"],
}

ENUM_VALUES = {
    "bell-style": ["none", "visible", "audible"],
    "editing-mode": ["emacs", "vi"],
    "keymap": ["emacs", "emacs-standard", "emacs-meta", "emacs-ctlx", "vi", "vi-move",
               "vi-command", "vi-insert"],
}


def _run_driver(driver: str, script: str) -> list[dict]:
    with tempfile.TemporaryDirectory() as d:
        sp = os.path.join(d, "script")
        rp = os.path.join(d, "results")
        with open(sp, "w") as f:
            f.write(script)
        env = dict(os.environ, RLD_RESULTS=rp, INPUTRC=os.path.join(d, "none"), HOME=d,
                   TERM="dumb")
        subprocess.run([driver, sp], env=env, stdin=subprocess.DEVNULL,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        with open(rp) as f:
            return [json.loads(x) for x in f if x.strip()]


def inventory(driver: str, platform_dir: str) -> dict:
    """Names the audit checks."""
    recs = _run_driver(driver, "option nodefuns\nrl_funmap_names\nrl_initialize\nrl_variable_dumper 1\n")
    funmap = recs[0]["ret"]
    variables = {}
    for line in recs[-1]["out"].splitlines():
        parts = line.split(None, 2)
        if len(parts) >= 2 and parts[0] == "set":
            variables[parts[1]] = parts[2] if len(parts) > 2 else ""
    bindable = {}
    out = subprocess.run([driver, "--bindable"], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        c, n = line.split()
        if n != "?unknown":
            bindable[c] = n
    abi_path = os.path.join(ROOT, "conformance", f"abi.{platform_dir}.json")
    with open(abi_path) as f:
        surf = json.load(f)["surface"]
    funcs = sorted(k for k, v in surf.items() if v["kind"] == "FunctionDecl")
    vars_ = sorted(k for k, v in surf.items() if v["kind"] == "VarDecl")
    hooks = []
    cmds = subprocess.run([driver, "--commands"], capture_output=True, text=True, check=True).stdout
    sect = None
    for line in cmds.splitlines():
        if line.startswith("# hooks"):
            sect = "hooks"
        elif line.startswith("#"):
            sect = None
        elif sect == "hooks":
            hooks.append(line.strip())
    return {"funmap": funmap, "variables": variables, "bindable": bindable,
            "functions": funcs, "vars": vars_, "hooks": hooks}


def untested() -> dict[str, str]:
    p = os.path.join(ROOT, "docs", "UNTESTED.md")
    out = {}
    if not os.path.exists(p):
        return out
    with open(p) as f:
        for line in f:
            m = re.match(r"^\s*[-*]\s+`([^`]+)`(?:\s*(?:—|-|:)\s*(.*))?", line)
            if m:
                for n in re.split(r"[,\s]+", m.group(1)):
                    if n:
                        out[n] = (m.group(2) or "").strip()
    return out


_SET_RE = re.compile(r"""(?:^|[\s"'])set\s+([A-Za-z][A-Za-z0-9-]*)\s+("[^"\n]*"|'[^'\n]*'|[^\s"']+)""",
                     re.MULTILINE)
_VBIND_RE = re.compile(r"""rl_variable_bind\s+("[^"\n]*"|'[^'\n]*'|\S+)\s+("[^"\n]*"|'[^'\n]*'|\S+)""")
_VVAL_RE = re.compile(r"""rl_variable_value\s+("[^"\n]*"|'[^'\n]*'|\S+)""")


def _unq(s: str) -> str:
    if len(s) >= 2 and s[0] == s[-1] and s[0] in "\"'":
        return s[1:-1]
    return s


def scan_variables(case, known: dict) -> dict[str, set]:
    """inputrc variable -> set of values used in CASE (static)."""
    texts = [case.inputrc or "", case.script or ""] + [f.content for f in case.files if f.kind == "file"]
    lower = {k.lower(): k for k in known}
    used: dict[str, set] = {}
    for t in texts:
        for m in _SET_RE.finditer(t):
            n = lower.get(m.group(1).lower())
            if n:
                used.setdefault(n, set()).add(_unq(m.group(2)))
        for m in _VBIND_RE.finditer(t):
            n = lower.get(_unq(m.group(1)).lower())
            if n:
                used.setdefault(n, set()).add(_unq(m.group(2)))
        for m in _VVAL_RE.finditer(t):
            n = lower.get(_unq(m.group(1)).lower())
            if n:
                used.setdefault(n, set()).add("<read>")
    return used


def _norm_value(var: str, default: str, v: str) -> str:
    if default in ("on", "off"):
        return "on" if v.lower() in ("on", "1") else "off"
    return v.lower() if var in ENUM_VALUES else v


def analyse(inv: dict, cases_runs: list) -> dict:
    funmap_cnt = {n: 0 for n in inv["funmap"]}
    fn_cnt = {n: 0 for n in inv["functions"]}
    var_cnt = {n: 0 for n in inv["vars"]}
    hook_set_cnt = {n: 0 for n in inv["hooks"]}
    hook_call_cnt = {n: 0 for n in inv["hooks"]}
    ivar_vals: dict[str, dict[str, int]] = {n: {} for n in inv["variables"]}
    ivar_cnt = {n: 0 for n in inv["variables"]}
    rev_bindable: dict[str, list[str]] = {}
    for c, n in inv["bindable"].items():
        rev_bindable.setdefault(n, []).append(c)
    known_cmds = set(fn_cnt)
    for case, trace in cases_runs:
        used_f, used_fn, used_v, hs, hc = set(), set(), set(), set(), set()
        for line in trace:
            kind, _, name = line.partition(" ")
            if kind == "func":
                used_f.add(name)
                used_fn.update(rev_bindable.get(name, []))
            elif kind in ("cmd", "api"):
                if name in known_cmds:
                    used_fn.add(name)
                for x in CUSTOM_MAP.get(name, []):
                    (used_fn if x in fn_cnt else used_v).add(x)
                if name in READING:
                    used_fn.add("rl_getc")
            elif kind in ("set", "get"):
                if name in hook_set_cnt:
                    if kind == "set":
                        hs.add(name)
                    used_v.add(name)
                else:
                    used_v.add(name)
            elif kind == "hook":
                hc.add(name)
                used_v.add(name)
        text = (case.script or "") + "\n" + (case.inputrc or "")
        for var, names in KEYMAP_VARS.items():
            if any(re.search(r"(?<![\w-])" + re.escape(n) + r"(?![\w-])", text) for n in names):
                used_v.add(var)
        for n in used_f:
            if n in funmap_cnt:
                funmap_cnt[n] += 1
        for n in used_fn:
            if n in fn_cnt:
                fn_cnt[n] += 1
        for n in used_v:
            if n in var_cnt:
                var_cnt[n] += 1
        for n in hs:
            hook_set_cnt[n] += 1
        for n in hc:
            if n in hook_call_cnt:
                hook_call_cnt[n] += 1
        for var, vals in scan_variables(case, inv["variables"]).items():
            ivar_cnt[var] += 1
            for v in vals:
                nv = v if v == "<read>" else _norm_value(var, inv["variables"][var], v)
                ivar_vals[var][nv] = ivar_vals[var].get(nv, 0) + 1
    return {"funmap": funmap_cnt, "functions": fn_cnt, "vars": var_cnt,
            "hooks_set": hook_set_cnt, "hooks_called": hook_call_cnt,
            "ivars": ivar_cnt, "ivar_values": ivar_vals}


def report(inv: dict, res: dict, ncases: int, out_path: str | None, as_json: bool) -> dict:
    unt = untested()
    lines = []
    gaps = []

    def section(title, counts, extra=None):
        lines.append(f"\n## {title}\n")
        low = [(n, c) for n, c in sorted(counts.items()) if c < MIN_CASES]
        ok = len(counts) - len(low)
        lines.append(f"{ok}/{len(counts)} exercised by >= {MIN_CASES} cases")
        for n, c in low:
            why = unt.get(n)
            tag = f"untested (docs/UNTESTED.md): {why}" if why is not None else "GAP"
            if why is None:
                gaps.append(f"{title}: {n}")
            lines.append(f"  {n:<44} {c:>4}  {tag}")
        if extra:
            extra()

    section("Bindable functions (rl_funmap_names)", res["funmap"])

    def ivar_extra():
        lines.append("\n  values (cases per value; '*' = default):")
        for n in sorted(res["ivars"]):
            d = inv["variables"][n]
            vals = res["ivar_values"][n]
            shown = ", ".join(f"{v}{'*' if v == d else ''}:{c}" for v, c in sorted(vals.items()))
            missing = []
            if d in ("on", "off"):
                nd = "off" if d == "on" else "on"
                if vals.get(nd, 0) < MIN_CASES:
                    missing.append(f"non-default '{nd}' in < {MIN_CASES} cases")
            elif n in ENUM_VALUES:
                miss = [v for v in ENUM_VALUES[n] if not vals.get(v)]
                if miss:
                    missing.append("values never used: " + " ".join(miss))
            else:
                if len([v for v in vals if v not in (d, "<read>")]) < 2:
                    missing.append("fewer than 2 non-default values")
            flag = ""
            if missing:
                if n in unt:
                    flag = "  (docs/UNTESTED.md)"
                else:
                    flag = "  GAP: " + "; ".join(missing)
                    gaps.append(f"inputrc variable value: {n}: {'; '.join(missing)}")
            lines.append(f"  {n:<36} [{d}] {shown}{flag}")

    section("inputrc variables (rl_variable_dumper)", res["ivars"], ivar_extra)
    section("Public functions (include/readline/*.h)", res["functions"])
    section("Public variables and hooks (include/readline/*.h)", res["vars"])
    lines.append("\n## Hooks: cases installing / cases where the hook was called\n")
    for n in sorted(res["hooks_set"]):
        s, c = res["hooks_set"][n], res["hooks_called"][n]
        flag = ""
        if n == "rl_getc_function":
            flag = "  (owned by the driver)"
        elif c < MIN_CASES and n not in ("rl_completion_entry_function", "rl_menu_completion_entry_function"):
            if n in unt:
                flag = "  (docs/UNTESTED.md)"
            else:
                flag = "  GAP"
                gaps.append(f"hook called: {n}")
        lines.append(f"  {n:<44} set {s:>4}  called {c:>4}{flag}")
    head = [f"# lineread completeness audit ({ncases} cases)",
            "",
            "Generated by `./rlt --audit --audit-out docs/AUDIT.txt` (runner/lrt/audit.py).",
            f"A name is covered when at least {MIN_CASES} cases exercise it; counts are cases.",
            f"Gaps: {len(gaps)}"]
    text = "\n".join(head + lines) + "\n"
    if out_path:
        with open(out_path, "w") as f:
            f.write(text)
    if not as_json:
        print(text)
    return {"gaps": gaps, "counts": res}
