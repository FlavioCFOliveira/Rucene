#!/usr/bin/env python3
"""Proof that the Lucene member extractor sees the shapes it used to miss.

`enrich_lucene_kg.py` has lost whole classes of member to its regex, one family
at a time, and each loss shrank the denominator every depth and triage figure is
quoted against. `CLAUDE.md` 6.1 wants a denominator that can be defended rather
than assumed, and nothing else audits the member surface -- `audit_kg.py`
re-derives files and types, not members -- so this check is where that guarantee
lives. It exercises the extractor against a hand-built source tree, each file
modelled on the Lucene 10.5.0 source that exposed the defect:

* **Enum constants** (A). `PreloadHint.INSTANCE` is the whole body of its enum,
  and JLS 8.9.3 gives it `public static final` implicitly, so a regex demanding
  an explicit modifier saw no constant of any Lucene enum.
* **Fields with an initialiser** (B). The regex demanded `;` or `{` immediately
  after the declarator, which `Type NAME = value;` never offers -- not even
  `HnswGraph.EMPTY`, whose anonymous class spans forty lines.
* **Members with no modifier at all** (C). An interface method
  (`Accountable.ramBytesUsed()`) and every package-private member carry none,
  and no constructor is required to. That was 949 methods, 338 constructors and
  438 fields: 10.1% of the method denominator.
* **An annotation written before the modifiers** (D). The regex put the
  annotation group *after* the modifier group, so `@Deprecated public static
  final` matched nothing -- which is how 32 of the 33 `Version.LUCENE_*`
  constants were missing.
* **Comments, strings and char literals** (E). Stripping strings before line
  comments lets a `//` comment holding an odd number of `"` open a phantom
  string that swallows the code after it, which deleted `Automaton#toDot` and
  `RegExp#parseCharExp`.
* **The traps that opening the regex up creates** (F). With the modifier
  optional, a continuation line reads as a declaration of its own and a match
  starting in the blanked Javadoc above a member swallows its modifiers into
  the return type. Both must stay shut.
* **C-style array declarators** (G). JLS 10.2 puts the brackets after the
  declarator, and each declarator carries its own: `int from[], to[];`.

Run it:

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
    # An interface whose methods carry no modifier at all -- `util/Accountable.java`
    # verbatim in shape, down to the field declared with no modifier either.
    "Accountable.java": """
package org.apache.lucene.util;

public interface Accountable {

  /** Returns an estimate of the JVM heap memory used by this object in bytes. */
  long ramBytesUsed();

  /** Returns nested resources of this class. */
  default Collection<Accountable> getChildResources() {
    return Collections.emptyList();
  }

  /** An accountable that always returns 0 */
  Accountable NULL_ACCOUNTABLE = () -> 0;
}
""",
    # A package-private class: no modifier on the type, the constructor, two of
    # the methods or the field. The shape of `index/DocumentsWriterDeleteQueue.java`,
    # which alone accounted for 28 of the missing methods.
    "DocumentsWriterDeleteQueue.java": """
package org.apache.lucene.index;

final class DocumentsWriterDeleteQueue {

  final long generation;
  private volatile boolean closed;

  DocumentsWriterDeleteQueue(long generation) {
    this.generation = generation;
  }

  long getNextSequenceNumber() {
    return generation;
  }

  static DocumentsWriterDeleteQueue newQueue(long generation) {
    return new DocumentsWriterDeleteQueue(generation);
  }

  public boolean isOpen() {
    return closed == false;
  }
}
""",
    # `@Deprecated public static final`: the annotation before the modifiers.
    # `util/Version.java:194`, plus the array-valued annotation whose braces
    # must leave the brace-depth accounting alone.
    "Version.java": """
package org.apache.lucene.util;

public final class Version {

  /**
   * @deprecated Use latest
   */
  @Deprecated public static final Version LUCENE_9_12_4 = new Version(9, 12, 4);

  @Deprecated
  public static final Version LUCENE_10_0_0 = new Version(10, 0, 0);

  @SuppressWarnings({"unchecked", "rawtypes"})
  static final Version[] ALL = new Version[0];

  public static final Version LATEST = LUCENE_10_0_0;

  public boolean onOrAfter(Version other) {
    return true;
  }
}
""",
    # A commented-out method whose strings mis-pair: `util/automaton/Automaton.java`
    # carries exactly this, an old `writeDot` left in a `/* */` block, and the
    # quotes inside it are what deleted `Automaton#toDot`. Stripping strings
    # before comments pairs the stray quote in the comment with the next quote
    # in real code and blanks everything between, declaration and all; the
    # `'"'` char literal in `parseSimpleExp` (`util/automaton/RegExp.java:1473`)
    # then swallows a `{`, so every member after it sits at the wrong brace
    # depth and is discarded. One left-to-right pass is the only order that is
    # right.
    # Raw: the backslashes below are Java escapes, and must reach the fixture
    # as backslashes.
    "Automaton.java": r"""
package org.apache.lucene.util.automaton;

public class Automaton {

  /*
  public void writeDot(String fileName) {
    // the name must not start with a "
    if (fileName.indexOf('/') == -1) {
      fileName = "/tmp/" + fileName;
    }
  }
  */

  /** Returns the dot (graphviz) representation of this automaton. */
  public String toDot() {
    StringBuilder b = new StringBuilder();
    b.append("digraph Automaton {\n");
    return b.toString();
  }

  final int parseSimpleExp() {
    if (match('"')) {
      int start = pos;
      while (more() && !peek("\"")) next();
    }
    return 0;
  }

  final int parseCharExp() {
    return match('\\');
  }

  public int getNumStates() {
    return 4;
  }
}
""",
    # The two traps that opening the regex up creates. `Map.of(a, b);` on its own
    # line is the continuation of a field, not a method; and a match allowed to
    # start in the blanked Javadoc above `public int size()` reads `public int`
    # as the return type, which is not a type, so the method vanishes.
    "Continuation.java": """
package org.apache.lucene.util;

public class Continuation {

  private static final Map<String, String> ALIASES =
      Map.of(
          "a", "b");

  private final int[] table =
      new int[] {1, 2, 3};

  /**
   * Returns the size.
   *
   * @return the size
   */
  public int size() {
    int local = table.length;
    Continuation other = null;
    return local;
  }
}
""",
    # JLS 10.2: brackets after the declarator, one set per declarator.
    # `util/automaton/RegExp.java:518` and the arrays JFlex generates in
    # `analysis/standard/StandardTokenizerImpl.java`.
    "CStyleArrays.java": """
package org.apache.lucene.util.automaton;

public class CStyleArrays {
  public final int from[], to[];
  private static final int ZZ_LEXSTATE[] = {0, 1};
  private char zzBuffer[] = new char[16];
  private int plain;

  strictfp double scaled() {
    return 0.0;
  }
}
""",
    # An enum constant whose argument is a string literal: the signature must
    # carry the real text, which only the unblanked source has.
    # `geo/SimpleWKTShapeParser.java`.
    "ShapeType.java": """
package org.apache.lucene.geo;

public enum ShapeType {
  POINT("point"),
  ENVELOPE("envelope");

  private final String shapeName;

  ShapeType(String shapeName) {
    this.shapeName = shapeName;
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
modifiers = {m["qualifiedName"]: m.get("modifiers")
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
print("=" * 78)
print("C. MEMBERS WITH NO MODIFIER AT ALL")
print("=" * 78)
check("an interface's abstract method is found",
      signature.get("org.apache.lucene.util.Accountable#ramBytesUsed()"),
      "long ramBytesUsed()")
check("an interface's default method is found",
      signature.get("org.apache.lucene.util.Accountable#getChildResources()"),
      "Collection<Accountable> getChildResources()")
check("an interface's implicitly public static final field is found",
      signature.get("org.apache.lucene.util.Accountable#NULL_ACCOUNTABLE"),
      "Accountable NULL_ACCOUNTABLE")
check("a package-private class yields every member, modifier or not",
      {q.split("#", 1)[1]: signature[q]
       for v in by_kind.values() for q in v
       if q.startswith("org.apache.lucene.index.DocumentsWriterDeleteQueue#")},
      {"DocumentsWriterDeleteQueue(long generation)":
           "DocumentsWriterDeleteQueue(long generation)",
       "getNextSequenceNumber()": "long getNextSequenceNumber()",
       "newQueue(long generation)":
           "DocumentsWriterDeleteQueue newQueue(long generation)",
       "isOpen()": "boolean isOpen()",
       "generation": "long generation",
       "closed": "boolean closed"})
check("a constructor with no modifier is a constructor, not a method",
      ("org.apache.lucene.index.DocumentsWriterDeleteQueue"
       "#DocumentsWriterDeleteQueue(long generation)")
      in by_kind["constructors"],
      True)
check("the modifiers recorded are the modifiers written",
      modifiers.get("org.apache.lucene.index.DocumentsWriterDeleteQueue"
                    "#getNextSequenceNumber()"),
      "")

print()
print("=" * 78)
print("D. AN ANNOTATION BEFORE THE MODIFIERS")
print("=" * 78)
check("`@Deprecated public static final` is a field",
      signature.get("org.apache.lucene.util.Version#LUCENE_9_12_4"),
      "Version LUCENE_9_12_4")
check("the annotation is not recorded as a modifier",
      modifiers.get("org.apache.lucene.util.Version#LUCENE_9_12_4"),
      "public static final")
check("an annotation on its own line behaves the same",
      signature.get("org.apache.lucene.util.Version#LUCENE_10_0_0"),
      "Version LUCENE_10_0_0")
check("an array-valued annotation leaves the brace depth alone",
      sorted(q.split("#", 1)[1] for q in by_kind["fields"]
             if q.startswith("org.apache.lucene.util.Version#")),
      ["ALL", "LATEST", "LUCENE_10_0_0", "LUCENE_9_12_4"])
check("the member after the annotated ones is still found",
      "org.apache.lucene.util.Version#onOrAfter(Version other)"
      in by_kind["methods"],
      True)

print()
print("=" * 78)
print("E. COMMENTS, STRINGS AND CHAR LITERALS")
print("=" * 78)
check("a string inside a comment deletes no declaration",
      sorted(q.split("#", 1)[1] for q in by_kind["methods"]
             if q.startswith("org.apache.lucene.util.automaton.Automaton#")),
      ["getNumStates()", "parseCharExp()", "parseSimpleExp()", "toDot()"])
check("a method that exists only inside a comment is not a member",
      [q for q in by_kind["methods"] if "writeDot" in q],
      [])

print()
print("=" * 78)
print("F. WHAT MUST NOT BE A MEMBER")
print("=" * 78)
check("a continuation line is not a declaration",
      sorted(q.split("#", 1)[1] for q in by_kind["methods"]
             if q.startswith("org.apache.lucene.util.Continuation#")),
      ["size()"])
check("the fields it continues are found whole",
      {q.split("#", 1)[1]: signature[q] for q in by_kind["fields"]
       if q.startswith("org.apache.lucene.util.Continuation#")},
      {"ALIASES": "Map<String, String> ALIASES", "table": "int[] table"})
check("a Javadoc'd method keeps its modifiers out of its return type",
      (modifiers.get("org.apache.lucene.util.Continuation#size()"),
       signature.get("org.apache.lucene.util.Continuation#size()")),
      ("public", "int size()"))
check("a local variable is not a field of the enclosing type",
      {q for q in by_kind["fields"]
       if q.endswith("#local") or q.endswith("#other")},
      set())

print()
print("=" * 78)
print("G. C-STYLE ARRAY DECLARATORS")
print("=" * 78)
check("brackets after the declarator belong to that declarator",
      {q.split("#", 1)[1]: signature[q] for q in by_kind["fields"]
       if q.startswith("org.apache.lucene.util.automaton.CStyleArrays#")},
      {"from": "int[] from", "to": "int[] to",
       "ZZ_LEXSTATE": "int[] ZZ_LEXSTATE", "zzBuffer": "char[] zzBuffer",
       "plain": "int plain"})
check("`strictfp` is a modifier, not a return type",
      (modifiers.get("org.apache.lucene.util.automaton.CStyleArrays#scaled()"),
       signature.get("org.apache.lucene.util.automaton.CStyleArrays#scaled()")),
      ("strictfp", "double scaled()"))

print()
print("=" * 78)
print("H. AN ENUM CONSTANT'S ARGUMENT LIST")
print("=" * 78)
check("the argument list is the source text, not the blanked text",
      {q.split("#", 1)[1]: signature[q] for q in by_kind["enumConstants"]
       if q.startswith("org.apache.lucene.geo.ShapeType#")},
      {"POINT": 'POINT("point")', "ENVELOPE": 'ENVELOPE("envelope")'})

print()
print(f"failures: {len(FAILURES)}")
for f in FAILURES:
    print("  -", f)
sys.exit(1 if FAILURES else 0)
