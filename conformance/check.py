#!/usr/bin/env python3
"""Header/ABI conformance tier.

    conformance/check.py --record            # GNU headers -> conformance/abi.<platform>.json
    conformance/check.py [--impl ref|rust]   # our headers (+ implementation library) vs. record

Recording needs the GNU headers ($LINEREAD_REF/gnu-include, created by
ref/build.sh).  Checking needs only our headers, a C compiler, nm and the
implementation's static library.

Items checked (each is scored separately):
  decl:NAME          every function, variable, typedef, struct/union/enum
                     declared by the headers, with its full type (clang AST)
  sizeof(T) alignof(T) offsetof(T,f)   layouts of all object types
  enum:NAME          enumerator values
  macro:NAME         integer/string value of object-like macros, presence of
                     empty macros, token text of the rest
  macro:NAME(c)      character macros evaluated for c in -1..256
  symbol:NAME        every declared function/variable is defined by the library
  link               a program referencing every symbol links
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REF = os.environ.get("LINEREAD_REF", os.path.join(os.path.dirname(ROOT), "lineread-ref"))
GNU_INC = os.path.join(REF, "gnu-include")
OUR_INC = os.path.join(ROOT, "include")
PLAT = f"{os.uname().sysname.lower()}-{os.uname().machine}"
CC = os.environ.get("CC", "clang")

PRELUDE = """#include <stdio.h>
#include <readline/rlstdc.h>
#include <readline/rlconf.h>
#include <readline/rltypedefs.h>
#include <readline/chardefs.h>
#include <readline/keymaps.h>
#include <readline/tilde.h>
#include <readline/readline.h>
#include <readline/history.h>
"""
BASELINE = "#include <stdio.h>\n#include <ctype.h>\n#include <string.h>\n#include <strings.h>\n#include <time.h>\n#include <stdlib.h>\n"

CHAR_MACROS = {
    "CTRL", "META", "UNMETA", "UNCTRL", "CTRL_CHAR", "META_CHAR", "whitespace",
    "ISALNUM", "ISALPHA", "ISDIGIT", "ISLOWER", "ISPRINT", "ISUPPER", "ISXDIGIT",
    "_rl_lowercase_p", "_rl_uppercase_p", "_rl_digit_p", "_rl_alphabetic_p",
    "_rl_pure_alphabetic", "_rl_to_upper", "_rl_to_lower", "_rl_digit_value",
    "_rl_isident", "ISOCTAL", "OCTVALUE", "HEXVALUE", "NON_NEGATIVE",
    "IN_CTYPE_DOMAIN", "isxdigit",
}


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


# ---------------------------------------------------------------- surface

def _type_of(node):
    t = node.get("type", {})
    return t.get("desugaredQualType") and t.get("qualType") or t.get("qualType")


def ast_surface(inc: str) -> dict:
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "s.c")
        with open(src, "w") as f:
            f.write(PRELUDE)
        out = run([CC, "-fsyntax-only", "-Xclang", "-ast-dump=json", f"-I{inc}", src]).stdout
    tu = json.loads(out)
    cur = None
    surface = {}
    anon = 0
    for node in tu["inner"]:
        loc = node.get("loc", {})
        f = loc.get("file") or loc.get("spellingLoc", {}).get("file") or loc.get("expansionLoc", {}).get("file")
        if f:
            cur = f
        if not cur or "/readline/" not in cur.replace(os.sep, "/"):
            continue
        kind = node["kind"]
        name = node.get("name")
        if kind in ("FunctionDecl", "VarDecl", "TypedefDecl"):
            entry = {"kind": kind, "type": node.get("type", {}).get("qualType")}
            if kind == "VarDecl":
                entry["storage"] = node.get("storageClass", "")
        elif kind == "RecordDecl":
            if not node.get("completeDefinition"):
                continue
            fields = [[c.get("name"), c.get("type", {}).get("qualType")]
                      for c in node.get("inner", []) if c["kind"] == "FieldDecl"]
            entry = {"kind": f"{node.get('tagUsed', 'struct')}", "fields": fields}
            if not name:
                anon += 1
                name = f"<anon{anon}>"
            name = f"{node.get('tagUsed', 'struct')} {name}"
        elif kind == "EnumDecl":
            enumers = [c.get("name") for c in node.get("inner", []) if c["kind"] == "EnumConstantDecl"]
            entry = {"kind": "enum", "enumerators": enumers}
            name = f"enum {name}" if name else f"enum <{enumers[0] if enumers else '?'}>"
        else:
            continue
        surface[name] = entry
    return surface


# ---------------------------------------------------------------- macros

MACRO_RE = re.compile(r"^#define (\w+)(\([^)]*\))? ?(.*)$")


def macro_defs(inc: str | None) -> dict:
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "m.c")
        with open(src, "w") as f:
            f.write(PRELUDE if inc else BASELINE)
        args = [CC, "-E", "-dM", src] + ([f"-I{inc}"] if inc else [])
        out = run(args).stdout
    defs = {}
    for line in out.splitlines():
        m = MACRO_RE.match(line)
        if m:
            defs[m.group(1)] = (m.group(2), m.group(3).strip())
    return defs


def norm_text(s: str) -> str:
    s = re.sub(r"\s+", " ", s).strip()
    return re.sub(r" ?([()\[\],;*+\-|&~!<>=?:]) ?", r"\1", s)


def readline_macros(inc: str) -> dict:
    base = macro_defs(None)
    defs = macro_defs(inc)
    return {k: v for k, v in defs.items() if base.get(k) != v}


def _compiles(code: str, inc: str) -> bool:
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "p.c")
        with open(src, "w") as f:
            f.write(PRELUDE + code)
        r = subprocess.run([CC, "-fsyntax-only", "-Werror=int-conversion", f"-I{inc}", src],
                           capture_output=True, text=True)
        return r.returncode == 0


def classify_macro(name: str, params, body: str, inc: str) -> str:
    if params is not None:
        nparams = len([p for p in params[1:-1].split(",") if p.strip()])
        if name in CHAR_MACROS and nparams == 1:
            return "char"
        return "text"
    if body == "":
        return "defined"
    if _compiles(f"enum {{ probe_ = (int)(sizeof(char[({name}) ? 1 : 1])) }};\nstatic const long long v_ = ({name});", inc):
        return "int"
    if _compiles(f'static const char v_[] = {name};', inc):
        return "str"
    return "text"


# ---------------------------------------------------------------- probe

def c_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def probe_source(spec: dict) -> str:
    out = [PRELUDE, "#include <stddef.h>\n#include <stdalign.h>\n",
           "static int first_ = 1;\n",
           "static void key_(const char *k) { printf(\"%s\\n  \\\"\", first_ ? \"{\" : \",\"); first_ = 0;"
           " for (; *k; k++) { if (*k == '\"' || *k == '\\\\') putchar('\\\\'); putchar(*k); } printf(\"\\\": \"); }\n",
           "static void str_(const char *s) { putchar('\"'); for (; *s; s++) { unsigned char c = (unsigned char)*s;"
           " if (c == '\"' || c == '\\\\') printf(\"\\\\%c\", c); else if (c < 32 || c >= 127) printf(\"\\\\u%04x\", c);"
           " else putchar(c); } putchar('\"'); }\n",
           "int main(void) {\n"]
    for t in spec["types"]:
        out.append(f"  key_({c_str('sizeof(' + t + ')')}); printf(\"%zu\", sizeof({t}));\n")
        out.append(f"  key_({c_str('alignof(' + t + ')')}); printf(\"%zu\", alignof({t}));\n")
    for t, f in spec["fields"]:
        out.append(f"  key_({c_str('offsetof(' + t + ',' + f + ')')}); printf(\"%zu\", offsetof({t}, {f}));\n")
    for e in spec["enums"]:
        out.append(f"  key_({c_str('enum:' + e)}); printf(\"%lld\", (long long)({e}));\n")
    for name, kind in spec["macros"].items():
        if kind == "int":
            out.append(f"  key_({c_str('macro:' + name)}); printf(\"%lld\", (long long)({name}));\n")
        elif kind == "str":
            out.append(f"  key_({c_str('macro:' + name)}); str_({name});\n")
        elif kind == "char":
            out.append(f"  key_({c_str('macro:' + name + '(c)')}); printf(\"[\");"
                       f" for (int c = -1; c <= 256; c++) printf(\"%s%lld\", c == -1 ? \"\" : \",\", (long long)({name}(c)));"
                       f" printf(\"]\");\n")
    out.append("  printf(\"\\n}\\n\");\n  return 0;\n}\n")
    return "".join(out)


def run_probe(spec: dict, inc: str) -> dict:
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "probe.c")
        exe = os.path.join(td, "probe")
        with open(src, "w") as f:
            f.write(probe_source(spec))
        r = subprocess.run([CC, "-std=c11", "-w", f"-I{inc}", "-o", exe, src], capture_output=True, text=True)
        if r.returncode != 0:
            return {"_compile_error": r.stderr[-4000:]}
        return json.loads(run([exe]).stdout)


def spec_from_surface(surface: dict, macro_kinds: dict) -> dict:
    types, fields, enums = [], [], []
    for name, e in surface.items():
        if e["kind"] == "TypedefDecl":
            ty = e["type"] or ""
            # function typedefs have no size
            if re.search(r"\)\s*\(.*\)\s*$", ty) or re.match(r"^[\w\s\*]+\(.*\)$", ty):
                continue
            types.append(name)
        elif e["kind"] in ("struct", "union") and not name.split(" ", 1)[1].startswith("<"):
            types.append(name)
            for fname, _ in e["fields"]:
                if fname:
                    fields.append((name, fname))
        elif e["kind"] == "enum":
            if not name.endswith(">"):
                types.append(name)
            enums.extend(e["enumerators"])
    return {"types": types, "fields": fields, "enums": enums,
            "macros": {k: v for k, v in macro_kinds.items() if v in ("int", "str", "char")}}


# ---------------------------------------------------------------- symbols

def impl_libs(impl: str) -> list[str]:
    out = run(["make", "-s", "--no-print-directory", f"IMPL={impl}", "print-libs"], cwd=ROOT).stdout.split()
    return out


def defined_symbols(libs: list[str]) -> set[str]:
    syms = set()
    for lib in libs:
        r = subprocess.run(["nm", "-g", "--defined-only", lib], capture_output=True, text=True)
        for line in r.stdout.splitlines():
            parts = line.split()
            if len(parts) >= 3 and parts[1] in "TDBSRCVWtdbsrcvw" and parts[1].isupper():
                s = parts[2]
                if sys.platform == "darwin" and s.startswith("_"):
                    s = s[1:]
                syms.add(s)
    return syms


def link_check(symbols: list[str], surface: dict, libs: list[str],
               extra: list[str] | None = None) -> tuple[bool, str]:
    lines = [PRELUDE, "void *volatile sink_;\nint main(void) {\n"]
    for s in symbols:
        lines.append(f"  sink_ = (void *)&{s};\n")
    lines.append("  return 0;\n}\n")
    with tempfile.TemporaryDirectory() as td:
        src = os.path.join(td, "link.c")
        with open(src, "w") as f:
            f.write("".join(lines))
        termlib = os.environ.get("TERMLIB", "-lncurses").split()
        r = subprocess.run([CC, "-w", f"-I{OUR_INC}", "-o", os.path.join(td, "a.out"), src] + libs + termlib
                           + (extra or []),
                           capture_output=True, text=True)
        return r.returncode == 0, r.stderr[-3000:]


# ---------------------------------------------------------------- main

def record_path() -> str:
    return os.path.join(ROOT, "conformance", f"abi.{PLAT}.json")


def do_record() -> None:
    if not os.path.isdir(GNU_INC):
        raise SystemExit(f"{GNU_INC} missing; run ref/build.sh")
    surface = ast_surface(GNU_INC)
    macros = readline_macros(GNU_INC)
    with concurrent.futures.ThreadPoolExecutor(8) as ex:
        kinds = dict(zip(macros, ex.map(lambda kv: classify_macro(kv[0], kv[1][0], kv[1][1], GNU_INC), macros.items())))
    macro_rec = {}
    for name, (params, body) in sorted(macros.items()):
        macro_rec[name] = {"kind": kinds[name]}
        if params is not None:
            macro_rec[name]["params"] = norm_text(params)
        if kinds[name] == "text":
            macro_rec[name]["text"] = norm_text(body)
    spec = spec_from_surface(surface, kinds)
    values = run_probe(spec, GNU_INC)
    if "_compile_error" in values:
        raise SystemExit(values["_compile_error"])
    ref_syms = defined_symbols(impl_libs("ref"))
    symbols = {n: (n in ref_syms) for n, e in surface.items()
               if e["kind"] == "FunctionDecl" or (e["kind"] == "VarDecl" and e.get("storage") == "extern")}
    data = {"platform": PLAT, "surface": dict(sorted(surface.items())), "macros": macro_rec,
            "spec": {"types": spec["types"], "fields": spec["fields"], "enums": spec["enums"]},
            "values": values, "symbols": dict(sorted(symbols.items()))}
    with open(record_path(), "w") as f:
        json.dump(data, f, indent=1, sort_keys=False)
        f.write("\n")
    print(f"recorded {len(surface)} declarations, {len(macro_rec)} macros, {len(values)} values, "
          f"{len(symbols)} symbols -> {os.path.relpath(record_path(), ROOT)}")


def do_check(impl: str) -> list[dict]:
    path = record_path()
    if not os.path.exists(path):
        return [{"id": "conformance:record", "status": "error", "message": f"{os.path.relpath(path, ROOT)} missing"}]
    with open(path) as f:
        rec = json.load(f)
    items = []

    def item(iid, ok, exp=None, act=None):
        d = {"id": iid, "status": "pass" if ok else "fail"}
        if not ok:
            d["expected"], d["actual"] = exp, act
        items.append(d)

    surface = ast_surface(OUR_INC)
    for name, e in rec["surface"].items():
        item(f"decl:{name}", surface.get(name) == e, e, surface.get(name))
    for name in surface:
        if name not in rec["surface"]:
            item(f"decl:{name}", False, None, surface[name])

    ours = readline_macros(OUR_INC)
    for name, m in rec["macros"].items():
        if name not in ours:
            item(f"macro-defined:{name}", False, True, False)
            continue
        params, body = ours[name]
        ok = (params is None) == ("params" not in m) and (params is None or norm_text(params) == m["params"])
        if m["kind"] == "text":
            ok = ok and norm_text(body) == m["text"]
            item(f"macro:{name}", ok, m.get("text"), norm_text(body))
        elif m["kind"] == "defined":
            item(f"macro:{name}", ok and body == "", "", body)
        else:
            item(f"macro-defined:{name}", ok, m.get("params"), params)
    spec = dict(rec["spec"])
    spec["macros"] = {k: v["kind"] for k, v in rec["macros"].items()
                      if v["kind"] in ("int", "str", "char") and k in ours}
    values = run_probe(spec, OUR_INC)
    if "_compile_error" in values:
        items.append({"id": "probe:compile", "status": "fail", "message": values["_compile_error"]})
    else:
        for k, v in rec["values"].items():
            item(k, values.get(k) == v, v, values.get(k))

    try:
        libs = impl_libs(impl)
    except subprocess.CalledProcessError as e:
        libs = []
        items.append({"id": "symbols:libs", "status": "error", "message": e.stderr})
    missing_libs = [l for l in libs if not os.path.exists(l)]
    if libs and not missing_libs:
        syms = defined_symbols(libs)
        for name, in_ref in rec["symbols"].items():
            # a symbol the reference does not define is not required
            item(f"symbol:{name}", (name in syms) or not in_ref, True, name in syms)
        needed = [n for n, in_ref in rec["symbols"].items() if in_ref]
        extra = ["-fsanitize=address,undefined"] if impl.endswith("asan") else []
        if impl.startswith("rust") and sys.platform != "darwin":
            extra += ["-lpthread", "-ldl", "-lm"]
        ok, err = link_check(needed, rec["surface"], libs, extra)
        d = {"id": "link", "status": "pass" if ok else "fail"}
        if not ok:
            d["message"] = err
        items.append(d)
    elif missing_libs:
        items.append({"id": "symbols:libs", "status": "error", "message": f"missing {' '.join(missing_libs)}"})
    return items


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--record", action="store_true")
    ap.add_argument("--impl", default="ref")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    o = ap.parse_args()
    if o.record:
        do_record()
        return 0
    items = do_check(o.impl)
    if o.json:
        json.dump(items, sys.stdout, indent=1)
        print()
    else:
        for it in items:
            if it["status"] != "pass" or o.verbose:
                print(f"{it['status'].upper():5} {it['id']}")
                if "message" in it:
                    print("      " + it["message"].replace("\n", "\n      "))
                elif it["status"] != "pass":
                    print(f"      expected {json.dumps(it.get('expected'))[:300]}")
                    print(f"      actual   {json.dumps(it.get('actual'))[:300]}")
        ok = sum(1 for i in items if i["status"] == "pass")
        print(f"conformance: {ok}/{len(items)} pass")
    return 0 if all(i["status"] == "pass" for i in items) else 1


if __name__ == "__main__":
    sys.exit(main())
