#!/usr/bin/env python3
"""Generate the Rust skeleton of a Lucene core test class (sprint 9).

Usage (from the repository root):
    j2r_skeleton.py <Lucene Test*.java> <output .rs> [--target <dir name>]

The skeleton bounds the translation step: it carries everything the
translator needs, so that the Java file and the Rust tree need no exploring.

* The `//!` header: the Java source path, the class Javadoc, the Java base
  class with its inherited test methods and the Rust port of that base, and
  a pointer to the rules (`tools/gen/J2R_GUIDE.md`).
* `use` lines for every Lucene/JDK type the class names whose Rust path is
  proven (it already compiles in a ported test or in the crates); a comment
  table for the types that are only defined (path to check) or not ported
  (translate from the given Lucene source, as is).
* The helpers the class calls without a receiver (`atLeast`, `newDirectory`,
  `expectThrows`, `assertEquals`, ...), each with its Rust counterpart and
  where it is defined.
* `class_def()` binding every declared `public void test*()` method in Java's
  order, with Lucene's method annotations, and the runner lines for the
  target (printed on stdout).
* One function per test method: its Javadoc as `///`, its Java body as
  comments, and a `todo!("J2R ...")` marker.
* The other members (fields, helpers, setUp/tearDown, nested classes), each
  with its Javadoc, as comments.

A pass is complete only when `grep -rn 'todo!("J2R' crates/` finds nothing
and `tools/gen/j2r_coverage.py` reports every declared method bound.
"""

import re
import sys
from collections import Counter, defaultdict
from functools import lru_cache
from pathlib import Path

LUCENE = Path("/tmp/lucene1050/lucene")
CORE_TEST = LUCENE / "core/src/test"
RUST_ROOTS = [Path("src"), Path("crates"), Path("tests")]
CRATES = ("rucene", "rucene_test_framework", "randomizedtesting", "rucene_codecs")
TF = Path("crates/rucene-test-framework")
HELPER_SOURCES = [
    ("crate::core_support::{}", TF / "tests/core_support/mod.rs"),
    ("LuceneTestCase::{}", TF / "src/util/lucene_test_case.rs"),
    ("TestUtil::{}", TF / "src/util/test_util.rs"),
]
KNOWN_ANNOTATIONS = {"Nightly", "Monster", "Slow", "Weekly", "Ignore"}
ANNOTATION_PATH = "rucene_test_framework::util::lucene_test_case"
JDK_COMMON = set(
    "ArrayList List Arrays Collections Set HashSet HashMap Map Iterator Iterable Collection "
    "LinkedList ArrayDeque Deque Queue TreeMap TreeSet LinkedHashMap LinkedHashSet SortedMap "
    "SortedSet NavigableMap IdentityHashMap CopyOnWriteArrayList ConcurrentHashMap "
    "AtomicInteger AtomicLong AtomicBoolean AtomicReference CountDownLatch CyclicBarrier "
    "Semaphore ExecutorService Executors TimeUnit Future Callable Runnable Locale Objects "
    "Optional Comparator StringBuilder Files Paths Charset StandardCharsets Function Supplier "
    "Consumer Predicate BiFunction Stream Collectors Character Number".split()
)
HELPER_ALIASES = {"assertThrows": "expect_throws"}
JAVA_KEYWORDS = set(
    "if for while switch catch return new super this synchronized assert throw "
    "try else do case default instanceof".split()
)


# --------------------------------------------------------------------------
# Java parsing
# --------------------------------------------------------------------------

def snake(name: str) -> str:
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


def strip_code(src: str) -> str:
    """Blank comments and string/char literals (same length) for parsing."""
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif src.startswith('"""', i):
            j = src.find('"""', i + 3)
            j = n if j < 0 else j + 3
            for k in range(i + 1, j - 1):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif c in "\"'":
            j = i + 1
            while j < n and src[j] != c:
                j += 2 if src[j] == "\\" else 1
            for k in range(i + 1, min(j, n)):
                out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out)


def match_brace(clean: str, open_idx: int) -> int:
    depth = 0
    for i in range(open_idx, len(clean)):
        if clean[i] == "{":
            depth += 1
        elif clean[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    raise ValueError("unbalanced braces")


def members(src: str, clean: str, start: int, end: int):
    """Yield (kind, start, text, header, body span) for each class member."""
    i = start
    while i < end:
        while i < end and clean[i] in " \t\r\n;":
            i += 1
        if i >= end:
            break
        j = i
        while j < end and clean[j] not in "{;":
            if clean[j] == "(":
                depth = 0
                while j < end:
                    if clean[j] == "(":
                        depth += 1
                    elif clean[j] == ")":
                        depth -= 1
                        if depth == 0:
                            break
                    j += 1
            j += 1
        if j >= end:
            break
        if clean[j] == "{":
            k = match_brace(clean, j)
            header = clean[i:j]
            if "=" in header.split("(")[0] or re.match(r"\s*[;,)]", clean[k + 1:end]):
                tail = clean.index(";", k + 1) + 1
                yield ("field", i, src[i:tail], header, None)
                i = tail
                continue
            yield ("block", i, src[i:k + 1], header, (j, k))
            i = k + 1
        else:
            yield ("field", i, src[i:j + 1], clean[i:j], None)
            i = j + 1


def javadoc_before(src: str, idx: int) -> str:
    """The `/** ... */` comment that ends right before `idx`, as plain lines."""
    head = src[:idx].rstrip()
    if not head.endswith("*/"):
        return ""
    start = head.rfind("/**")
    if start < 0 or "*/" in head[start:-2]:
        return ""
    body = head[start + 3:-2]
    lines = [re.sub(r"^\s*\* ?", "", l).rstrip() for l in body.splitlines()]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    text = "\n".join(lines)
    text = re.sub(r"\{@(?:link|linkplain|code)\s+([^}]+)\}", r"`\1`", text)
    text = re.sub(r"</?(?:p|code|b|i|em|pre)>", "", text)
    return text


def comment(text: str, indent: str = "    ", mark: str = "//") -> str:
    lines = text.rstrip().splitlines()
    while lines and not lines[0].strip():
        lines.pop(0)
    base = min((len(l) - len(l.lstrip()) for l in lines if l.strip()), default=0)
    return "\n".join(f"{indent}{mark} {l[base:]}".rstrip() for l in lines)


def annotations(header: str):
    return re.findall(r"@(\w+)(\s*\([^)]*\))?", header)


# --------------------------------------------------------------------------
# Rust / Lucene indexes
# --------------------------------------------------------------------------

def expand_use(tree: str, prefix: str = ""):
    """Expand a `use` tree (without `use` and `;`) into full paths."""
    tree = tree.strip()
    m = re.match(r"^([\w:]*?)(?:::)?\{(.*)\}$", tree, re.S)
    if not m:
        path = (prefix + "::" + tree) if prefix else tree
        path = re.sub(r"\s+as\s+\w+$", "", path)
        yield path
        return
    base = m.group(1)
    full = (prefix + "::" + base) if prefix and base else (prefix or base)
    depth, cur = 0, ""
    for ch in m.group(2) + ",":
        if ch == "," and depth == 0:
            if cur.strip() and cur.strip() != "self":
                yield from expand_use(cur, full)
            cur = ""
            continue
        depth += ch == "{"
        depth -= ch == "}"
        cur += ch
    return


@lru_cache(None)
def proven_paths():
    """Type name -> most used compiling `use` path, mined from the Rust tree."""
    seen = defaultdict(Counter)
    for root in RUST_ROOTS:
        for f in root.rglob("*.rs"):
            text = f.read_text(errors="replace")
            for m in re.finditer(r"^\s*(?:pub\s+)?use\s+([^;]+);", text, re.M):
                for path in expand_use(m.group(1)):
                    if path.split("::")[0] in CRATES:
                        seen[path.split("::")[-1]][path] += 1
    return {name: [p for p, _ in c.most_common()] for name, c in seen.items()}


@lru_cache(None)
def rust_sources():
    files = []
    for root in (Path("src"), Path("crates")):
        for f in root.rglob("*.rs"):
            if "/tests/" not in f.as_posix():
                files.append((f, f.read_text(errors="replace").splitlines()))
    return files


def rust_mentions(name: str):
    """Rust doc lines that name the Java type (folded or renamed ports)."""
    pat = re.compile(r"//.*`(?:[\w.]*\.)?" + re.escape(name) + r"[`.#(]")
    hits = []
    for f, lines in rust_sources():
        for n, line in enumerate(lines, 1):
            if pat.search(line):
                hits.append((0 if "Equivalent to" in line or "Port of" in line else 1, f"{f}:{n}"))
    return [h for _, h in sorted(hits)[:2]]


@lru_cache(None)
def definitions():
    """Type name -> [file:line] of `pub struct|enum|trait|type` in the crates."""
    defs = defaultdict(list)
    for root in (Path("src"), Path("crates")):
        for f in root.rglob("*.rs"):
            if "/tests/" in f.as_posix():
                continue
            for n, line in enumerate(f.read_text(errors="replace").splitlines(), 1):
                m = re.match(r"\s*pub(?:\([^)]*\))?\s+(?:struct|enum|trait|type)\s+(\w+)", line)
                if m:
                    defs[m.group(1)].append(f"{f}:{n}")
    return defs


@lru_cache(None)
def lucene_classes():
    """Simple class name -> Lucene source files (main and test trees)."""
    idx = defaultdict(list)
    for sub in ("core/src/java", "core/src/test", "test-framework/src/java", "codecs/src/java"):
        for f in (LUCENE / sub).rglob("*.java"):
            idx[f.stem].append(f)
    return idx


@lru_cache(None)
def helper_index():
    """Rust helper fn name -> [(call form, file:line)]."""
    idx = defaultdict(list)
    for form, path in HELPER_SOURCES:
        if not path.exists():
            continue
        for n, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
            m = re.match(r"\s*pub fn (\w+)", line)
            if m:
                idx[m.group(1)].append((form.format(m.group(1)), f"{path}:{n}"))
    return idx


@lru_cache(None)
def assert_methods():
    path = Path("crates/randomizedtesting/src/junit/assert.rs")
    names = set()
    if path.exists():
        names = set(re.findall(r"pub fn (\w+)", path.read_text(errors="replace")))
    return names


def java_test_methods(java: Path):
    clean = strip_code(java.read_text(errors="replace"))
    return re.findall(r"public\s+(?:final\s+)?void\s+(test\w*)\s*\(\s*\)", clean)


def rust_port_of(java_name: str):
    """Rust files that declare a port of the Java class `java_name`."""
    hits = []
    for root in (TF / "tests", TF / "src", Path("tests")):
        for f in root.rglob("*.rs"):
            text = " ".join(f.read_text(errors="replace").replace("//!", " ").split())
            if f.stem == snake(java_name) or \
               re.search(r"Port of `[^`]*\b" + re.escape(java_name) + r"(\.java)?`", text) or \
               re.search(r'"org\.apache\.lucene\.(?:\w+\.)*' + re.escape(java_name) + '"', text):
                hits.append(str(f))
    return hits


# --------------------------------------------------------------------------
# Skeleton
# --------------------------------------------------------------------------

def main() -> None:
    java = Path(sys.argv[1])
    out = Path(sys.argv[2])
    target = sys.argv[sys.argv.index("--target") + 1] if "--target" in sys.argv else "<target>"
    src = java.read_text()
    clean = strip_code(src)
    pkg = re.search(r"^\s*package\s+([\w.]+)\s*;", clean, re.M).group(1)
    cls = java.stem
    m = re.search(r"\bclass\s+" + re.escape(cls) + r"\b[^{]*\{", clean)
    if not m:
        sys.exit(f"{java}: top-level class {cls} not found")
    decl_start = clean.rfind(";", 0, m.start()) + 1
    cls_header = clean[decl_start:m.end() - 1]
    open_idx = m.end() - 1
    close_idx = match_brace(clean, open_idx)
    rel = java.as_posix().split("/lucene/core/src/test/")[-1]
    ext = re.search(r"\bextends\s+([\w.]+)", cls_header)
    base = ext.group(1).split(".")[-1] if ext else None
    cls_doc = javadoc_before(src, decl_start + len(cls_header) - len(cls_header.lstrip()))

    tests, others = [], []
    local_methods = set()
    for kind, start, text, header, span in members(src, clean, open_idx + 1, close_idx):
        h = " ".join(header.split())
        doc = javadoc_before(src, start)
        mname = re.search(r"(\w+)\s*\([^)]*\)\s*(?:throws[\w\s,.]+)?$", h)
        if kind == "block" and mname:
            local_methods.add(mname.group(1))
        mm = re.search(r"\bpublic\s+(?:final\s+)?void\s+(test\w*)\s*\(\s*\)", h)
        if kind == "block" and mm and " static " not in f" {h} ":
            tests.append((mm.group(1), annotations(h), src[span[0] + 1:span[1]], doc))
        else:
            others.append((text, doc))

    # ---- types the class names, resolved against the Rust tree ----------
    imports = re.findall(r"^\s*import\s+(?:static\s+)?([\w.]+)(?:\.\*)?\s*;", clean, re.M)
    body = clean[open_idx:close_idx]
    tokens = set(re.findall(r"\b([A-Z][A-Za-z0-9]*[a-z][A-Za-z0-9]*)\b", body))
    tokens |= {i.split(".")[-1] for i in imports if i.split(".")[-1][:1].isupper()}
    nested = set(re.findall(r"\b(?:class|interface|enum|record)\s+(\w+)", body))
    tokens -= nested | {cls, "String", "Override", "Exception", "Object", "Integer", "Long",
                        "Float", "Double", "Boolean", "Character", "Byte", "Short", "Math",
                        "System", "Thread", "Throwable", "Test"}
    proven, defs, luc = proven_paths(), definitions(), lucene_classes()
    uses, located, missing, jdk = [], [], [], []
    for t in sorted(tokens):
        if t in proven:
            uses.append(proven[t][0])
            alts = [x for x in proven[t][1:] if x.split("::")[:2] != proven[t][0].split("::")[:2]]
            if alts:
                located.append(f"{t}: `use {proven[t][0]}` chosen (most used); other compiling paths: "
                               + ", ".join(f"`{x}`" for x in alts[:2]) + " — pick the type the Java names")
        elif t in JDK_COMMON:
            jdk.append(t)
        elif t.endswith(("Exception", "Error")) and t not in defs:
            err = [h for h in rust_mentions(t) if h.startswith("src/error.rs")]
            located.append(f"{t}: Java exception -> a `LuceneError` variant (src/error.rs"
                           + (f", {err[0]}" if err else "; pick by the exception class name") + ");"
                           " check it with `crate::core_support::expect_throws_panic` or"
                           " `LuceneTestCase::expect_throws` as the ported tests do")
        elif t in defs:
            located.append(f"{t}: defined at {defs[t][0]} (no compiling `use` yet; check the public path)")
        elif rust_mentions(t):
            located.append(f"{t}: no Rust type of that name; represented as documented at "
                           + ", ".join(rust_mentions(t)))
        elif t in luc:
            missing.append(f"{t}: NOT PORTED — translate as is from {luc[t][0]}")
        else:
            jdk.append(t)

    # ---- helpers called without a receiver ------------------------------
    local_methods |= set(re.findall(r"\b\w[\w<>\[\],.? ]*\s+([a-z]\w*)\s*\([^;{)]*\)\s*(?:throws[\w\s,.]+)?\{", body))
    calls = set(re.findall(r"(?<![\w.])([a-z]\w*)\s*\(", body)) - JAVA_KEYWORDS - local_methods
    helpers, asserts = [], assert_methods()
    for c in sorted(calls):
        sn = HELPER_ALIASES.get(c, snake(c))
        cands = helper_index().get(sn, [])
        if cands:
            helpers.append(f"{c} -> " + "; ".join(f"`{f}` ({loc})" for f, loc in cands[:2]))
        elif sn in asserts:
            helpers.append(f"{c} -> `Assert::{sn}` (crates/randomizedtesting/src/junit/assert.rs)")
        else:
            helpers.append(f"{c} -> no Rust helper found: translate it from LuceneTestCase/Assert as is")

    # ---- base class -------------------------------------------------------
    base_lines = []
    if base and base != "LuceneTestCase":
        jfiles = luc.get(base, [])
        inherited = java_test_methods(jfiles[0]) if jfiles else []
        ports = rust_port_of(base)
        base_lines.append(f"Java base class `{base}`" + (f" ({jfiles[0]})" if jfiles else ""))
        base_lines.append("  Rust port: " + (", ".join(ports) if ports else
                          "NONE — port the base first, inside this task"))
        if inherited:
            base_lines.append("  inherited test methods (bind them too unless the base binding already does): "
                              + ", ".join(inherited))

    # ---- emit ---------------------------------------------------------------
    cls_anns = [a for a in annotations(cls_header) if a[0] != "SuppressWarnings"]
    o = [f"//! Port of `lucene/core/src/test/{rel}`."]
    if cls_doc:
        o.append("//!")
        o.append(comment(cls_doc, "", "//!"))
    o.append("//!")
    o.append("//! J2R skeleton (`tools/gen/j2r_skeleton.py`). Rules: `tools/gen/J2R_GUIDE.md`.")
    o.append("//! Translate every commented Java member, then delete the Java comments,")
    o.append("//! the `J2R` notes and the `todo!` markers; keep the `///` docs.")
    for line in base_lines:
        o.append("//! " + line)
    o.append("")
    o.append("use std::sync::Arc;\n")
    o.append("use randomizedtesting::jdk::reflect::Class;")
    o.append("use rucene::error::Result;")
    o.append("use rucene_test_framework::util::LuceneTestCase;")
    for u in sorted(set(uses) - {"rucene::error::Result", "randomizedtesting::jdk::reflect::Class",
                                 "rucene_test_framework::util::LuceneTestCase"}):
        o.append(f"use {u};")
    ann_used = sorted({a for a, args in cls_anns + [x for t in tests for x in t[1]]
                       if a in KNOWN_ANNOTATIONS and not args} | ({"AwaitsFix"} if any(
                           a == "AwaitsFix" for t in tests for a, _ in t[1]) else set()))
    if ann_used:
        o.append(f"use {ANNOTATION_PATH}::{{{', '.join(ann_used)}}};")
    o.append(f"\nuse crate::core_support::{{{'class_annotated' if cls_anns else 'class'}, random, test}};\n")

    if located or missing or jdk:
        o.append("// J2R types without a compiling `use` path:")
        for line in located + missing:
            o.append(f"// * {line}")
        if jdk:
            o.append("// * JDK or local names (map them as the ported tests do): " + ", ".join(jdk))
        o.append("")
    if helpers:
        o.append("// J2R helpers called without a receiver (Java -> Rust):")
        for line in helpers:
            o.append(f"// * {line}")
        o.append("")

    o.append(f"/// `{cls}`.")
    o.append("pub fn class_def() -> Arc<Class> {")
    entries = []
    for name, anns, _, _ in tests:
        e = f'test("{name}", {snake(name)})'
        notes = []
        for a, args in anns:
            if a == "Override":
                continue
            if a in KNOWN_ANNOTATIONS and not args:
                e += f".annotate({a})"
            elif a == "AwaitsFix":
                url = re.search(r'"([^"]*)"', args or "")
                e += f'.annotate(AwaitsFix {{ bug_url: "{url.group(1) if url else ""}".into() }})'
            else:
                notes.append(f"@{a}{(args or '').strip()}")
        entries.append(e + "," + (f" // J2R annotation {', '.join(notes)}" if notes else ""))
    fq = f"{pkg}.{cls}"
    if cls_anns:
        o.append(f'    class_annotated(\n        "{fq}",\n        vec![')
        o.extend(f"            {e}" for e in entries)
        o.append("        ],\n        |b| {")
        o.append("            // J2R class annotations (see the ported `class_annotated` uses):")
        o.append(comment("".join(f"@{a}{args or ''}\n" for a, args in cls_anns), "            "))
        o.append(f'            todo!("J2R {cls}: class annotations")')
        o.append("        },\n    )")
    else:
        o.append(f'    class(\n        "{fq}",\n        vec![')
        o.extend(f"            {e}" for e in entries)
        o.append("        ],\n    )")
    o.append("}\n")

    for name, _, body_src, doc in tests:
        if doc:
            o.append(comment(doc, "", "///"))
            o.append("///")
        o.append(f"/// Port of `{cls}.{name}`.")
        o.append(f"fn {snake(name)}() -> Result<()> {{")
        o.append(comment(body_src))
        o.append(f'    todo!("J2R {cls}.{name}")')
        o.append("}\n")

    if others:
        o.append(f"// J2R: the other members of `{cls}` (fields, helpers, setUp/tearDown,")
        o.append("// nested classes); translate each one where the tests use it, keeping")
        o.append("// its Javadoc as `///`.")
        for text, doc in others:
            if doc:
                o.append(comment("/** " + doc.replace("\n", "\n * ") + " */", ""))
            o.append(comment(text, ""))
            o.append("")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(o).rstrip() + "\n")

    mod = out.stem
    print(f'#[path = "{target}/{out.name}"]\nmod {mod};')
    print(f"#[test]\nfn {mod}() {{\n    run({mod}::class_def);\n}}")
    print(f"// {cls}: {len(tests)} test methods, {len(others)} other members, "
          f"{len(uses)} proven uses, {len(located)} located, {len(missing)} not ported, "
          f"{len(helpers)} helpers", file=sys.stderr)


if __name__ == "__main__":
    main()
