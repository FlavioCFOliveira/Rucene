#!/usr/bin/env python3
"""Proof that the Lucene member extractor sees the shapes it used to miss.

Until 2026-09-06 `enrich_lucene_kg.py` recorded no enum constant at all and no
field carrying an initialiser. Its member regex demands at least one explicit
modifier -- and JLS 8.9.3 gives an enum constant its `public static final`
implicitly -- and it demanded `;` or `{` immediately after the declarator, which
`Type NAME = value;` never offers. Three port claims the crate makes were
refused for want of the member they name: `store.PreloadHint.INSTANCE`,
`store.ReadOnceHint.INSTANCE` and `util.hnsw.HnswGraph.EMPTY`, the last an
anonymous class spanning forty lines.

Nothing audits the member surface -- `audit_kg.py` re-derives files and types,
not members -- so the gap was silent, and `CLAUDE.md` 6.1 wants a denominator
that can be defended rather than assumed. This check exercises the extractor
against a hand-built source tree covering each shape, so the guarantee is
reproducible and cheap to re-run:

    python3 tools/kg/member_extraction_check.py

`failures: 0` is the pass condition. It writes nothing outside a temporary
directory and never touches the graph.
"""
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import enrich_lucene_kg as en

FAILURES = []


def check(label, got, want):
    ok = got == want
    print(f"{'OK ' if ok else 'XX '}{label}")
    if not ok:
        print(f"     got  {got!r}")
        print(f"     want {want!r}")
        FAILURES.append(label)


# --- the fixture tree -------------------------------------------------------
#
# Each file reproduces a shape taken from Lucene 10.5.0, named after the type it
# is modelled on, so a failure points at real source rather than at a puzzle.

SOURCES = {
    # A single-constant enum: the whole of `store/PreloadHint.java`.
    "PreloadHint.java": """
package org.apache.lucene.store;

/** A hint that the file should be preloaded into memory */
public enum PreloadHint implements IOContext.FileOpenHint {
  INSTANCE
}
""",
    # Constants with arguments and with constant-specific class bodies, plus
    # members after the closing `;` -- the shape of `search/ScoreMode.java` and
    # `codecs/lucene103/blocktree/TrieBuilder.java`.
    "ScoreMode.java": """
package org.apache.lucene.search;

public enum ScoreMode {
  @Deprecated COMPLETE(true),
  TOP_SCORES(false) {
    @Override
    public boolean isExhaustive() {
      return false;
    }
  },
  TOP_DOCS(false, false);

  private final boolean needsScores;

  private ScoreMode(boolean needsScores) {
    this.needsScores = needsScores;
  }

  public boolean needsScores() {
    return needsScores;
  }
}
""",
    # A legal enum that declares no constant at all. Its first member must not
    # be read as one.
    "EmptyEnum.java": """
package org.apache.lucene.util;

enum EmptyEnum {
  ;

  static int counter = 0;

  public static void reset() {
    counter = 0;
  }
}
""",
    # A `public static final` whose initialiser is a multi-line anonymous class:
    # `util/hnsw/HnswGraph.java:106`. The body declares members of its own, none
    # of which belongs to the enclosing type.
    "HnswGraph.java": """
package org.apache.lucene.util.hnsw;

public abstract class HnswGraph {

  public abstract int neighborCount();

  /** Empty graph value */
  public static final HnswGraph EMPTY =
      new HnswGraph() {

        @Override
        public int neighborCount() {
          return 0;
        }

        @Override
        public String toString() {
          int unreachable = 0;
          return "EMPTY";
        }
      };

  public int size() {
    return 0;
  }
}
""",
    # Initialisers with commas, generics, arrays, lambdas and calls, and a
    # multi-variable declaration -- everything that used to make the naive
    # `= [^,;]+` strip produce nonsense.
    "Initialisers.java": """
package org.apache.lucene.util;

public class Initialisers {
  public static final int BLOCK_SIZE = 1 << 7;
  private static final int[] MASKS = new int[] {1, 2, 3};
  static final Map<String, String> ALIASES = Map.of("a", "b", "c", "d");
  private final Comparator<Long> BY_VALUE = (a, b) -> Long.compare(a, b);
  private long first = compute(1, 2), second, third = 3;
  private int plain;

  public int compute(int a, int b) {
    return a + b;
  }
}
""",
}


def run():
    root = Path(tempfile.mkdtemp(prefix="rucene-member-check-"))
    pkgdir = root / "lucene/core/src/java/org/apache/lucene/fixture"
    pkgdir.mkdir(parents=True)
    for name, text in SOURCES.items():
        (pkgdir / name).write_text(text, encoding="utf-8")

    en.configure(str(root), "check", "1970-01-01")
    members = en.extract_members()
    shutil.rmtree(root)
    return members


members = run()
by_kind = {k: {m["qualifiedName"] for m in v} for k, v in members.items()}
signature = {m["qualifiedName"]: m["signature"]
             for v in members.values() for m in v}
owner_of = {m["qualifiedName"]: m["parentQualifiedName"]
            for v in members.values() for m in v}

print("=" * 78)
print("A. ENUM CONSTANTS")
print("=" * 78)
check("a single-constant enum yields its constant",
      "org.apache.lucene.store.PreloadHint#INSTANCE" in by_kind["enumConstants"],
      True)
check("constants with arguments and class bodies are all found",
      sorted(q.rsplit("#", 1)[1] for q in by_kind["enumConstants"]
             if q.startswith("org.apache.lucene.search.ScoreMode#")),
      ["COMPLETE", "TOP_DOCS", "TOP_SCORES"])
check("an argument list is kept in the signature",
      signature.get("org.apache.lucene.search.ScoreMode#TOP_DOCS"),
      "TOP_DOCS(false, false)")
check("an enum that declares no constant yields none",
      [q for q in by_kind["enumConstants"]
       if q.startswith("org.apache.lucene.util.EmptyEnum#")],
      [])
check("a member of a constant-specific class body is not the enum's",
      "org.apache.lucene.search.ScoreMode#isExhaustive()" in by_kind["methods"],
      False)
check("the enum's own members survive the constant list",
      {"org.apache.lucene.search.ScoreMode#needsScores()",
       "org.apache.lucene.search.ScoreMode#needsScores"} & (
          by_kind["methods"] | by_kind["fields"]),
      {"org.apache.lucene.search.ScoreMode#needsScores()",
       "org.apache.lucene.search.ScoreMode#needsScores"})

print()
print("=" * 78)
print("B. FIELDS WITH AN INITIALISER")
print("=" * 78)
check("a field initialised by a multi-line anonymous class is found",
      signature.get("org.apache.lucene.util.hnsw.HnswGraph#EMPTY"),
      "HnswGraph EMPTY")
check("no member of the anonymous class is attributed to the type",
      sorted(q for q in by_kind["methods"]
             if owner_of[q] == "org.apache.lucene.util.hnsw.HnswGraph"),
      ["org.apache.lucene.util.hnsw.HnswGraph#neighborCount()",
       "org.apache.lucene.util.hnsw.HnswGraph#size()"])
check("initialisers holding commas, generics, arrays and lambdas are stripped",
      {q.rsplit("#", 1)[1]: signature[q] for q in by_kind["fields"]
       if q.startswith("org.apache.lucene.util.Initialisers#")},
      {"BLOCK_SIZE": "int BLOCK_SIZE",
       "MASKS": "int[] MASKS",
       "ALIASES": "Map<String, String> ALIASES",
       "BY_VALUE": "Comparator<Long> BY_VALUE",
       "first": "long first",
       "second": "long second",
       "third": "long third",
       "plain": "int plain"})

print()
print(f"failures: {len(FAILURES)}")
for f in FAILURES:
    print("  -", f)
sys.exit(1 if FAILURES else 0)
