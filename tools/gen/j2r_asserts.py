#!/usr/bin/env python3
"""Map Lucene core `assert` statements to Rucene (sprint 9, tasks #364-#369).

Usage (from the repository root):
    j2r_asserts.py <package> [--out <file.md>]

`<package>` is a Lucene core package path under `org/apache/lucene/`, such as
`index`, `search`, `util`, `codecs` (sub-packages included).

For every Java source file of the package it lists each `assert` statement
(file:line, enclosing method, condition and message), finds the Rust file
that ports the class (a `//!`/`///` doc naming `<Class>.java`, or the file
named after the class), counts the `assert!`/`debug_assert!` family in the
Rust function of the same name, and marks the method:

* `ok`       - the Rust function has at least as many asserts as Java;
* `MISSING`  - fewer: each Java assert of that method is listed, to be ported
               at the equivalent point (same condition and message);
* `NO FN`    - no Rust function of that name in the port (the method may be
               folded elsewhere: locate it, or record why no counterpart
               applies);
* `NO PORT`  - no Rust file ports the class.

The pairing is by method and count; the translator confirms each pair by
reading both sides, and records the final map in the task.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

JAVA_ROOT = Path("/tmp/lucene1050/lucene/core/src/java/org/apache/lucene")
RUST_ROOT = Path("src")
KEYWORDS = {"if", "for", "while", "switch", "catch", "synchronized", "return", "new", "else", "try"}
RUST_ASSERT = re.compile(r"\b(?:debug_)?assert(?:_eq|_ne)?!\s*\(")


def snake(name: str) -> str:
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


def blank(src: str, spans):
    out = list(src)
    for a, b in spans:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)


def strip_java(src: str) -> str:
    spans = [m.span() for m in re.finditer(
        r'//[^\n]*|/\*.*?\*/|"""(?:.|\n)*?"""|"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])+\'', src, re.S)]
    return blank(src, spans)


def strip_rust(src: str) -> str:
    spans = [m.span() for m in re.finditer(
        r'//[^\n]*|/\*.*?\*/|r(#*)".*?"\1|b?"(?:\\.|[^"\\])*"|b?\'(?:\\.|[^\'\\\n])\'', src, re.S)]
    return blank(src, spans)


def match_brace(clean: str, i: int) -> int:
    depth = 0
    for j in range(i, len(clean)):
        if clean[j] == "{":
            depth += 1
        elif clean[j] == "}":
            depth -= 1
            if depth == 0:
                return j
    return len(clean) - 1


def line_of(src: str, idx: int) -> int:
    return src.count("\n", 0, idx) + 1


def java_methods(clean: str):
    """[(name, start, end)] of method and constructor bodies."""
    out = []
    for m in re.finditer(r"\b(\w+)\s*\([^;{}()]*(?:\([^()]*\)[^;{}()]*)*\)\s*(?:throws\s+[\w.,\s]+)?\{", clean):
        name = m.group(1)
        if name in KEYWORDS:
            continue
        before = clean[max(0, m.start() - 200):m.start()]
        if re.search(r"(?:->|=|\(|,|return)\s*$", before) or re.search(r"\bnew\s+$", before):
            continue
        start = m.end() - 1
        out.append((name, start, match_brace(clean, start)))
    return out


def java_asserts(path: Path):
    src = path.read_text(errors="replace")
    clean = strip_java(src)
    methods = java_methods(clean)
    found = []
    for m in re.finditer(r"(?<![\w.])assert\b(?!\s*\()", clean):
        end = clean.find(";", m.end())
        stmt = " ".join(src[m.start():end + 1].split())
        owner = "<class>"
        best = None
        for name, a, b in methods:
            if a < m.start() < b and (best is None or a > best[1]):
                best = (name, a, b)
        if best:
            owner = best[0]
        found.append((line_of(src, m.start()), owner, stmt))
    return found


def rust_index():
    """Java class name -> Rust files that port it."""
    idx = defaultdict(set)
    for f in RUST_ROOT.rglob("*.rs"):
        text = f.read_text(errors="replace")
        for cls in set(re.findall(r"\b(\w+)\.java\b", text)):
            idx[cls].add(f)
        idx["@file:" + f.stem].add(f)
    return idx


def rust_fn_asserts(files, fn_name):
    """(found, count) of asserts inside every `fn <fn_name>` of the files."""
    total, found = 0, False
    for f in files:
        src = f.read_text(errors="replace")
        clean = strip_rust(src)
        cut = re.search(r"#\[cfg\(test\)\]\s*mod\s+\w+", clean)
        limit = cut.start() if cut else len(clean)
        for m in re.finditer(r"\bfn\s+" + re.escape(fn_name) + r"\b[^{;]*\{", clean[:limit]):
            found = True
            a = m.end() - 1
            b = match_brace(clean, a)
            total += len(RUST_ASSERT.findall(clean[a:b]))
    return found, total


def main() -> None:
    pkg = sys.argv[1]
    out = Path(sys.argv[sys.argv.index("--out") + 1]) if "--out" in sys.argv else None
    idx = rust_index()
    lines = [f"# Assert map: org.apache.lucene.{pkg.replace('/', '.')}", "",
             "| Java file:line | method | status | Rust port | Java assert |", "|---|---|---|---|---|"]
    tally = defaultdict(int)
    for java in sorted((JAVA_ROOT / pkg).rglob("*.java")):
        asserts = java_asserts(java)
        if not asserts:
            continue
        cls = java.stem
        files = sorted(idx.get(cls, set()) | idx.get("@file:" + snake(cls), set()))
        rel = java.relative_to(JAVA_ROOT.parent.parent.parent.parent)
        by_method = defaultdict(list)
        for ln, owner, stmt in asserts:
            by_method[owner].append((ln, stmt))
        for owner, items in by_method.items():
            if not files:
                status, port = "NO PORT", "-"
            else:
                fn = snake(cls) if owner == cls else ("new" if owner == cls else snake(owner))
                fn = "new" if owner == cls else fn
                has, n = rust_fn_asserts(files, fn)
                port = ", ".join(str(f) for f in files[:2])
                if not has:
                    status = "NO FN"
                elif n >= len(items):
                    status = "ok"
                else:
                    status = f"MISSING ({n}/{len(items)})"
            key = status.split(" (")[0]
            tally[key] += len(items)
            for ln, stmt in items:
                lines.append(f"| {rel}:{ln} | {owner} | {status} | {port} | `{stmt.replace('|', '\\|')}` |")
    summary = ["", "| status | Java asserts |", "|---|---|"] + [f"| {k} | {v} |" for k, v in sorted(tally.items())]
    report = "\n".join(lines[:4] + summary[1:] + [""] + lines[4:]) if False else "\n".join(lines + summary)
    if out:
        out.write_text(report + "\n")
    print("\n".join(summary[1:]))
    print(f"total: {sum(tally.values())}")


if __name__ == "__main__":
    main()
