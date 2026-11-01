#!/usr/bin/env python3
"""Compare two `./rlt --json` summaries: regressions, improvements, score.

    ./rlt --impl rust --json > /tmp/before.json
    ... change things ...
    ./rlt --impl rust --json > /tmp/after.json
    python3 impl/tools/compare.py /tmp/before.json /tmp/after.json [-a AREA]
"""
import argparse, json

ap = argparse.ArgumentParser()
ap.add_argument("before"); ap.add_argument("after")
ap.add_argument("-a", "--area", action="append", help="only these areas")
args = ap.parse_args()
load = lambda p: json.load(open(p))
A, B = load(args.before), load(args.after)
a = {c["id"]: c for c in A["cases"]}
b = {c["id"]: c for c in B["cases"]}
def keep(c): return not args.area or c["area"] in args.area
reg = [k for k, c in b.items() if keep(c) and a.get(k, {}).get("status") == "pass" and c["status"] != "pass"]
imp = [k for k, c in b.items() if keep(c) and a.get(k, {}).get("status") != "pass" and c["status"] == "pass"]
for k in reg: print("REGRESSED", k)
for k in imp: print("FIXED    ", k)
print(f"regressed {len(reg)}  fixed {len(imp)}  score {A['score']['score']} -> {B['score']['score']}")
