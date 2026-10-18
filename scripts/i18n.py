#!/usr/bin/env python3
"""Copies the Beeper Desktop translations of the UI strings to crates/beeper-ui/assets/i18n.

Usage: scripts/i18n.py <the extracted app.asar of Beeper Desktop>

Each entry has the plural forms of a translation. A string that Desktop translates only with a context gets a plain
entry when all of its contexts have the same translation, and keeps its context entries otherwise. The script lists the
strings without a plain entry in some language, which need `trx` or `trnx`.
"""
import glob
import json
import re
import sys

js = open(glob.glob(f"{sys.argv[1]}/build/renderer/matrix-util-*.js")[0], encoding="utf-8").read()
rust = "".join(open(f, encoding="utf-8").read() for f in glob.glob("crates/*/src/*.rs"))
unescape = {"\\": "\\", '"': '"', "n": "\n"}
tokens = re.finditer(r'//[^\n]*|r(#*)".*?"\1|\'(?:\\.|[^\\\'\n])\'|"((?:[^"\\]|\\.)*)"', rust, re.S)
used = {re.sub(r"\\(.)", lambda e: unescape.get(e.group(1), e.group(0)), t[2]) for t in tokens if t[2] is not None}
catalogs = {}
for m in re.finditer(r'JSON\.parse\(`(\{"":\{"domain":"beeper-desktop","lang":"([^"]+)".*?)`\)', js, re.S):
    body = re.sub(r"\\(.)", lambda e: e.group(1) if e.group(1) in "\\`$" else e.group(0), m.group(1), flags=re.S)
    catalog = {k: [f.replace("&nbsp;", "\xa0") for f in v] for k, v in json.loads(body).items() if k and all(v)}
    catalogs[m.group(2)] = {k: v for k, v in catalog.items() if k.split("\x04")[-1] in used}
variants = {}
for lang, table in catalogs.items():
    for k, v in table.items():
        variants.setdefault((lang, k.split("\x04")[-1]), set()).add(tuple(v))
ambiguous = {s for (_, s), vs in variants.items() if len(vs) > 1}
need = {s for (_, s) in variants if s in ambiguous and any(s not in t for t in catalogs.values())}
for lang, table in catalogs.items():
    table = {k: v for k, v in table.items() if "\x04" not in k or k.split("\x04")[1] in ambiguous}
    table |= {s: list(vs.pop()) for (l, s), vs in variants.items() if l == lang and s not in ambiguous}
    with open(f"crates/beeper-ui/assets/i18n/{lang}.json", "w", encoding="utf-8") as f:
        entries = (json.dumps(k, ensure_ascii=False) + ":" + json.dumps(v, ensure_ascii=False) for k, v in sorted(table.items()))
        f.write("{\n" + ",\n".join(entries) + "\n}\n")
print("Strings that need a context:", sorted(need))
