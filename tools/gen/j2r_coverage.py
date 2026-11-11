#!/usr/bin/env python3
"""Coverage of the Lucene core test classes by Rucene's bound suites (sprint 9).

Usage: j2r_coverage.py [package-prefix] [--missing]

For every `Test*.java` under Lucene's core test tree, counts the declared
`public void test*()` methods and the `test("<name>", ...)` bindings of the
Rust class definition whose Java name (`"org.apache.lucene.…"`) matches.
Inherited methods are not counted on either side. `J2R` markers still present
in the Rust file are reported as untranslated.

Known limit: a class bound through a generic base or a macro that builds its
Java name at runtime (`concat!`, `format!`) is reported as unbound; check such
reports against the Rust tree before translating.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

JAVA = Path("/tmp/lucene1050/lucene/core/src/test")
RUST = [Path("crates"), Path("tests")]
COMMENT_RE = re.compile(r"//[^\n]*|/\*.*?\*/", re.S)
TEST_RE = re.compile(r"^\s*public\s+(?:final\s+)?void\s+(test\w*)\s*\(\s*\)", re.M)


def rust_index():
    by_class = defaultdict(lambda: {"methods": set(), "todo": 0, "files": set()})
    for root in RUST:
        for f in root.rglob("*.rs"):
            text = f.read_text(errors="replace")
            names = set(re.findall(r'"(org\.apache\.lucene\.(?:\w+\.)*Test\w+)"', text))
            for name in names:
                entry = by_class[name]
                entry["files"].add(str(f))
                # any binding form: test(..), t(..), Method::new(..), test_closure(..)
                entry["methods"].update(re.findall(r'"(test\w*)"', text))
                entry["todo"] += text.count('todo!("J2R')
    return by_class


def main() -> None:
    prefix = next((a for a in sys.argv[1:] if not a.startswith("--")), "")
    show_missing = "--missing" in sys.argv
    rust = rust_index()
    totals = defaultdict(lambda: [0, 0, 0, 0])  # classes, bound, methods, translated
    for java in sorted(JAVA.rglob("Test*.java")):
        rel = java.relative_to(JAVA).with_suffix("")
        fq = ".".join(rel.parts)
        if not fq.startswith("org.apache.lucene." + prefix):
            continue
        pkg = ".".join(rel.parts[3:-1]) or "(root)"
        methods = set(TEST_RE.findall(COMMENT_RE.sub("", java.read_text(errors="replace"))))
        entry = rust.get(fq)
        done = (methods & entry["methods"]) if entry else set()
        if entry and entry["todo"]:
            done = set()
        t = totals[pkg]
        t[0] += 1
        t[1] += 1 if entry and (done or not methods) else 0
        t[2] += len(methods)
        t[3] += len(done)
        if show_missing and (not entry or methods - done):
            missing = sorted(methods - done)
            state = "unbound" if not entry else f"{len(missing)} missing"
            print(f"{fq}: {state}" + (f" {missing}" if entry and missing else ""))
    print(f"{'package':28} {'classes':>8} {'bound':>7} {'methods':>8} {'ported':>7}")
    all_t = [0, 0, 0, 0]
    for pkg, t in sorted(totals.items()):
        print(f"{pkg:28} {t[0]:8} {t[1]:7} {t[2]:8} {t[3]:7}")
        all_t = [a + b for a, b in zip(all_t, t)]
    print(f"{'TOTAL':28} {all_t[0]:8} {all_t[1]:7} {all_t[2]:8} {all_t[3]:7}")


if __name__ == "__main__":
    main()
