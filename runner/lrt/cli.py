"""Command line entry point: ./rlt --help"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import fnmatch
import json
import os
import subprocess
import sys
import time

from . import compare as cmp
from . import coverage
from .casefile import Case, CaseError, find_case_files, parse_case_file
from .execute import PLATFORM, run_case

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PLAT_DIR = f"{os.uname().sysname.lower()}-{os.uname().machine}"
REF = os.environ.get("LINEREAD_REF", os.path.join(os.path.dirname(ROOT), "lineread-ref"))


def driver_path(impl: str) -> str:
    return os.path.join(ROOT, "build", PLAT_DIR, impl, "rl-driver")


def build(impl: str) -> None:
    r = subprocess.run(["make", "-s", f"IMPL={impl}"], cwd=ROOT)
    if r.returncode != 0:
        raise SystemExit(f"building the driver for IMPL={impl} failed")


def _run_one(args):
    case, driver, impl, trace, keep, extra = args
    t0 = time.monotonic()
    try:
        res = run_case(case, driver, impl, trace_enabled=trace, keep=keep, extra_env=extra)
    except Exception as e:  # noqa: BLE001
        import traceback
        res = {"exit": "runner-error", "runner_error": traceback.format_exc() or str(e)}
    res["_time"] = time.monotonic() - t0
    return res


def select_cases(opts) -> list[Case]:
    files = find_case_files(opts.paths or ["tests"], ROOT)
    cases = []
    for f in files:
        rel = os.path.relpath(f, ROOT)
        for c in parse_case_file(f, rel):
            cases.append(c)
    out = []
    for c in cases:
        if opts.only_names and c.name not in opts.only_names.get(c.file, {c.name}):
            continue
        if opts.name and not any(fnmatch.fnmatchcase(c.name, g) or fnmatch.fnmatchcase(c.id, g) for g in opts.name):
            continue
        if opts.tag and not any(t in c.tags for t in opts.tag):
            continue
        if opts.exclude_tag and any(t in c.tags for t in opts.exclude_tag):
            continue
        out.append(c)
    return out


def print_actual(case: Case, res: dict) -> None:
    print(f"--- {case.id}  exit={res.get('exit')}")
    for i, r in enumerate(res.get("results", [])):
        print(f"  results[{i}] {cmp.short(r, 10000)}")
    cols = case.screen[0]
    for si, snap in enumerate(res.get("snaps", [])):
        print(f"  snap {si} cursor={snap['cursor']}")
        for row in snap["screen"]:
            print(f"   |{row:<{cols}}|")
    print(f"  screen cursor={res.get('cursor')}")
    print("   +" + "-" * cols + "+")
    for row in res.get("screen", []):
        print(f"   |{row:<{cols}}|")
    print("   +" + "-" * cols + "+")
    if "sanitizer" in res:
        print("  sanitizer:\n" + res["sanitizer"])
    if "runner_error" in res:
        print("  runner error:\n" + res["runner_error"])
    if "_raw" in res:
        print("  raw output: " + repr(res["_raw"]))


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="rlt", description="lineread behavioural test runner")
    ap.add_argument("paths", nargs="*", help="case files/directories (default: tests); FILE::NAME selects one case")
    ap.add_argument("--impl", default="ref", help="ref|cov|asan|rust (default ref)")
    ap.add_argument("--driver", help="use this driver binary instead of building one")
    ap.add_argument("--no-build", action="store_true", help="do not run make first")
    ap.add_argument("-t", "--tag", action="append", help="only cases with this tag (repeatable, any)")
    ap.add_argument("-T", "--exclude-tag", action="append", help="skip cases with this tag")
    ap.add_argument("-k", "--name", action="append", help="glob on case name or FILE::NAME id")
    ap.add_argument("-j", "--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--repeat", type=int, default=1, help="run every case N times (flakiness check)")
    ap.add_argument("--record", action="store_true", help="write expectations (common file)")
    ap.add_argument("--record-platform", action="store_true",
                    help=f"write per-platform overrides ({PLATFORM}) for cases that differ from the common file")
    ap.add_argument("--json", action="store_true", help="machine-readable summary on stdout")
    ap.add_argument("-v", "--verbose", action="store_true", help="list passing cases too")
    ap.add_argument("--show", action="store_true", help="print actual results and screens")
    ap.add_argument("--trace", action="store_true", help="trace the runner/driver protocol (implies -j1)")
    ap.add_argument("--keep", action="store_true", help="keep per-case temp directories")
    ap.add_argument("--list", action="store_true", help="list selected cases and exit")
    ap.add_argument("--max-diff", type=int, default=12, help="mismatches shown per failing case")
    ap.add_argument("--coverage", action="store_true", help="run against the coverage build and report")
    ap.add_argument("--coverage-html", metavar="DIR", help="also write an llvm-cov HTML report")
    ap.add_argument("--timeout-scale", type=float, default=1.0)
    ap.add_argument("--conformance", action="store_true",
                    help="also run the header/ABI conformance tier (default when no paths/filters are given)")
    ap.add_argument("--no-conformance", action="store_true")
    opts = ap.parse_args(argv)
    if not (opts.paths or opts.tag or opts.exclude_tag or opts.name or opts.record or
            opts.record_platform or opts.no_conformance):
        opts.conformance = True

    # FILE::NAME selectors
    opts.only_names = {}
    paths = []
    for p in opts.paths:
        if "::" in p:
            f, n = p.split("::", 1)
            rel = os.path.relpath(os.path.abspath(f), ROOT)
            opts.only_names.setdefault(rel, set()).add(n)
            paths.append(f)
        else:
            paths.append(p)
    opts.paths = paths
    if opts.coverage:
        opts.impl = "cov"
    if opts.trace:
        opts.jobs = 1

    try:
        cases = select_cases(opts)
    except CaseError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except Exception as e:  # keys errors etc.
        print(f"error: {e}", file=sys.stderr)
        return 2
    if opts.list:
        for c in cases:
            print(f"{c.id}  [{' '.join(c.tags)}]")
        return 0

    driver = opts.driver or driver_path(opts.impl)
    if not opts.driver and not opts.no_build:
        build(opts.impl)
    if not os.path.exists(driver):
        print(f"error: driver {driver} not found", file=sys.stderr)
        return 2

    extra_env = {}
    covdir = os.path.join(ROOT, "build", PLAT_DIR, "coverage")
    if opts.coverage:
        extra_env.update(coverage.prepare(covdir))

    # validate key notation up front
    from .keys import KeyError_, parse_keys
    for c in cases:
        try:
            parse_keys(c.keys, c.signals)
        except KeyError_ as e:
            print(f"error: {c.id}: keys: {e}", file=sys.stderr)
            return 2

    runnable = []
    skipped = []
    for c in cases:
        if c.skip or (c.platforms and PLATFORM not in c.platforms):
            skipped.append(c)
        else:
            c.timeout *= opts.timeout_scale
            runnable.append(c)

    t0 = time.monotonic()
    work = [(c, driver, opts.impl, opts.trace, opts.keep, extra_env) for c in runnable for _ in range(opts.repeat)]
    results: dict[str, list[dict]] = collections.defaultdict(list)
    if opts.jobs <= 1 or len(work) <= 1:
        for w in work:
            results[w[0].id].append(_run_one(w))
    else:
        with concurrent.futures.ProcessPoolExecutor(max_workers=opts.jobs) as ex:
            futs = {ex.submit(_run_one, w): w[0].id for w in work}
            for fut in concurrent.futures.as_completed(futs):
                results[futs[fut]].append(fut.result())
    elapsed = time.monotonic() - t0

    # group by file for expectations
    by_file: dict[str, list[Case]] = collections.defaultdict(list)
    for c in runnable:
        by_file[c.file].append(c)

    case_reports = []
    record_mode = opts.record or opts.record_platform
    for f, fcases in by_file.items():
        absf = os.path.join(ROOT, f)
        common, plat = cmp.load_expected(absf, PLATFORM)
        changed = False
        for c in fcases:
            runs = [cmp.apply_ignores(r, c.ignore) for r in results[c.id]]
            rec_runs = [cmp.recordable(r) for r in runs]
            stable = all(r == rec_runs[0] for r in rec_runs)
            rep = {"id": c.id, "file": c.file, "name": c.name, "area": c.area, "tags": c.tags,
                   "time": round(sum(r.get("_time", 0) for r in results[c.id]) / max(len(runs), 1), 3)}
            if record_mode:
                if any("runner_error" in r for r in runs):
                    rep["status"] = "error"
                    rep["message"] = runs[0].get("runner_error", "")
                elif not stable:
                    rep["status"] = "flaky"
                    rep["message"] = "runs differ; expectation not recorded"
                else:
                    new = rec_runs[0]
                    if opts.record:
                        if common.get(c.name) != new:
                            common[c.name] = new
                            changed = True
                        rep["status"] = "recorded"
                    else:
                        if common.get(c.name) == new:
                            if c.name in plat:
                                del plat[c.name]
                                changed = True
                        elif plat.get(c.name) != new:
                            plat[c.name] = new
                            changed = True
                        rep["status"] = "recorded"
                case_reports.append((c, rep, runs))
                continue
            exp = plat.get(c.name, common.get(c.name))
            if exp is None:
                rep["status"] = "error"
                rep["message"] = "no expectation recorded (run with --record)"
                rep["fields_passed"], rep["fields_total"] = 0, 1
                case_reports.append((c, rep, runs))
                continue
            exp = cmp.apply_ignores(exp, c.ignore)
            statuses = []
            first_fail = None
            fp = ft = 0
            for r in runs:
                mism, p, t = cmp.compare(exp, r)
                statuses.append(not mism)
                if mism and first_fail is None:
                    first_fail = (mism, p, t, r)
                if fp == 0 and ft == 0 or mism:
                    fp, ft = p, t
            if all(statuses):
                rep["status"] = "pass"
            elif any(statuses):
                rep["status"] = "flaky"
            else:
                rep["status"] = "fail"
            rep["fields_passed"], rep["fields_total"] = fp, ft
            if first_fail:
                rep["mismatches"] = [{"path": p, "expected": e, "actual": a} for p, e, a in first_fail[0]]
                if "runner_error" in first_fail[3]:
                    rep["message"] = first_fail[3]["runner_error"]
            case_reports.append((c, rep, runs))
        if record_mode and changed:
            names = [c.name for c in parse_case_file(absf, f)]
            order = {n: i for i, n in enumerate(names)}
            if opts.record:
                common = {k: common[k] for k in sorted(common, key=lambda n: order.get(n, 1 << 30)) if k in order}
                cmp.write_expected(cmp.expected_path(absf), common)
            else:
                plat = {k: plat[k] for k in sorted(plat, key=lambda n: order.get(n, 1 << 30)) if k in order}
                cmp.write_expected(cmp.expected_path(absf, PLATFORM), plat)

    for c in skipped:
        case_reports.append((c, {"id": c.id, "file": c.file, "name": c.name, "area": c.area,
                                 "tags": c.tags, "status": "skip",
                                 "message": c.skip or f"not for {PLATFORM}"}, []))

    # sort reports in case order
    order = {c.id: i for i, c in enumerate(cases)}
    case_reports.sort(key=lambda x: order.get(x[1]["id"], 0))

    if opts.conformance:
        sys.path.insert(0, os.path.join(ROOT, "conformance"))
        import check as conf  # noqa: E402
        for it in conf.do_check(opts.impl):
            c = Case(file="conformance", name=it["id"], line=0, tags=["conformance"])
            rep = {"id": f"conformance::{it['id']}", "file": "conformance", "name": it["id"],
                   "area": "conformance", "tags": ["conformance"], "status": it["status"],
                   "fields_passed": int(it["status"] == "pass"), "fields_total": 1}
            if "message" in it:
                rep["message"] = it["message"]
            if it["status"] == "fail" and "expected" in it:
                rep["mismatches"] = [{"path": it["id"], "expected": it["expected"], "actual": it["actual"]}]
            case_reports.append((c, rep, []))

    counts = collections.Counter(rep["status"] for _, rep, _ in case_reports)
    areas: dict[str, dict] = collections.defaultdict(lambda: collections.Counter())
    tags: dict[str, dict] = collections.defaultdict(lambda: collections.Counter())
    for c, rep, _ in case_reports:
        if rep["status"] == "skip":
            continue
        area = rep.get("area", c.area)
        for bucket in [areas[area]] + [tags[t] for t in c.tags]:
            bucket["total"] += 1
            bucket["passed"] += rep["status"] in ("pass", "recorded")
            bucket["fields_passed"] += rep.get("fields_passed", 0)
            bucket["fields_total"] += rep.get("fields_total", 0)

    if opts.show:
        for c, rep, runs in case_reports:
            if runs:
                print_actual(c, runs[0])

    if opts.json:
        summary = {
            "impl": opts.impl, "platform": PLATFORM, "elapsed": round(elapsed, 2),
            "total": sum(v for k, v in counts.items() if k != "skip"),
            "counts": dict(counts),
            "areas": {k: dict(v) for k, v in sorted(areas.items())},
            "tags": {k: dict(v) for k, v in sorted(tags.items())},
            "cases": [rep for _, rep, _ in case_reports],
        }
        json.dump(summary, sys.stdout, indent=1, ensure_ascii=False, default=str)
        sys.stdout.write("\n")
    else:
        for c, rep, runs in case_reports:
            st = rep["status"]
            if st in ("pass", "recorded") and not opts.verbose:
                continue
            extra = ""
            if "fields_total" in rep and st != "error":
                extra = f" ({rep['fields_passed']}/{rep['fields_total']} fields)"
            print(f"{st.upper():8} {c.id}{extra}")
            if rep.get("message"):
                for line in str(rep["message"]).rstrip().splitlines():
                    print(f"         {line}")
            for m in rep.get("mismatches", [])[:opts.max_diff]:
                print(f"         {m['path']}:")
                print(f"           expected {cmp.short(m['expected'])}")
                print(f"           actual   {cmp.short(m['actual'])}")
            if len(rep.get("mismatches", [])) > opts.max_diff:
                print(f"         ... {len(rep['mismatches']) - opts.max_diff} more")
        if areas and not record_mode:
            print(f"\n{'area':<14} {'cases':>11} {'fields':>8}")
            for a, v in sorted(areas.items()):
                fpct = 100.0 * v["fields_passed"] / v["fields_total"] if v["fields_total"] else 0
                print(f"{a:<14} {v['passed']:>5}/{v['total']:<5} {fpct:7.1f}%")
        total = sum(v for k, v in counts.items() if k != "skip")
        good = counts["pass"] + counts["recorded"]
        parts = [f"{k} {v}" for k, v in sorted(counts.items())]
        print(f"\n{good}/{total} ok  [{', '.join(parts)}]  impl={opts.impl} repeat={opts.repeat} "
              f"in {elapsed:.1f}s")

    if opts.coverage:
        coverage.report(covdir, driver, os.path.join(REF, "src"), opts.coverage_html, opts.json)

    bad = counts["fail"] + counts["error"] + counts["flaky"]
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
