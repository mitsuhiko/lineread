"""Source-based coverage of the reference implementation (llvm-cov)."""

from __future__ import annotations

import glob
import json
import os
import shutil
import subprocess
import sys


def _tool(name: str) -> list[str]:
    if sys.platform == "darwin":
        return ["xcrun", name]
    if shutil.which(name):
        return [name]
    cands = sorted(glob.glob(f"/usr/bin/{name}-*") + glob.glob(f"/usr/lib/llvm-*/bin/{name}"))
    if cands:
        return [cands[-1]]
    raise SystemExit(f"cannot find {name}; install llvm")


def prepare(covdir: str) -> dict:
    raw = os.path.join(covdir, "raw")
    shutil.rmtree(raw, ignore_errors=True)
    os.makedirs(raw, exist_ok=True)
    return {"LLVM_PROFILE_FILE": os.path.join(raw, "%8m.profraw")}


def report(covdir: str, binary: str, srcdir: str, html: str | None, as_json: bool) -> dict:
    raw = glob.glob(os.path.join(covdir, "raw", "*.profraw"))
    if not raw:
        raise SystemExit("no profile data was written")
    prof = os.path.join(covdir, "merged.profdata")
    subprocess.run(_tool("llvm-profdata") + ["merge", "-sparse", "-o", prof] + raw, check=True)
    out = subprocess.run(
        _tool("llvm-cov") + ["export", "-summary-only", f"-instr-profile={prof}", binary],
        check=True, capture_output=True, text=True).stdout
    data = json.loads(out)
    srcdir = os.path.realpath(srcdir)
    files = {}
    tot = {"lines": [0, 0], "branches": [0, 0], "functions": [0, 0]}
    for f in data["data"][0]["files"]:
        fn = os.path.realpath(f["filename"])
        if not fn.startswith(srcdir + os.sep):
            continue
        rel = os.path.relpath(fn, srcdir)
        s = f["summary"]
        files[rel] = {k: {"covered": s[k]["covered"], "count": s[k]["count"]} for k in tot}
        for k in tot:
            tot[k][0] += s[k]["covered"]
            tot[k][1] += s[k]["count"]
    summary = {
        "files": files,
        "total": {k: {"covered": v[0], "count": v[1]} for k, v in tot.items()},
    }
    with open(os.path.join(covdir, "summary.json"), "w") as fp:
        json.dump(summary, fp, indent=1, sort_keys=True)
    if html:
        srcs = [os.path.join(srcdir, f) for f in sorted(files)]
        subprocess.run(_tool("llvm-cov") + ["show", "-format=html", f"-instr-profile={prof}",
                                            f"-output-dir={html}", "-show-branches=count", binary] + srcs,
                       check=True)
    if not as_json:
        def pct(c):
            return f"{100.0 * c['covered'] / c['count']:6.1f}%" if c["count"] else "     - "
        print(f"\n{'file':<22} {'lines':>16} {'branches':>16} {'functions':>12}")
        for rel in sorted(files):
            s = files[rel]
            print(f"{rel:<22} {pct(s['lines'])} {s['lines']['covered']:>4}/{s['lines']['count']:<5}"
                  f"{pct(s['branches'])} {s['branches']['covered']:>4}/{s['branches']['count']:<5}"
                  f"{pct(s['functions'])}")
        t = summary["total"]
        print(f"{'TOTAL':<22} {pct(t['lines'])} {t['lines']['covered']:>4}/{t['lines']['count']:<5}"
              f"{pct(t['branches'])} {t['branches']['covered']:>4}/{t['branches']['count']:<5}"
              f"{pct(t['functions'])}")
        print(f"coverage data: {covdir}/summary.json" + (f", html: {html}/index.html" if html else ""))
    return summary
