#!/usr/bin/env python3
"""Test-class census of the sprint-17 modules against their Rust ports.

For each module (``memory``, ``facet``, ``highlighter``, ``spatial3d``,
``spatial-extras``, ``benchmark``) the script counts, from the Lucene 10.5.0
clone, the JUnit test classes and test methods exactly as Lucene's build runs
them, and matches each one against the Rust port under
``crates/rucene-<module>/tests``.

What counts as a Java test (Lucene 10.5.0):

* **Classes.** ``gradle/testing/defaults-tests.gradle:202-204`` disables class
  scanning and runs ``include '**/Test*.class'`` / ``exclude '**/*$*'``: a
  top-level class whose simple name starts with ``Test``. Abstract classes and
  classes without a runnable method are not suites.
* **Methods.** ``LuceneTestCase`` runs ``LuceneJUnit3MethodProvider`` (a method
  named ``test*``, ``public``, not ``static``, no parameters, leaf of its
  override chain) and the ``JUnit4MethodProvider`` (``@Test``). Inherited methods
  count for every concrete subclass; an override counts once, with the
  annotations of the overriding (leaf) method.
* ``@Nightly``/``@Weekly``/``@Monster``/``@Slow``/``@AwaitsFix``/``@BadApple``/
  ``@Ignore`` (method or class level) are counted and reported separately; they
  are still bound methods. ``@Repeat`` and ``@ParametersFactory`` constructors
  are noted, because they multiply the runs of a bound method.

How a Rust port is matched (the convention of the sprint-17 test ports): every
ported suite is a ``Class`` built under its Java fully qualified name, written
as a string literal (``class("org.apache.lucene....TestX", ...)``,
``Class::builder("...")``, ``const NAME: &str = "..."``), and every ported
method is registered under its Java name as a string literal
(``test("testFoo", ...)``, ``Method::new("testFoo")``). A Java class is ported
when its FQN literal appears in a Rust test file of the crate; a Java method is
ported when its name literal appears in a Rust file that binds the class or the
ancestor that declares the method. Comments are blanked first, so a method
named only in documentation does not count.

Usage::

    tools/census/sprint17_tests.py [--lucene /tmp/lucene1050/lucene]
                                   [--json OUT] [MODULE ...]

Exit status 0 when every runnable Java class and method has a Rust port in the
modules examined, 1 otherwise.
"""

import argparse
import json
import os
import re
import sys

MODULES = ["memory", "facet", "highlighter", "spatial3d", "spatial-extras", "benchmark"]

GROUP_ANNOTATIONS = {"Nightly", "Weekly", "Monster", "Slow", "AwaitsFix", "BadApple", "Ignore"}


# --------------------------------------------------------------------------
# Java side
# --------------------------------------------------------------------------


def strip_java(src: str) -> str:
    """Blank comments, strings and char literals, preserving length.

    One left-to-right scan: the four token classes exclude each other in source
    order (same algorithm as ``tools/kg/javatext.py``, copied so this script
    does not depend on the knowledge-graph tooling).
    """
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == "/" and i + 1 < n and src[i + 1] == "*":
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("".join("\n" if ch == "\n" else " " for ch in src[i:j]))
            i = j
        elif c == '"':
            if src.startswith('"""', i):
                j = src.find('"""', i + 3)
                j = n if j < 0 else j + 3
            else:
                j = i + 1
                while j < n and src[j] != '"':
                    j += 2 if src[j] == "\\" else 1
                j += 1
            out.append('"' + " " * (j - i - 2) + '"' if j - i >= 2 else " " * (j - i))
            i = j
        elif c == "'":
            j = i + 1
            while j < n and src[j] != "'":
                j += 2 if src[j] == "\\" else 1
            j += 1
            out.append(" " * (j - i))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


ANNOT = r"@[A-Za-z_][\w.]*(?:\s*\([^()]*(?:\([^()]*\)[^()]*)*\))?"
MODIFIER = r"(?:public|protected|private|static|final|abstract|synchronized|strictfp|default)"
PACKAGE_RE = re.compile(r"^\s*package\s+([\w.]+)\s*;", re.M)
IMPORT_RE = re.compile(r"^\s*import\s+(?:static\s+)?([\w.]+)\s*;", re.M)
TYPE_RE = re.compile(
    r"(?P<head>(?:(?:" + ANNOT + r"|" + MODIFIER + r")\s+)*)"
    r"(?P<kind>class|interface|enum|record)\s+(?P<name>[A-Za-z_$][\w$]*)"
)
EXTENDS_RE = re.compile(r"^\s*(?:<[^{]*?>\s*)?extends\s+([\w.]+)")
METHOD_RE = re.compile(
    r"(?P<head>(?:(?:" + ANNOT + r"|" + MODIFIER + r")\s+)*)"
    r"(?:<[^>]*>\s+)?void\s+(?P<name>[A-Za-z_$][\w$]*)\s*\((?P<params>[^)]*)\)"
)
CTOR_PARAMS_FACTORY_RE = re.compile(r"@ParametersFactory\b")
REPEAT_RE = re.compile(r"@Repeat\b")
ASSUME_RE = re.compile(r"\bassume(?:True|False|NotNull|NoException|That)?\s*\(")


def annotations_of(head: str):
    return {a.split(".")[-1] for a in re.findall(r"@([A-Za-z_][\w.]*)", head)}


def modifiers_of(head: str):
    stripped = re.sub(ANNOT, " ", head)
    return set(re.findall(MODIFIER, stripped))


def depth_map(text: str):
    """Brace depth at every offset of the stripped source."""
    depths = [0] * (len(text) + 1)
    d = 0
    for i, c in enumerate(text):
        depths[i] = d
        if c == "{":
            d += 1
        elif c == "}":
            d -= 1
    depths[len(text)] = d
    return depths


def parse_java_file(path: str):
    """The top-level type of ``path``: name, package, extends, methods."""
    raw = open(path, encoding="utf-8", errors="replace").read()
    text = strip_java(raw)
    depths = depth_map(text)
    pkg_m = PACKAGE_RE.search(text)
    package = pkg_m.group(1) if pkg_m else ""
    imports = {imp.split(".")[-1]: imp for imp in IMPORT_RE.findall(text)}

    top = None
    for m in TYPE_RE.finditer(text):
        if depths[m.start("kind")] == 0:
            top = m
            break
    if top is None:
        return None
    name = top.group("name")
    head = top.group("head")
    body_open = text.find("{", top.end())
    decl_tail = text[top.end() : body_open]
    ext = EXTENDS_RE.search(decl_tail)
    extends = ext.group(1) if ext else ""

    methods = {}
    helpers = []
    parameterized = False
    for m in METHOD_RE.finditer(text, body_open):
        if depths[m.start("name")] != 1:
            continue
        mname = m.group("name")
        mods = modifiers_of(m.group("head"))
        annots = annotations_of(m.group("head"))
        no_args = m.group("params").strip() == ""
        junit3 = (
            mname.startswith("test") and "public" in mods and "static" not in mods and no_args
        )
        junit4 = "Test" in annots and "static" not in mods
        methods[mname] = {
            "test": bool(junit3 or junit4),
            "annotations": sorted(annots & (GROUP_ANNOTATIONS | {"Repeat"})),
            "abstract": "abstract" in mods,
        }
        if mname.startswith("test") and not no_args:
            helpers.append(mname)
    if CTOR_PARAMS_FACTORY_RE.search(text):
        parameterized = True

    return {
        "name": name,
        "package": package,
        "fqn": f"{package}.{name}" if package else name,
        "kind": top.group("kind"),
        "abstract": "abstract" in modifiers_of(head),
        "annotations": sorted(annotations_of(head) & GROUP_ANNOTATIONS),
        "extends": extends,
        "imports": imports,
        "methods": methods,
        "helpers": sorted(set(helpers)),
        "parameterized": parameterized,
        "assumes": len(ASSUME_RE.findall(text)),
        "file": path,
    }


def scan_java(roots):
    classes = {}
    for root in roots:
        for dirpath, _dirs, files in os.walk(root):
            for fn in sorted(files):
                if fn.endswith(".java"):
                    c = parse_java_file(os.path.join(dirpath, fn))
                    if c is not None:
                        classes[c["fqn"]] = c
    return classes


def resolve_super(c, classes):
    """The FQN of ``c``'s superclass, when it is a scanned class."""
    ext = c["extends"]
    if not ext:
        return None
    if ext in classes:
        return ext
    simple = ext.split(".")[-1]
    candidates = []
    if simple in c["imports"]:
        candidates.append(c["imports"][simple])
    candidates.append(f"{c['package']}.{simple}")
    for cand in candidates:
        if cand in classes:
            return cand
    # Nested-class or wildcard imports: unique simple-name match only.
    matches = [f for f, k in classes.items() if k["name"] == simple]
    return matches[0] if len(matches) == 1 else None


def effective_methods(fqn, classes):
    """Runnable methods of ``fqn``: name -> (declaring FQN, annotations)."""
    chain = []
    cur = fqn
    while cur is not None and cur not in chain:
        chain.append(cur)
        cur = resolve_super(classes[cur], classes)
    result = {}
    shadowed = set()
    for owner in chain:  # leaf first
        for mname, info in classes[owner]["methods"].items():
            if mname in shadowed:
                continue
            shadowed.add(mname)
            if info["test"] and not info["abstract"]:
                result[mname] = (owner, info["annotations"])
    return result, chain


# --------------------------------------------------------------------------
# Rust side
# --------------------------------------------------------------------------


def rust_string_literals(src: str):
    """String literal contents of Rust source, comments skipped."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif c == "/" and src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            i = j
        elif c in "rb" and re.match(r"(?:br|r)#*\"", src[i:i + 260]) and (
            i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")
        ):
            m = re.match(r"(?:br|r)(#*)\"", src[i:])
            hashes = m.group(1)
            start = i + m.end()
            end = src.find('"' + hashes, start)
            end = n if end < 0 else end
            out.append(src[start:end])
            i = end + 1 + len(hashes)
        elif c == '"':
            j = i + 1
            buf = []
            while j < n and src[j] != '"':
                if src[j] == "\\":
                    buf.append(src[j : j + 2])
                    j += 2
                else:
                    buf.append(src[j])
                    j += 1
            out.append("".join(buf))
            i = j + 1
        elif c == "'":
            m = re.match(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'])'", src[i:])
            i += m.end() if m else 1
        else:
            i += 1
    return out


FQN_LITERAL_RE = re.compile(r"^(?:org|com)\.[\w.]+\.[A-Z][\w$]*$")


def scan_rust(tests_dir: str):
    """FQN literal -> files binding it; file -> set of string literals."""
    bindings = {}
    literals = {}
    if not os.path.isdir(tests_dir):
        return bindings, literals
    for dirpath, _dirs, files in os.walk(tests_dir):
        for fn in sorted(files):
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dirpath, fn)
            lits = set(rust_string_literals(open(path, encoding="utf-8").read()))
            literals[path] = lits
            for lit in lits:
                if FQN_LITERAL_RE.match(lit):
                    bindings.setdefault(lit, set()).add(path)
    return bindings, literals


# --------------------------------------------------------------------------
# Census
# --------------------------------------------------------------------------


def census_module(module, lucene, repo):
    test_root = os.path.join(lucene, module, "src", "test")
    # Bases may live in the module's tests or in the test framework.
    roots = [test_root, os.path.join(lucene, "test-framework", "src", "java")]
    classes = scan_java(roots)
    tests_dir = os.path.join(repo, "crates", f"rucene-{module}", "tests")
    bindings, literals = scan_rust(tests_dir)

    module_fqns = sorted(
        f for f, c in classes.items() if c["file"].startswith(test_root + os.sep)
    )
    runnable = []
    not_suites = []
    for fqn in module_fqns:
        c = classes[fqn]
        if c["kind"] != "class" or not c["name"].startswith("Test"):
            continue
        methods, chain = effective_methods(fqn, classes)
        if c["abstract"] or not methods:
            not_suites.append(fqn)
            continue
        runnable.append((fqn, methods, chain))

    rows = []
    missing_classes, missing_methods = [], []
    totals = {"java_classes": 0, "java_methods": 0, "ported_classes": 0, "ported_methods": 0}
    flagged, notes = [], []
    for fqn, methods, chain in runnable:
        c = classes[fqn]
        totals["java_classes"] += 1
        totals["java_methods"] += len(methods)
        bound = fqn in bindings
        if bound:
            totals["ported_classes"] += 1
        else:
            missing_classes.append(fqn)
        ported = 0
        miss = []
        for mname, (owner, annots) in sorted(methods.items()):
            files = set()
            for anc in chain[: chain.index(owner) + 1]:
                files |= bindings.get(anc, set())
            if bound and any(mname in literals[f] for f in files):
                ported += 1
            else:
                miss.append(f"{fqn}#{mname}")
            groups = sorted(set(annots) & GROUP_ANNOTATIONS | set(c["annotations"]))
            if groups:
                flagged.append(f"{fqn}#{mname} @{' @'.join(groups)}")
            if "Repeat" in annots:
                notes.append(f"{fqn}#{mname}: @Repeat (one bound method, several runs)")
        totals["ported_methods"] += ported
        missing_methods.extend(miss)
        if c["parameterized"]:
            notes.append(f"{fqn}: @ParametersFactory (every method runs per parameter set)")
        for h in c["helpers"]:
            notes.append(
                f"{fqn}#{h}(..): test-named helper with parameters, not run by JUnit, not counted"
            )
        if c["assumes"]:
            notes.append(f"{fqn}: {c['assumes']} assume*() site(s) (skips at run time, still bound)")
        rows.append(
            {
                "class": fqn,
                "java_methods": len(methods),
                "ported_methods": ported,
                "rust_files": sorted(os.path.relpath(f, repo) for f in bindings.get(fqn, ())),
            }
        )

    java_simple = {classes[f]["fqn"] for f, _m, _c in runnable}
    extra = sorted(
        f
        for f in bindings
        if f.rsplit(".", 1)[-1].startswith("Test")
        and "$" not in f  # nested/anonymous classes of a ported suite
        and f not in java_simple
        and f not in classes
    )
    return {
        "module": module,
        **totals,
        "not_suites": not_suites,
        "missing_classes": missing_classes,
        "missing_methods": missing_methods,
        "group_annotated": flagged,
        "notes": notes,
        "rust_only_test_fqns": extra,
        "classes": rows,
    }


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--lucene", default="/tmp/lucene1050/lucene")
    ap.add_argument("--repo", default=os.path.normpath(os.path.join(here, "..", "..")))
    ap.add_argument("--json", help="write the full census as JSON to this path")
    ap.add_argument("--verbose", action="store_true", help="print the per-class table")
    ap.add_argument("modules", nargs="*", default=MODULES)
    args = ap.parse_args()

    for m in args.modules:
        if m not in MODULES:
            ap.error(f"unknown module {m!r}; expected one of {MODULES}")

    results = [census_module(m, args.lucene, args.repo) for m in args.modules]
    ok = True
    header = f"{'module':<16}{'java cls':>9}{'java mth':>9}{'port cls':>9}{'port mth':>9}{'missing':>9}"
    print(header)
    print("-" * len(header))
    for r in results:
        missing = len(r["missing_classes"]) + len(r["missing_methods"])
        print(
            f"{r['module']:<16}{r['java_classes']:>9}{r['java_methods']:>9}"
            f"{r['ported_classes']:>9}{r['ported_methods']:>9}{missing:>9}"
        )
    tot = {k: sum(r[k] for r in results) for k in ("java_classes", "java_methods", "ported_classes", "ported_methods")}
    print("-" * len(header))
    print(
        f"{'total':<16}{tot['java_classes']:>9}{tot['java_methods']:>9}"
        f"{tot['ported_classes']:>9}{tot['ported_methods']:>9}"
    )
    for r in results:
        print(f"\n== {r['module']}")
        if args.verbose:
            for row in r["classes"]:
                print(
                    f"  {row['class']}: {row['ported_methods']}/{row['java_methods']}"
                    f"  {', '.join(row['rust_files']) or '-'}"
                )
        for key, title in (
            ("missing_classes", "MISSING classes"),
            ("missing_methods", "MISSING methods"),
            ("group_annotated", "group-annotated methods (bound, filtered at run time)"),
            ("notes", "explained differences"),
            ("not_suites", "Test* types that are not suites (abstract / no runnable method)"),
            ("rust_only_test_fqns", "Rust Test* FQN literals with no Java class"),
        ):
            if r[key]:
                print(f"  {title} ({len(r[key])}):")
                for item in r[key]:
                    print(f"    {item}")
        if r["missing_classes"] or r["missing_methods"]:
            ok = False
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(results, f, indent=2)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
