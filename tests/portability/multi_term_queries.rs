//! Multi-term queries and their rewrite methods against Apache Lucene Core
//! 10.5.0 (rmp task #405).
//!
//! The scoring corpus of `similarity_scores.rs` is defined by the same integer
//! formulas here and in `MultiTermQueryFixture.java`, and indexed with the
//! default similarity by Rucene into `R` and by Lucene into `L`.
//!
//! * The query set (`MultiTermQueryFixture.query`, [`query_report`]) runs
//!   `WildcardQuery`, `PrefixQuery`, `RegexpQuery`, `FuzzyQuery` (maxEdits 0–2,
//!   transpositions on/off, prefixLength 0/1), `TermRangeQuery` (inclusive and
//!   exclusive bounds, open ends), `TermInSetQuery` and `AutomatonQuery` under
//!   `CONSTANT_SCORE_BLENDED_REWRITE`, `CONSTANT_SCORE_REWRITE`,
//!   `SCORING_BOOLEAN_REWRITE`, `TopTermsScoringBooleanQueryRewrite(3)` and,
//!   on the keyword field that carries `SORTED_SET` doc values,
//!   `DOC_VALUES_REWRITE`. Each query prints its total hits (value and
//!   relation) and its hits with score bits. The report ends with the terms
//!   `Terms.intersect(CompiledAutomaton, BytesRef)` returns, per leaf, for a
//!   few automata.
//! * The four reports (each side over each index) are compared line by line.
//! * Positive control: Rucene writes a perturbed corpus (document 0 gains the
//!   body token `w7` and the keyword `zebra`); its report must differ from
//!   Lucene's report on `L`, with the same document counts.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{
    Document, Field, FieldType, NumericDocValuesField, SortedSetDocValuesField, Store, StringField,
};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexOptions, IndexReader, StoredFields, Term};
use rucene::search::{
    constant_score_blended_rewrite, constant_score_rewrite, doc_values_rewrite,
    scoring_boolean_rewrite, AutomatonQuery, DefaultProvider, FuzzyQuery, IndexSearcher,
    PrefixQuery, Query, RegexpQuery, RewriteMethod, TermInSetQuery, TermRangeQuery,
    TopTermsScoringBooleanQueryRewrite, TotalHits, TotalHitsRelation, WildcardQuery,
    DEFAULT_MAX_EXPANSIONS,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::automaton::{
    Automata, Automaton, CompiledAutomaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// Scoring corpus: the formulas of MultiTermQueryFixture
// ---------------------------------------------------------------------------

/// `MultiTermQueryFixture.BATCHES`.
const BATCHES: [(i32, i32); 3] = [(0, 100), (100, 200), (200, 300)];

/// `MultiTermQueryFixture.WORDS`.
const WORDS: [&str; 20] = [
    "apple", "apply", "ample", "maple", "apricot", "banana", "bandana", "cabana", "cherry",
    "sherry", "berry", "merry", "date", "data", "gate", "grape", "graph", "form", "from", "zebra",
];

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

/// `MultiTermQueryFixture.body(int)`; the perturbed corpus appends `w7` to
/// document 0.
fn body(i: i32, perturbed: bool) -> String {
    let mut words: Vec<String> = Vec::new();
    for j in 0..i % 9 + 2 {
        let word = if j % 4 == 3 {
            words[(j - 2) as usize].clone()
        } else {
            format!("w{}", (i * 31 + j * 17) % 40)
        };
        words.push(word);
    }
    if perturbed && i == 0 {
        words.push("w7".to_string());
    }
    words.join(" ")
}

fn title(i: i32) -> String {
    (0..i % 3 + 1)
        .map(|j| format!("w{}", (i * 7 + j * 11) % 40))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `MultiTermQueryFixture.TEXT_TYPE`.
fn text_type() -> FieldType {
    let t = FieldType::new();
    t.set_tokenized(true).unwrap();
    t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    t.freeze();
    t
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

/// `MultiTermQueryFixture.doc(int)`; the perturbed corpus gives document 0 a
/// third keyword, `zebra`.
fn doc(i: i32, perturbed: bool) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    d.add(Box::new(
        Field::new("body", body(i, perturbed), text_type()).unwrap(),
    ));
    d.add(Box::new(
        Field::new("title", title(i), text_type()).unwrap(),
    ));
    let mut keywords = vec![WORDS[(i % 20) as usize], WORDS[((i * 7 + 3) % 20) as usize]];
    if perturbed && i == 0 {
        keywords.push("zebra");
    }
    for kw in keywords {
        d.add(Box::new(
            StringField::new("kw", kw.to_string(), Store::NO).unwrap(),
        ));
        d.add(Box::new(SortedSetDocValuesField::new("kw", bytes(kw))));
    }
    d.add(Box::new(NumericDocValuesField::new(
        "rank",
        ((i * 37) % 101) as i64,
    )));
    d
}

/// `MultiTermQueryFixture.write(Directory)`: the same operations, in order.
fn write_corpus(path: &Path, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    let w = IndexWriter::new(dir, config).unwrap();
    for (b, (from, to)) in BATCHES.iter().enumerate() {
        for i in *from..*to {
            w.add_document(&doc(i, perturbed)).unwrap();
        }
        if b == 0 {
            w.delete_documents_by_terms(vec![term("id", &id(7))])
                .unwrap();
            w.delete_documents_by_terms(vec![term("id", &id(42))])
                .unwrap();
        } else if b == 2 {
            w.delete_documents_by_terms(vec![term("id", &id(250))])
                .unwrap();
        }
        w.commit().unwrap();
    }
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Query set: the output of MultiTermQueryFixture.query, line for line
// ---------------------------------------------------------------------------

/// `MultiTermQueryFixture.TOP_N`.
const TOP_N: i32 = 10;

/// `Float.floatToIntBits`, which collapses every NaN to `0x7fc00000`.
fn bits(f: f32) -> String {
    let raw = if f.is_nan() { 0x7fc0_0000 } else { f.to_bits() };
    format!("{raw:08x}")
}

fn relation(total: &TotalHits) -> &'static str {
    match total.relation() {
        TotalHitsRelation::EQUAL_TO => "EQUAL_TO",
        TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO => "GREATER_THAN_OR_EQUAL_TO",
    }
}

fn stored_id(stored: &dyn StoredFields, doc: i32) -> String {
    let document = stored.document(doc).unwrap();
    document
        .get_field("id")
        .and_then(|f| f.string_value())
        .unwrap_or_else(|| "null".to_string())
}

/// `MultiTermQueryFixture.regexp(String)`: the determinized automaton of a
/// regular expression with every syntax flag.
fn regexp(s: &str) -> Automaton {
    let automaton = RegExp::new(s).unwrap().to_automaton().unwrap();
    Operations::determinize(&automaton, DEFAULT_DETERMINIZE_WORK_LIMIT).unwrap()
}

type Rewrites = Vec<(&'static str, Arc<dyn RewriteMethod>)>;

/// `MultiTermQueryFixture.textRewrites()`.
fn text_rewrites() -> Rewrites {
    vec![
        ("blended", constant_score_blended_rewrite()),
        ("constant", constant_score_rewrite()),
        ("scoring-boolean", scoring_boolean_rewrite()),
        (
            "top-terms-3",
            Arc::new(TopTermsScoringBooleanQueryRewrite::new(3)),
        ),
    ]
}

/// `MultiTermQueryFixture.keywordRewrites()`.
fn keyword_rewrites() -> Rewrites {
    let mut r = text_rewrites();
    r.push(("doc-values", doc_values_rewrite()));
    r
}

fn wildcard(field: &str, text: &str, rewrite: Arc<dyn RewriteMethod>) -> Arc<dyn Query> {
    Arc::new(
        WildcardQuery::with_rewrite_method(
            term(field, text),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
            rewrite,
        )
        .unwrap(),
    )
}

fn prefix(field: &str, text: &str, rewrite: Arc<dyn RewriteMethod>) -> Arc<dyn Query> {
    Arc::new(PrefixQuery::with_rewrite_method(term(field, text), rewrite).unwrap())
}

fn regexp_query(field: &str, text: &str, rewrite: Arc<dyn RewriteMethod>) -> Arc<dyn Query> {
    Arc::new(
        RegexpQuery::with_everything(
            term(field, text),
            RegExp::ALL,
            0,
            &DefaultProvider,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
            rewrite,
            true,
        )
        .unwrap(),
    )
}

fn fuzzy(
    field: &str,
    text: &str,
    max_edits: i32,
    transpositions: bool,
    prefix_length: i32,
    rewrite: Arc<dyn RewriteMethod>,
) -> Arc<dyn Query> {
    Arc::new(
        FuzzyQuery::with_rewrite_method(
            term(field, text),
            max_edits,
            prefix_length,
            DEFAULT_MAX_EXPANSIONS,
            transpositions,
            rewrite,
        )
        .unwrap(),
    )
}

fn range(
    field: &str,
    lower: Option<&str>,
    upper: Option<&str>,
    include_lower: bool,
    include_upper: bool,
    rewrite: Arc<dyn RewriteMethod>,
) -> Arc<dyn Query> {
    Arc::new(
        TermRangeQuery::with_rewrite_method(
            field,
            lower.map(bytes),
            upper.map(bytes),
            include_lower,
            include_upper,
            rewrite,
        )
        .unwrap(),
    )
}

fn terms_in_set(field: &str, texts: &[&str], rewrite: Arc<dyn RewriteMethod>) -> Arc<dyn Query> {
    Arc::new(
        TermInSetQuery::with_rewrite_method(
            rewrite,
            field,
            texts.iter().map(|t| bytes(t)).collect(),
        )
        .unwrap(),
    )
}

type QueryFactory = Box<dyn Fn(Arc<dyn RewriteMethod>) -> Arc<dyn Query>>;

/// `MultiTermQueryFixture.keywordQueries()`, in the same order.
fn keyword_queries() -> Vec<(String, QueryFactory)> {
    let mut q: Vec<(String, QueryFactory)> = vec![
        (
            "wildcard kw:ap*e".to_string(),
            Box::new(|r| wildcard("kw", "ap*e", r)),
        ),
        (
            "wildcard kw:?erry".to_string(),
            Box::new(|r| wildcard("kw", "?erry", r)),
        ),
        (
            "prefix kw:ba".to_string(),
            Box::new(|r| prefix("kw", "ba", r)),
        ),
        (
            "regexp kw:[bc]a.*a".to_string(),
            Box::new(|r| regexp_query("kw", "[bc]a.*a", r)),
        ),
    ];
    for e in 0..=2 {
        for t in [true, false] {
            for p in 0..=1 {
                q.push((
                    format!("fuzzy kw:form maxEdits={e} transpositions={t} prefixLength={p}"),
                    Box::new(move |r| fuzzy("kw", "form", e, t, p, r)),
                ));
            }
        }
    }
    q.push((
        "fuzzy kw:apple maxEdits=1".to_string(),
        Box::new(|r| fuzzy("kw", "apple", 1, true, 0, r)),
    ));
    q.push((
        "fuzzy kw:apple maxEdits=2".to_string(),
        Box::new(|r| fuzzy("kw", "apple", 2, true, 0, r)),
    ));
    q.push((
        "range kw:[banana TO date]".to_string(),
        Box::new(|r| range("kw", Some("banana"), Some("date"), true, true, r)),
    ));
    q.push((
        "range kw:{banana TO date}".to_string(),
        Box::new(|r| range("kw", Some("banana"), Some("date"), false, false, r)),
    ));
    q.push((
        "range kw:{* TO cherry}".to_string(),
        Box::new(|r| range("kw", None, Some("cherry"), false, false, r)),
    ));
    q.push((
        "range kw:{grape TO *]".to_string(),
        Box::new(|r| range("kw", Some("grape"), None, false, true, r)),
    ));
    q.push((
        "terms kw:{apple berry zebra nothere}".to_string(),
        Box::new(|r| terms_in_set("kw", &["apple", "berry", "zebra", "nothere"], r)),
    ));
    q.push((
        "automaton kw:(apple|cherry|g.*)".to_string(),
        Box::new(|r| {
            Arc::new(
                AutomatonQuery::with_rewrite_method(
                    term("kw", ""),
                    regexp("(apple|cherry|g.*)"),
                    false,
                    r,
                )
                .unwrap(),
            )
        }),
    ));
    q.push((
        "automaton kw:binary [c TO g}".to_string(),
        Box::new(|r| {
            let interval =
                Automata::make_binary_interval(Some(&bytes("c")), true, Some(&bytes("g")), false)
                    .unwrap();
            Arc::new(
                AutomatonQuery::with_rewrite_method(term("kw", ""), interval, true, r).unwrap(),
            )
        }),
    ));
    q
}

/// `MultiTermQueryFixture.textQueries()`, in the same order.
fn text_queries() -> Vec<(String, QueryFactory)> {
    vec![
        (
            "wildcard body:w1*".to_string(),
            Box::new(|r| wildcard("body", "w1*", r)),
        ),
        (
            "prefix body:w2".to_string(),
            Box::new(|r| prefix("body", "w2", r)),
        ),
        (
            "regexp body:w[1-3]?[05]".to_string(),
            Box::new(|r| regexp_query("body", "w[1-3]?[05]", r)),
        ),
        (
            "fuzzy body:w12 maxEdits=1 transpositions=true prefixLength=1".to_string(),
            Box::new(|r| fuzzy("body", "w12", 1, true, 1, r)),
        ),
        (
            "fuzzy body:w21 maxEdits=1 transpositions=false prefixLength=0".to_string(),
            Box::new(|r| fuzzy("body", "w21", 1, false, 0, r)),
        ),
        (
            "range body:[w20 TO w29]".to_string(),
            Box::new(|r| range("body", Some("w20"), Some("w29"), true, true, r)),
        ),
        (
            "terms body:{w3 w33}".to_string(),
            Box::new(|r| terms_in_set("body", &["w3", "w33"], r)),
        ),
    ]
}

/// `MultiTermQueryFixture.intersections()`: name, field, automaton, start
/// term.
fn intersections() -> Vec<(&'static str, &'static str, Automaton, Option<BytesRef>)> {
    vec![
        ("kw a.*e", "kw", regexp("a.*e"), None),
        (
            "kw a.*e after ample",
            "kw",
            regexp("a.*e"),
            Some(bytes("ample")),
        ),
        (
            "kw wildcard ?erry",
            "kw",
            WildcardQuery::to_automaton(&term("kw", "?erry"), DEFAULT_DETERMINIZE_WORK_LIMIT)
                .unwrap(),
            None,
        ),
        ("body w[1-3]?[05]", "body", regexp("w[1-3]?[05]"), None),
        (
            "body w1.* after w15",
            "body",
            regexp("w1.*"),
            Some(bytes("w15")),
        ),
    ]
}

fn print_query(
    out: &mut String,
    searcher: &IndexSearcher,
    stored: &dyn StoredFields,
    name: &str,
    rewrite: &str,
    query: Arc<dyn Query>,
) {
    let top = searcher.search(query, TOP_N).unwrap();
    writeln!(
        out,
        "query {name} rewrite={rewrite} total={} {}",
        top.total_hits.value(),
        relation(&top.total_hits)
    )
    .unwrap();
    for hit in &top.score_docs {
        writeln!(
            out,
            "  hit doc={} score={} id={}",
            hit.doc,
            bits(hit.score),
            stored_id(stored, hit.doc)
        )
        .unwrap();
    }
}

/// Rucene's run of the query set over the index at `path`: the same lines
/// `MultiTermQueryFixture query` prints.
fn query_report(path: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    let stored = stored.as_ref();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    let mut out = String::new();
    writeln!(
        out,
        "reader maxDoc={} numDocs={}",
        reader.max_doc(),
        reader.num_docs()
    )
    .unwrap();
    for ctx in &leaves {
        let leaf = ctx.leaf_reader();
        writeln!(
            out,
            "leaf ord={} docBase={} maxDoc={} numDocs={}",
            ctx.ord(),
            ctx.doc_base(),
            leaf.max_doc(),
            leaf.num_docs()
        )
        .unwrap();
    }
    for (name, factory) in keyword_queries() {
        for (rewrite, method) in keyword_rewrites() {
            print_query(&mut out, &searcher, stored, &name, rewrite, factory(method));
        }
    }
    for (name, factory) in text_queries() {
        for (rewrite, method) in text_rewrites() {
            print_query(&mut out, &searcher, stored, &name, rewrite, factory(method));
        }
    }
    for (name, field, automaton, start) in intersections() {
        let compiled = CompiledAutomaton::new(automaton, false, true, false).unwrap();
        for ctx in &leaves {
            let mut line = format!("intersect {name} leaf={}:", ctx.ord());
            let terms = ctx
                .leaf_reader()
                .terms(field)
                .unwrap()
                .expect("the field has terms in every leaf");
            let mut te = terms.intersect(&compiled, start.as_ref()).unwrap();
            while let Some(t) = te.next().unwrap() {
                write!(
                    line,
                    " {}={}",
                    String::from_utf8_lossy(t.slice()),
                    te.doc_freq().unwrap()
                )
                .unwrap();
            }
            writeln!(out, "{line}").unwrap();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Java oracle
// ---------------------------------------------------------------------------

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Runs `MultiTermQueryFixture <mode> <dir>`: `(success, stdout, stderr)`.
fn run_java(mode: &str, dir: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         MultiTermQueryFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(mode)
        .arg(dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The stderr lines of a Java run without the JVM's start-up warnings.
fn java_errors(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            !(l.starts_with("WARNING:")
                || l.contains("VectorizationProvider lookup")
                || l.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// The first differing lines of two reports, or `None` when they agree.
fn diff(left_name: &str, left: &str, right_name: &str, right: &str) -> Option<String> {
    if left == right {
        return None;
    }
    let l: Vec<&str> = left.lines().collect();
    let r: Vec<&str> = right.lines().collect();
    let mut out = format!(
        "{left_name} ({} lines) differs from {right_name} ({} lines):\n",
        l.len(),
        r.len()
    );
    let mut shown = 0;
    let mut context = "";
    for k in 0..l.len().max(r.len()) {
        let a = l.get(k).copied().unwrap_or("<end>");
        let b = r.get(k).copied().unwrap_or("<end>");
        if a.starts_with("query ") || a.starts_with("intersect ") {
            context = a;
        }
        if a != b {
            writeln!(
                out,
                "  line {} [in {context}]\n    {left_name}: {a}\n    {right_name}: {b}",
                k + 1
            )
            .unwrap();
            shown += 1;
            if shown == 40 {
                out.push_str("  ...\n");
                break;
            }
        }
    }
    Some(out)
}

/// The `reader` and `leaf` lines of a report: the document counts.
fn count_lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|l| l.starts_with("reader ") || l.starts_with("leaf "))
        .collect()
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".to_string())
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn multi_term_queries_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-multi-term-queries")
        .tempdir()
        .unwrap();
    let r = tmp.path().join("rucene");
    let l = tmp.path().join("lucene");
    let p = tmp.path().join("rucene-perturbed");
    for dir in [&r, &l, &p] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    // Both sides write the corpus.
    write_corpus(&r, false);
    let (ok, _, err) = run_java("write", &l);
    assert!(ok, "Lucene failed to write the corpus:\n{err}");

    // The query set, from both sides over both indexes.
    let mut java_query = |dir: &Path, name: &str| -> Option<String> {
        let (ok, out, err) = run_java("query", dir);
        if ok {
            Some(out)
        } else {
            failures.push(format!("{name} failed:\n{out}\n{}", java_errors(&err)));
            None
        }
    };
    let lucene_on_r = java_query(&r, "Lucene query set on R");
    let lucene_on_l = java_query(&l, "Lucene query set on L");
    let rust_query = |dir: &Path| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| query_report(dir)))
            .map_err(panic_message)
    };
    let rucene_on_r = rust_query(&r)
        .map_err(|e| failures.push(format!("Rucene query set on R panicked: {e}")))
        .ok();
    let rucene_on_l = rust_query(&l)
        .map_err(|e| failures.push(format!("Rucene query set on L panicked: {e}")))
        .ok();

    // The oracle's own report must exercise every query and every
    // intersection: an empty result would make the comparisons vacuous.
    let oracle = lucene_on_l
        .as_ref()
        .expect("the Lucene report on L is required");
    let expected_queries = keyword_queries().len() * keyword_rewrites().len()
        + text_queries().len() * text_rewrites().len();
    let mut query_count = 0;
    let mut lines = oracle.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("query ") {
            query_count += 1;
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  hit ")),
                "the oracle query has no hits: {line}"
            );
        } else if line.starts_with("intersect ") {
            assert!(
                line.contains('='),
                "the oracle intersection returns no term: {line}"
            );
        }
    }
    assert_eq!(
        query_count, expected_queries,
        "the oracle must run every query under every rewrite"
    );
    let leaf_count = oracle.lines().filter(|l| l.starts_with("leaf ")).count();
    assert!(
        leaf_count >= 2,
        "the corpus must have several segments: {leaf_count}"
    );
    writeln!(
        report,
        "== oracle report ({} lines, {query_count} queries)",
        oracle.lines().count()
    )
    .unwrap();

    let pairs = [
        ("Lucene on R", &lucene_on_r, "Rucene on R", &rucene_on_r),
        ("Lucene on L", &lucene_on_l, "Rucene on L", &rucene_on_l),
        ("Lucene on L", &lucene_on_l, "Lucene on R", &lucene_on_r),
        ("Rucene on L", &rucene_on_l, "Rucene on R", &rucene_on_r),
    ];
    for (a_name, a, b_name, b) in pairs {
        if let (Some(a), Some(b)) = (a, b) {
            if let Some(d) = diff(a_name, a, b_name, b) {
                failures.push(d);
            } else {
                writeln!(report, "== {a_name} == {b_name}").unwrap();
            }
        }
    }

    // Positive control: one token and one keyword more in document 0 must
    // change what the comparison sees, with the same document counts.
    write_corpus(&p, true);
    let perturbed =
        rust_query(&p).unwrap_or_else(|e| panic!("Rucene query set on P panicked: {e}"));
    assert_eq!(
        count_lines(&perturbed),
        count_lines(oracle),
        "the perturbed corpus keeps the document counts"
    );
    let control = diff("Lucene on L", oracle, "Rucene on P", &perturbed);
    assert!(
        control.is_some(),
        "the comparison must fail on the perturbed corpus"
    );
    writeln!(
        report,
        "== positive control: Lucene on L != Rucene on P\n{}",
        control
            .unwrap()
            .lines()
            .take(4)
            .collect::<Vec<_>>()
            .join("\n")
    )
    .unwrap();

    println!("{report}");
    assert!(
        failures.is_empty(),
        "{} multi-term query failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
