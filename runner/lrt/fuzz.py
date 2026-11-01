"""Differential fuzzer: random but valid cases run against two implementations.

    ./rlt fuzz [--a ref] [--b rust] [--seed S] [-n N] [-j J] [--out tests/fuzz]
    ./rlt fuzz --flaky 5 -n 500          # reference only, every case 5 times
    ./rlt fuzz --print 17                # print the case of iteration 17

Every iteration derives its own RNG from (seed, iteration), so a run is
reproducible and any single iteration can be regenerated with --print.  A
case consists of a random inputrc (variables, editing mode), a prompt, a
history preload, a completion word list (or filename completion over a few
fixture files), a screen size, the reading mode (readline or callback) and a
random key sequence drawn from a weighted alphabet of bound keys, text and
UTF-8 characters, with occasional <dump>/<snap> probes.

Implementations are named like for ./rlt --impl (ref, asan, cov, rust) or
given as driver binaries (--a-driver/--b-driver).  Results, screens,
snapshots, cursors and exit status are compared field by field.  On a
mismatch the key sequence, then the inputrc lines, history and word list are
minimised (greedy delta debugging, re-running both implementations), and a
ready-to-record case is written to tests/fuzz/fuzz-<seed>-<iteration>.case:

    ./rlt tests/fuzz/fuzz-1-42.case --record      # against the reference

Only the compiled reference driver is needed (no source tree).
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import random
import sys
import time

from . import compare as cmp
from .casefile import Case, FileSpec
from .execute import run_case

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# ---------------------------------------------------------------------------
# alphabet

TEXT = list("abcdefghijklmnopqrstuvwxyz") + list("ABCXYZ0123456789") + [
    "\\s", "\\s", "\\s", ".", ",", "-", "_", "/", "(", ")", "'", "\"", "=", "$", ";", "*", "!"]
UTF8 = ["é", "ö", "ß", "ñ", "ж", "日", "本", "語"]
WORDS = ["foo", "bar", "baz", "alpha", "beta", "gamma", "hello", "world", "make", "ls -l",
         "echo", "git commit", "x", "a.b", "path/to/file", "qu'ote", "日本語", "café"]

EMACS_KEYS = [
    ("\\C-a", 4), ("\\C-e", 4), ("\\C-b", 5), ("\\C-f", 5), ("\\C-d", 3), ("\\C-h", 3),
    ("<bs>", 5), ("\\C-k", 3), ("\\C-u", 2), ("\\C-w", 3), ("\\C-y", 3), ("\\M-y", 2),
    ("\\C-t", 2), ("\\C-_", 3), ("\\C-xu", 1), ("\\C-x\\C-u", 1), ("\\M-r", 1),
    ("\\C-p", 3), ("\\C-n", 3), ("<up>", 2), ("<down>", 2), ("<left>", 3), ("<right>", 3),
    ("<home>", 1), ("<end>", 1), ("<del>", 1),
    ("\\M-b", 3), ("\\M-f", 3), ("\\M-d", 2), ("\\M-\\C-?", 2), ("\\M-c", 1), ("\\M-u", 1),
    ("\\M-l", 1), ("\\M-t", 1), ("\\M-.", 1), ("\\M-_", 1), ("\\M-<", 1), ("\\M->", 1),
    ("\\M-\\s", 1), ("<pause>\\C-@", 1), ("\\C-x\\C-x", 1), ("\\M-1", 1), ("\\M-3", 1),
    ("\\M--", 1), ("\\C-u2", 1), ("\\M-\\\\", 1), ("\\C-]a", 1), ("\\M-\\C-]b", 1),
    ("\\C-r", 2), ("\\C-s", 1), ("\\C-g", 1), ("\\M-p", 1), ("\\M-n", 1),
    ("\\t", 4), ("\\M-?", 1), ("\\M-*", 1), ("\\M-/", 1), ("\\M-~", 1), ("\\M-!", 1),
    ("\\C-x(", 1), ("\\C-x)", 1), ("\\C-xe", 1), ("\\C-q\\C-a", 1), ("\\C-v\\t", 1),
    ("\\M-#", 1), ("\\C-x*", 1), ("\\M-0\\C-k", 1), ("\\C-l", 1), ("\\M-\\C-y", 1),
    ("\\r", 3),
]

VI_INSERT_KEYS = [("\\e<pause>", 8), ("\\C-w", 2), ("\\C-u", 1), ("<bs>", 3), ("\\C-v\\t", 1),
                  ("\\t", 2), ("\\r", 3), ("\\C-h", 1), ("\\C-y", 1)]
VI_COMMAND_KEYS = [
    ("h", 4), ("l", 4), ("j", 2), ("k", 2), ("w", 4), ("b", 4), ("e", 3), ("W", 2), ("B", 2),
    ("E", 2), ("0", 2), ("$", 2), ("^", 1), ("x", 3), ("X", 2), ("dw", 2), ("db", 1), ("dd", 1),
    ("d$", 1), ("cw", 1), ("cb", 1), ("yw", 1), ("yy", 1), ("p", 2), ("P", 2), ("u", 3), (".", 2),
    ("i", 3), ("a", 3), ("A", 2), ("I", 2), ("rZ", 1), ("R", 1), ("~", 2), ("fa", 1), ("Fo", 1),
    ("ta", 1), ("To", 1), (";", 1), (",", 1), ("D", 1), ("C", 1), ("S", 1), ("s", 1), ("%", 1),
    ("2", 2), ("3", 1), ("G", 1), ("/foo\\r", 1), ("?a\\r", 1), ("n", 1), ("N", 1), ("|", 1),
    ("_", 1), ("*", 1), ("=", 1), ("\\\\", 1), ("#", 1), ("@a", 1), ("ma", 1), ("`a", 1),
    ("\\r", 2), ("\\C-e", 1),
]

BOOL_VARS = [
    "bind-tty-special-chars", "colored-completion-prefix", "colored-stats",
    "completion-ignore-case", "completion-map-case", "disable-completion", "enable-active-region",
    "enable-bracketed-paste", "expand-tilde", "history-preserve-point", "horizontal-scroll-mode",
    "mark-directories", "mark-modified-lines", "mark-symlinked-directories", "match-hidden-files",
    "menu-complete-display-prefix", "page-completions", "print-completions-horizontally",
    "revert-all-at-newline", "search-ignore-case", "show-all-if-ambiguous",
    "show-all-if-unmodified", "show-mode-in-prompt", "skip-completed-text", "visible-stats",
    "byte-oriented", "convert-meta", "input-meta", "output-meta", "enable-meta-key",
    "force-meta-prefix", "enable-keypad",
]
VALUED_VARS = {
    "bell-style": ["none", "visible", "audible"],
    "completion-query-items": ["0", "2", "5", "100"],
    "completion-display-width": ["-1", "0", "20"],
    "completion-prefix-display-length": ["0", "2"],
    "history-size": ["-1", "0", "2", "10"],
    "comment-begin": ["#", "//"],
    "isearch-terminators": ["\"\\C-[\\C-J\"", "\"ab\""],
    "emacs-mode-string": ["E>", "\"\\1\\e[1m\\2@\\1\\e[0m\\2\""],
    "vi-ins-mode-string": ["+", "INS"],
    "vi-cmd-mode-string": [":", "CMD"],
}
PROMPTS = ["> ", "", "prompt$ ", "a much longer prompt string> ", "two\\nlines> ",
           "\\001\\e[1m\\002bold\\001\\e[0m\\002> ", "日本> "]
SCREENS = [(80, 24), (80, 24), (40, 10), (20, 6), (33, 8)]
FILES = ["alpha.txt", "alpine.c", "beta", "dir1/", "dir1/inner", ".hidden", "with space"]


def _wchoice(rng, items):
    total = sum(w for _, w in items)
    x = rng.uniform(0, total)
    for v, w in items:
        x -= w
        if x <= 0:
            return v
    return items[-1][0]


# ---------------------------------------------------------------------------
# case model


class FuzzCase:
    """The minimisable parts of a generated case."""

    def __init__(self):
        self.inputrc: list[str] = []
        self.history: list[str] = []
        self.words: list[str] = []
        self.files: list[str] = []
        self.tokens: list[str] = []
        self.prompt = "> "
        self.mode = "readline"
        self.count = 3
        self.screen = (80, 24)
        self.completion = "words"
        self.name = "fuzz"

    def script(self) -> str:
        lines = []
        for h in self.history:
            lines.append(f"add_history {json.dumps(h, ensure_ascii=False)}")
        if self.completion == "words" and self.words:
            lines.append("complete_words " + " ".join(json.dumps(w, ensure_ascii=False) for w in self.words))
            lines.append("set rl_completion_entry_function words")
        prompt = '"' + self.prompt + '"'
        lines.append(f"{self.mode} {prompt} {self.count} addhist")
        lines.append("history_dump")
        return "".join(l + "\n" for l in lines)

    def keys(self) -> str:
        return "".join(self.tokens)

    def to_case(self) -> Case:
        c = Case(file="tests/fuzz/generated.case", name=self.name, line=0, tags=["fuzz"])
        c.inputrc = "".join(l + "\n" for l in self.inputrc)
        c.script = self.script()
        c.keys = self.keys()
        c.screen = self.screen
        c.timeout = 20.0
        for f in self.files:
            if f.endswith("/"):
                c.files.append(FileSpec(path=f.rstrip("/"), kind="dir"))
            else:
                c.files.append(FileSpec(path=f, kind="file", content="x\n"))
        return c

    def case_text(self, header: str) -> str:
        out = [header, "tags: fuzz", ""]
        out.append(f"=== {self.name}")
        if self.screen != (80, 24):
            out.append(f"screen: {self.screen[0]}x{self.screen[1]}")
        out.append("timeout: 20")
        for f in self.files:
            if f.endswith("/"):
                out.append(f"--- dir {f.rstrip('/')}")
            else:
                out.append(f"--- file {f}")
                out.append("x")
        if self.inputrc:
            out.append("--- inputrc")
            out.extend(self.inputrc)
        out.append("--- script")
        out.append(self.script().rstrip("\n"))
        out.append("--- keys")
        out.append(self.keys())
        return "\n".join(out) + "\n"

    def copy(self) -> "FuzzCase":
        n = FuzzCase()
        n.__dict__.update({k: (list(v) if isinstance(v, list) else v) for k, v in self.__dict__.items()})
        return n


def generate(seed: int, it: int) -> FuzzCase:
    rng = random.Random(f"{seed}:{it}")
    fc = FuzzCase()
    fc.name = f"fuzz-{seed}-{it}"
    vi = rng.random() < 0.3
    fc.inputrc.append("set keyseq-timeout 50")
    if vi:
        fc.inputrc.append("set editing-mode vi")
    for v in rng.sample(BOOL_VARS, rng.randint(0, 4)):
        if v == "enable-keypad" and rng.random() < 0.5:
            continue
        fc.inputrc.append(f"set {v} {rng.choice(['on', 'off'])}")
    for v in rng.sample(sorted(VALUED_VARS), rng.randint(0, 2)):
        fc.inputrc.append(f"set {v} {rng.choice(VALUED_VARS[v])}")
    if rng.random() < 0.15:
        fc.inputrc.append('"\\C-xm": "macro text"')
    fc.prompt = rng.choice(PROMPTS)
    fc.screen = rng.choice(SCREENS)
    fc.mode = "callback" if rng.random() < 0.2 else "readline"
    fc.count = rng.randint(1, 3)
    fc.history = [rng.choice(WORDS) for _ in range(rng.randint(0, 5))]
    if rng.random() < 0.25:
        fc.completion = "filename"
        fc.files = sorted(set(rng.sample(FILES, rng.randint(1, len(FILES)))))
        if "dir1/inner" in fc.files and "dir1/" not in fc.files:
            fc.files.insert(0, "dir1/")
        fc.files.sort(key=lambda f: (f.count("/") - f.endswith("/"), f))
    else:
        fc.completion = "words"
        fc.words = sorted(set(rng.choice(WORDS).split()[0] for _ in range(rng.randint(0, 6))))
    in_cmd = False
    for _ in range(rng.randint(3, 40)):
        r = rng.random()
        if r < 0.04:
            fc.tokens.append(rng.choice(["<dump>", "<snap>"]))
            continue
        if vi and in_cmd:
            k = _wchoice(rng, VI_COMMAND_KEYS)
            if k in ("i", "a", "A", "I", "cw", "cb", "C", "S", "s", "R"):
                in_cmd = False
            if k == "\\r":
                in_cmd = False
            fc.tokens.append(k)
            continue
        if r < 0.45:
            fc.tokens.append("".join(rng.choice(TEXT) for _ in range(rng.randint(1, 4))))
        elif r < 0.52:
            fc.tokens.append(rng.choice(UTF8))
        elif vi:
            k = _wchoice(rng, VI_INSERT_KEYS)
            if k == "\\e<pause>":
                in_cmd = True
            fc.tokens.append(k)
        else:
            fc.tokens.append(_wchoice(rng, EMACS_KEYS))
    return fc


# ---------------------------------------------------------------------------
# running


def _comparable(res: dict) -> dict:
    return cmp.recordable(res)


def run_pair(fc: FuzzCase, da: tuple, db: tuple):
    """Returns (mismatches, res_a, res_b)."""
    case = fc.to_case()
    ra = run_case(case, da[0], da[1])
    rb = run_case(fc.to_case(), db[0], db[1])
    exp = _comparable(ra)
    mism, _, _ = cmp.compare(exp, rb)
    if "runner_error" in ra or "runner_error" in rb:
        mism = mism or [("runner_error", ra.get("runner_error"), rb.get("runner_error"))]
    return mism, ra, rb


def _still_fails(fc, da, db) -> bool:
    mism, _, _ = run_pair(fc, da, db)
    return bool(mism)


def minimise(fc: FuzzCase, da, db, budget: int = 400) -> FuzzCase:
    """Greedy delta debugging over tokens, then inputrc/history/words lines."""
    runs = 0
    for attr in ("tokens", "inputrc", "history", "words", "files"):
        n = 2
        while runs < budget:
            items = getattr(fc, attr)
            if not items:
                break
            chunk = max(1, len(items) // n)
            reduced = False
            for start in range(0, len(items), chunk):
                cand = fc.copy()
                setattr(cand, attr, items[:start] + items[start + chunk:])
                if attr == "inputrc" and "set keyseq-timeout 50" not in cand.inputrc:
                    continue
                runs += 1
                if _still_fails(cand, da, db):
                    fc = cand
                    reduced = True
                    break
                if runs >= budget:
                    break
            if not reduced:
                if chunk == 1:
                    break
                n = min(len(items), n * 2)
    return fc


def _iteration(args):
    try:
        return _iteration1(args)
    except Exception:  # noqa: BLE001
        import traceback
        return args[1], [("runner_error", None, traceback.format_exc())], 0.0


def _iteration1(args):
    seed, it, da, db, flaky = args
    fc = generate(seed, it)
    t0 = time.monotonic()
    if flaky:
        runs = [_comparable(run_case(fc.to_case(), da[0], da[1])) for _ in range(flaky)]
        bad = [i for i, r in enumerate(runs) if r != runs[0]]
        mism = []
        if bad:
            m, _, _ = cmp.compare(runs[0], runs[bad[0]])
            mism = m or [("?", "differs", "differs")]
        return it, mism, time.monotonic() - t0
    mism, _, _ = run_pair(fc, da, db)
    return it, mism, time.monotonic() - t0


def resolve_driver(name: str | None, path: str | None) -> tuple[str, str]:
    from . import cli
    if path:
        return os.path.abspath(path), os.path.basename(path)
    cli.build(name)
    return cli.driver_path(name), name


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="rlt fuzz", description="differential fuzzer (see runner/lrt/fuzz.py)")
    ap.add_argument("--a", default="ref", help="implementation A, the expectation (default ref)")
    ap.add_argument("--b", default="rust", help="implementation B (default rust)")
    ap.add_argument("--a-driver", help="driver binary for A instead of building --a")
    ap.add_argument("--b-driver", help="driver binary for B instead of building --b")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("-n", "--iterations", type=int, default=200)
    ap.add_argument("--start", type=int, default=0, help="first iteration number")
    ap.add_argument("-j", "--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--out", default=os.path.join(ROOT, "tests", "fuzz"), help="where regression cases go")
    ap.add_argument("--no-minimise", action="store_true")
    ap.add_argument("--max-failures", type=int, default=5, help="stop minimising/writing after N failures")
    ap.add_argument("--flaky", type=int, metavar="N", help="run every case N times against A only (harness determinism)")
    ap.add_argument("--print", type=int, metavar="IT", help="print the case of iteration IT and exit")
    ap.add_argument("--json", action="store_true", help="JSON summary on stdout")
    opts = ap.parse_args(argv)

    if opts.print is not None:
        sys.stdout.write(generate(opts.seed, opts.print).case_text(f"# fuzz seed {opts.seed} iteration {opts.print}"))
        return 0

    da = resolve_driver(opts.a, opts.a_driver)
    db = da if opts.flaky else resolve_driver(opts.b, opts.b_driver)
    work = [(opts.seed, it, da, db, opts.flaky or 0)
            for it in range(opts.start, opts.start + opts.iterations)]
    t0 = time.monotonic()
    failures = []
    done = 0
    with concurrent.futures.ProcessPoolExecutor(max_workers=opts.jobs) as ex:
        for it, mism, dt in ex.map(_iteration, work, chunksize=4):
            done += 1
            if mism:
                failures.append((it, mism))
                print(f"MISMATCH seed={opts.seed} iteration={it}: " +
                      "; ".join(f"{p}" for p, _, _ in mism[:5]), file=sys.stderr)
            if done % 100 == 0:
                print(f"... {done}/{len(work)} iterations, {len(failures)} mismatches "
                      f"({time.monotonic() - t0:.0f}s)", file=sys.stderr)
    written = []
    if failures and not opts.flaky:
        os.makedirs(opts.out, exist_ok=True)
        for it, mism in failures[:opts.max_failures]:
            fc = generate(opts.seed, it)
            if not opts.no_minimise:
                fc = minimise(fc, da, db)
            fc.name = f"fuzz-{opts.seed}-{it}"
            path = os.path.join(opts.out, f"fuzz-{opts.seed}-{it}.case")
            m2, _, _ = run_pair(fc, da, db)
            detail = "".join(f"#   {p}: expected {cmp.short(e)} actual {cmp.short(a)}\n" for p, e, a in m2[:8])
            header = (f"# Found by ./rlt fuzz --seed {opts.seed} (iteration {it}): {da[1]} vs {db[1]}.\n"
                      f"# Record the reference behaviour with: ./rlt {os.path.relpath(path, ROOT)} --record\n"
                      f"# First differences:\n{detail}").rstrip("\n")
            with open(path, "w", encoding="utf-8") as f:
                f.write(fc.case_text(header))
            written.append(path)
            print(f"wrote {os.path.relpath(path, ROOT)}", file=sys.stderr)
    summary = {"seed": opts.seed, "iterations": len(work), "start": opts.start,
               "a": da[1], "b": db[1], "flaky_repeat": opts.flaky,
               "mismatches": [it for it, _ in failures], "written": written,
               "elapsed": round(time.monotonic() - t0, 1)}
    if opts.json:
        json.dump(summary, sys.stdout, indent=1)
        sys.stdout.write("\n")
    else:
        what = f"{da[1]} x{opts.flaky}" if opts.flaky else f"{da[1]} vs {db[1]}"
        print(f"fuzz seed={opts.seed} iterations={len(work)} ({what}): {len(failures)} mismatches "
              f"in {summary['elapsed']}s")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
