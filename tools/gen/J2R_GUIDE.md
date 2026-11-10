# J2R guide — translating Lucene core tests to Rucene

This guide is the standing brief of every translator in sprint 9 (tasks
#361–#403). It records the conventions the ported suites already follow, so
that no translator has to rediscover them. Every rule below cites the code
that establishes it.

## 1. Scope and phase

* **Translation only.** The code phase ports the Java faithfully and stops at
  the compile gate. No test runs, no investigation, no fixes of failures:
  those belong to the test phase, after all the code compiles.
* **Every method.** Translate every declared method and every inherited one
  the class runs. Never skip, stub or `#[ignore]` a method; keep Lucene's own
  annotations (`@Nightly`, `@Monster`, `@AwaitsFix`, `@SuppressCodecs`, …).
* **Missing Lucene members.** A member a test needs and Rucene lacks is
  translated from Lucene 10.5.0 (`/tmp/lucene1050/lucene/core/src/java/…`) as
  is, in `src/`, in the module that mirrors Lucene's package. A
  package-private Java member becomes `pub`, with a doc line saying so.
* **No new divergence** unless Lucene's way is impossible in Rust; then
  declare it (section 9). Pre-existing divergences you notice in `src/` are
  reported, not fixed.

## 2. Workflow per class

1. Generate the skeleton (from the repository root):

   ```bash
   python3 tools/gen/j2r_skeleton.py \
     /tmp/lucene1050/lucene/core/src/test/org/apache/lucene/index/TestX.java \
     crates/rucene-test-framework/tests/core_index/test_x.rs --target core_index
   ```

   It prints the `mod` and `#[test]` lines for the target file
   (`crates/rucene-test-framework/tests/core_index_tests.rs`), and writes the skeleton:
   * the `//!` header: Java path, class Javadoc, Java base class with its
     inherited test methods and the Rust port of that base;
   * `use` lines whose paths already compile in the ported tests;
   * a `J2R types` table: types only defined (check the public path),
     represented under another name (read the cited doc line), or not ported
     (translate them from the cited Lucene file);
   * a `J2R helpers` table: each receiver-less Java call (`atLeast`,
     `newDirectory`, `expectThrows`, …) with its Rust counterpart;
   * `class_def()` binding every declared test method with its annotations;
   * one function per test, with the Java Javadoc as `///` and the Java body
     as comments, ending in `todo!("J2R …")`;
   * the other members (fields, helpers, nested classes) as comments.
2. Translate each commented member in place; delete the Java comments, the
   `J2R` notes and the `todo!` markers; keep the `///` docs.
3. Intermediate gate (section 10).
4. Check coverage:

   ```bash
   grep -rn 'todo!("J2R' crates/          # must print nothing
   python3 tools/gen/j2r_coverage.py index --missing
   ```

## 3. Binding a class

* `class_def()` returns the class; each method is
  `test("testName", fn_name)` (`crates/rucene-test-framework/tests/core_support/mod.rs:35`). The Java
  name is the fully qualified one: `"org.apache.lucene.index.TestX"`.
* Class annotations use `class_annotated(name, methods, |b| b.annotate(...))`
  (`crates/rucene-test-framework/tests/core_support/mod.rs:59`); example with `@Monster` and
  `@SuppressCodecs`: `crates/rucene-test-framework/tests/core_index/test_2b_docs.rs:24-40`.
* Method annotations: `.annotate(Nightly)`, `.annotate(AwaitsFix { bug_url })`
  (`crates/rucene-test-framework/tests/core_geo/test_tessellator.rs:28-36`), from
  `rucene_test_framework::util::lucene_test_case`.
* Instance state, `@Before`/`setUp` and `@After`/`tearDown`: a class with
  fields uses `Class::builder(...).constructor(Constructor::no_args(T::new))`
  and `Method::new("setUp"|"initialize").annotate(Before).body(...)`
  (`crates/rucene-test-framework/tests/core_internal_hppc/test_long_hash_set.rs:477-486`).
* **One binding per Java class.** A class already bound elsewhere without its
  own methods (for example in a `base_*_suites.rs` file) moves to a single
  binding that carries them; remove the duplicate.
* The target registers each class with `#[test] fn x() { run(x::class_def); }`
  (`crates/rucene-test-framework/tests/core_document_tests.rs`).

## 4. Java base classes

* An abstract test base (`Base*TestCase`) becomes a trait for its abstract
  methods plus a fixture for its state; the inherited test methods are bound
  in each concrete class. Example:
  `crates/rucene-test-framework/tests/core_search/base_vector_similarity_query_test_case.rs` (reused by
  `TestByteVectorSimilarityQuery` and `TestFloatVectorSimilarityQuery`).
* When the skeleton header says `Rust port: NONE`, port the base first,
  inside the same task.

## 5. Exceptions and results

* Test functions return `rucene::error::Result<()>`; an `Err` is rethrown as
  its Java exception (`crates/rucene-test-framework/tests/core_support/mod.rs:35`,
  `crates/rucene-test-framework/src/util/java_exceptions.rs:198`).
* A Java exception class maps to a `LuceneError` variant (`src/error.rs`);
  `java_class_name`/`is_instance` (`crates/rucene-test-framework/src/util/java_exceptions.rs:130,203`) give
  the Java class.
* `expectThrows(X.class, () -> …)`:
  * body returning `Result`:
    `rucene_test_framework::util::java_exceptions::expect_throws(X, || …)`
    (`crates/rucene-test-framework/src/util/java_exceptions.rs:210`); the IAE message shortcut is
    `iae_message` (`crates/rucene-test-framework/tests/core_support/mod.rs:397`);
  * body that panics with a Java throwable: `expect_throws_panic(X, || …)`
    (`crates/rucene-test-framework/tests/core_support/mod.rs:246`; example
    `crates/rucene-test-framework/tests/core_util/test_chars_ref.rs:102`);
  * `LuceneTestCase::expect_throws(X, || -> Result<(), Throwable>)`
    (`crates/rucene-test-framework/src/util/lucene_test_case.rs:1198`).
* A call whose Java counterpart throws an unchecked exception: `unchecked(…)`
  (`crates/rucene-test-framework/tests/core_support/mod.rs:222`). `try { } catch (X e) { }`:
  `try_catch(X, || …)` (`crates/rucene-test-framework/tests/core_support/mod.rs:229`).

## 6. Java constructs without a direct Rust form (user decisions, 2026-11-09)

* **`null`.** An API that checks null takes `Option<…>` and raises Java's
  error for `None` (geo constructors, `LateInteractionField`,
  `AcceptDocs::from_iterator_supplier`). A container that holds null uses
  `Option<T>` values in the test. **Never leave a Java null check out
  because the Rust signature cannot take null:** change the API to `Option`
  and update its callers (rule #163; `TestIndexWriterConfig` setters, #379).
* **Reflection** (`getMethods()`, `getDeclaredMethods()`, `getSimpleName()`):
  a declared list compares the trait's methods with the explicit
  implementations.
* **`assertSame`.** Share the instance (`Arc` or a static) and check identity
  with `Arc::ptr_eq` (`crates/rucene-test-framework/tests/core_util/test_msb_radix_sorter.rs:9,159`);
  equality only where sharing cannot be expressed.
* **`instanceof` / casts.** The opt-in `as_any()` pattern: a default method
  returning `None`, overridden by the concrete types Java tests; then
  `x.as_any().downcast_ref::<T>()` (`crates/rucene-test-framework/tests/core_search/test_conjunctions.rs:148`).
* **Anonymous subclasses and overrides.** An override hook trait on the core
  type, default to Lucene's behaviour; example `IndexSearcherOverrides`
  (`crates/rucene-test-framework/tests/core_search/test_boolean_query.rs:1104-1109`, calling
  `searcher.super_count`). Wrappers for `FilterX` subclasses.
* **Fields changed after `doc.add`.** Rucene's `Document` gives no mutable
  access: rebuild the document for each `addDocument` from a copy of the field
  (`crates/rucene-test-framework/tests/core_search/support.rs:139`, `copy_field`), with no extra random
  draws.
* **UTF-16.** Java `char[]` and `CharSequence` units are `u16`
  (`CharArraySet`, `CharArrayMap`, `CharReader`, `CharacterBuffer`).

## 7. Randomness

* Draw exactly what Java draws, in Java's order: `random()`
  (`crates/rucene-test-framework/tests/core_support/mod.rs:88`), `LuceneTestCase::at_least`,
  `TestUtil::next_int`, `LuceneTestCase::rarely`, `usually`.
* Randomized helpers keep Java's names in snake case:
  `LuceneTestCase::new_directory`, `new_index_writer_config`,
  `new_searcher`, `new_text_field`, `new_string_field`;
  `rucene_test_framework::index::RandomIndexWriter`,
  `rucene_test_framework::analysis::MockAnalyzer`.

## 8. Documentation in the translated file

* `//!` header: `Port of \`lucene/core/src/test/…/TestX.java\`.`, then the
  class Javadoc and any file-level divergence.
* Each function: the Java Javadoc as `///`, then `/// Port of \`TestX.testY\`.`
* Keep Java's inline comments that explain the test.

## 9. Declaring a divergence

At the line, in the item's doc:

```rust
/// **Divergence.** Java <does what> (`File.java:line`); Rucene <does what>,
/// because <why Lucene's way is impossible or unsafe in Rust>.
```

Example: `crates/rucene-test-framework/tests/lucene_index_tests.rs:149`. List every divergence in the
report.

## 10. Compile gate and resources

The machine is shared by two lanes; never deviate:

* cargo only through `rucene-cargo` (memory-capped builds), always `-j 6`;
* the lane's own `CARGO_TARGET_DIR` and `TMPDIR` (given in the brief);
* **intermediate gate** (after each class): type-check only the target you
  changed, with no codegen or linking:
  `rucene-cargo check -j 6 -p rucene-test-framework --test <target>`
  (e.g. `--test core_index_tests`); when you changed `src/`, also
  `rucene-cargo check -j 6 -p rucene --lib --tests`;
* **final gate** (once, at the end of the pass, before the report):
  `rucene-cargo test -j 6 -p rucene -p rucene-test-framework -p rucene-codecs -p randomizedtesting --no-run`
  must finish with no errors; this is the pass's compile criterion. No test
  is executed in the code phase;
* `rucene-cargo fmt` on changed files; foreground builds, no polling;
* keep the tree compiling at every gate; if capacity runs low, finish the
  class in progress, gate, and stop.

## 12. Assertion parity (tasks #364-#369)

1. Generate the map of the task's package:
   `python3 tools/gen/j2r_asserts.py index --out <scratch>/asserts_index.md`
   (packages: `codecs`, `index`, `internal`, `search`, `util`, `analysis`,
   `document`, `geo`, `store`). Baseline on 2026-11-10: 2384 Java asserts,
   713 `ok`, 1063 `MISSING`, 365 `NO FN`, 243 `NO PORT`.
2. Work only the rows that are not `ok`, and confirm each `ok` row you touch:
   * `MISSING` and `NO FN`: port each Java `assert` at the equivalent point as
     `debug_assert!(cond, "msg", …)` with Java's condition and message (a Java
     `assert` runs only with `-ea`, as `debug_assert!` runs only in debug
     builds); Java overloads map to differently named Rust functions, so
     locate the right one;
   * `NO PORT`: the class has no Rust port; record the reason no counterpart
     applies, or port the assert where the class's logic was folded.
3. Record the final map (Java file:line -> Rust file:line, or the reason) in
   the task, as its acceptance criteria require.

## 11. Report

Per task: classes and methods translated against the Java totals
(`j2r_coverage.py`), Lucene members added in `src/` (and API changes to
existing callers), declared divergences, methods expected to fail (from
reading, not running), pre-existing divergences noticed, and the exact class
where the pass stopped.
