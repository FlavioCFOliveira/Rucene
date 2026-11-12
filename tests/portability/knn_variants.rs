//! kNN query variants against Apache Lucene Core 10.5.0 (rmp task #411).
//!
//! The corpus is defined by the same integer formulas here and in
//! `KnnVariantsFixture.java`: three commits of 800, 800 and 300 documents —
//! the first two segments are large enough for Lucene to build an HNSW graph
//! for both vector fields (`Lucene99HnswVectorsWriter.shouldCreateGraph`: more
//! nodes than `ln(n) * HNSW_GRAPH_THRESHOLD`; 720 float and 686 byte vectors
//! per segment), the last is not — with a float vector field `fvec`
//! (EUCLIDEAN, quarter-integer components) and a byte vector field `bvec`
//! (DOT_PRODUCT), each missing from some documents, keyword
//! fields for seeds and filters, and deletions in the first and the last
//! segment. Rucene writes it under `R`, Lucene under `L`.
//!
//! * The query set (`KnnVariantsFixture.queries`, [`queries`]) runs on both
//!   roots from both sides: `KnnFloatVectorQuery`/`KnnByteVectorQuery`,
//!   `SeededKnnVectorQuery.fromFloatQuery`/`fromByteQuery` (seed = a term
//!   query, with and without a filter; Lucene 10.5.0 has no
//!   `SeededKnnFloatVectorQuery`/`SeededKnnByteVectorQuery` classes),
//!   `PatienceKnnVectorQuery.fromFloatQuery`/`fromByteQuery`/`fromSeededQuery`
//!   (default and explicit saturation threshold and patience),
//!   `FloatVectorSimilarityQuery`/`ByteVectorSimilarityQuery` (result
//!   similarity with the default decay, decay 0 and decay 1, with and without a
//!   filter; Lucene 10.5.0 replaced the traversal similarity by a decay),
//!   `RescoreTopNQuery.createFullPrecisionRescorerQuery` over a kNN query and
//!   over a term query, and the exact-search fallbacks of
//!   `AbstractKnnVectorQuery.getLeafResults`: a filter whose cost is below the
//!   per-leaf top k (exact search at once) and a filter whose cost exceeds the
//!   nodes the graph search is expected to visit, so the filtered graph search
//!   runs with a visit limit, reaches it and falls back to exact search. Per query: total hits value and relation; per hit: doc id,
//!   score bits and id. The four reports are compared line by line.
//! * `KnnVariantsFixture probe` reruns the fallback queries through
//!   subclasses that record every `approximateSearch` and `exactSearch` call;
//!   the test asserts that Lucene took both fallback paths on this corpus, so
//!   the comparison of those queries is not vacuous.
//! * Positive control: Rucene writes a perturbed corpus (document 123's vectors
//!   change in one component); its report must differ from Lucene's report on
//!   `L` with the same document counts.
//!
//! The oracle runs with `-Dlucene.useScalarFMA=false` and without the
//! incubator vector module, so Lucene computes every float score with the
//! scalar, non-fused path Rucene reproduces; the first report line records it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, KnnByteVectorField, KnnFloatVectorField, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexReader, StoredFields, Term, VectorSimilarityFunction};
use rucene::search::{
    AbstractKnnVectorQueryImpl, BooleanQueryBuilder, ByteVectorSimilarityQuery,
    FloatVectorSimilarityQuery, IndexSearcher, KnnByteVectorQuery, KnnFloatVectorQuery, Occur,
    PatienceKnnVectorQuery, Query, RescoreTopNQuery, SeededKnnVectorQuery, TermQuery, TotalHits,
    TotalHitsRelation,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// Corpus: the formulas of KnnVariantsFixture
// ---------------------------------------------------------------------------

/// `KnnVariantsFixture.BATCHES`.
const BATCHES: [(i32, i32); 3] = [(0, 800), (800, 1600), (1600, 1900)];

/// `KnnVariantsFixture.DIM`.
const DIM: i32 = 8;

/// `KnnVariantsFixture.TOP_N`.
const TOP_N: i32 = 20;

/// The document whose vectors the perturbed corpus changes.
const PERTURBED_DOC: i32 = 123;

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

/// `KnnVariantsFixture.fvec(int)`: quarter-integer components, so every
/// product and sum is exact.
fn fvec(i: i32) -> Vec<f32> {
    (0..DIM)
        .map(|j| ((i * i * 7 + i * 13 + j * j * 5 + i * j * 3) % 251 - 125) as f32 / 4.0)
        .collect()
}

/// `KnnVariantsFixture.bvec(int)`, Java's signed bytes as their two's
/// complement `u8`.
fn bvec(i: i32) -> Vec<u8> {
    (0..DIM)
        .map(|j| ((i * i * 3 + j * 11 + i * j * 7) % 256 - 128) as i8 as u8)
        .collect()
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

/// `KnnVariantsFixture.doc(int)`; the perturbed corpus moves document 123's
/// float vector by a quarter in its first component and flips the low bit of
/// its byte vector's first component.
fn doc(i: i32, perturbed: bool) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    d.add(Box::new(
        StringField::new("tag", format!("t{}", i % 10), Store::NO).unwrap(),
    ));
    d.add(Box::new(
        StringField::new("g", format!("g{}", i % 20), Store::NO).unwrap(),
    ));
    if i % 97 == 0 {
        d.add(Box::new(
            StringField::new("rare", "yes".to_string(), Store::NO).unwrap(),
        ));
    }
    if i % 10 != 9 {
        let mut v = fvec(i);
        if perturbed && i == PERTURBED_DOC {
            v[0] += 0.25;
        }
        d.add(Box::new(
            KnnFloatVectorField::new("fvec", &v, VectorSimilarityFunction::EUCLIDEAN).unwrap(),
        ));
    }
    if i % 7 != 6 {
        let mut v = bvec(i);
        if perturbed && i == PERTURBED_DOC {
            v[0] ^= 1;
        }
        d.add(Box::new(
            KnnByteVectorField::new("bvec", &v, VectorSimilarityFunction::DOT_PRODUCT).unwrap(),
        ));
    }
    d
}

/// `KnnVariantsFixture.write(String)`: the same operations, in order.
fn write_corpus(path: &Path, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    std::fs::create_dir_all(path).unwrap();
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
            for deleted in [7, 42, 100] {
                w.delete_documents_by_terms(vec![term("id", &id(deleted))])
                    .unwrap();
            }
        } else if b == 2 {
            w.delete_documents_by_terms(vec![term("id", &id(1650))])
                .unwrap();
        }
        w.commit().unwrap();
    }
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Query set: KnnVariantsFixture.queries()
// ---------------------------------------------------------------------------

/// `KnnVariantsFixture.TARGET_X`: a float target that is no document's
/// vector.
const TARGET_X: [f32; 8] = [0.5, -1.25, 3.0, 7.75, -6.0, 2.5, 0.0, -0.25];

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `KnnVariantsFixture.tags012()`: `tag:t0 tag:t1 tag:t2`, as optional
/// clauses.
fn tags012() -> Arc<dyn Query> {
    let mut b = BooleanQueryBuilder::new();
    for t in ["t0", "t1", "t2"] {
        b.add(tq("tag", t), Occur::SHOULD).unwrap();
    }
    Arc::new(b.build())
}

fn knn_float(target: &[f32], k: i32, filter: Option<Arc<dyn Query>>) -> KnnFloatVectorQuery {
    KnnFloatVectorQuery::with_filter("fvec", target.to_vec(), k, filter).unwrap()
}

fn knn_byte(target: &[u8], k: i32, filter: Option<Arc<dyn Query>>) -> KnnByteVectorQuery {
    KnnByteVectorQuery::with_filter("bvec", target.to_vec(), k, filter).unwrap()
}

/// A kNN query as the delegate of a seeded or patience query.
fn delegate<Q: AbstractKnnVectorQueryImpl + 'static>(q: Q) -> Arc<dyn AbstractKnnVectorQueryImpl> {
    Arc::new(q)
}

fn arc<Q: Query + 'static>(q: Q) -> Arc<dyn Query> {
    Arc::new(q)
}

/// `KnnVariantsFixture.queries()`, in the same order. Java's one-argument
/// `PatienceKnnVectorQuery.fromFloatQuery`/`fromByteQuery`/`fromSeededQuery`
/// (the default saturation threshold and patience) are
/// `PatienceKnnVectorQuery::from_query`.
fn queries() -> Vec<(&'static str, Arc<dyn Query>)> {
    let ta = fvec(123);
    let tb = fvec(1233);
    let tc = fvec(1777);
    let ba = bvec(123);
    let bb = bvec(1233);
    let mut q: Vec<(&'static str, Arc<dyn Query>)> = vec![
        ("knn-float k=10 A", arc(knn_float(&ta, 10, None))),
        ("knn-float k=10 X", arc(knn_float(&TARGET_X, 10, None))),
        ("knn-byte k=10 A", arc(knn_byte(&ba, 10, None))),
        (
            "seeded-float k=10 X seed=tag:t3",
            arc(SeededKnnVectorQuery::from_float_query(
                delegate(knn_float(&TARGET_X, 10, None)),
                tq("tag", "t3"),
            )
            .unwrap()),
        ),
        (
            "seeded-float k=5 A filter=g:g3 seed=tag:t7",
            arc(SeededKnnVectorQuery::from_float_query(
                delegate(knn_float(&ta, 5, Some(tq("g", "g3")))),
                tq("tag", "t7"),
            )
            .unwrap()),
        ),
        (
            "seeded-byte k=10 B seed=tag:t1",
            arc(SeededKnnVectorQuery::from_byte_query(
                delegate(knn_byte(&bb, 10, None)),
                tq("tag", "t1"),
            )
            .unwrap()),
        ),
        (
            "patience-float default k=10 X",
            arc(
                PatienceKnnVectorQuery::from_query(delegate(knn_float(&TARGET_X, 10, None)))
                    .unwrap(),
            ),
        ),
        (
            "patience-float 0.9/5 k=10 A",
            arc(PatienceKnnVectorQuery::from_float_query(
                delegate(knn_float(&ta, 10, None)),
                0.9,
                5,
            )
            .unwrap()),
        ),
        (
            "patience-byte default k=10 A",
            arc(PatienceKnnVectorQuery::from_query(delegate(knn_byte(&ba, 10, None))).unwrap()),
        ),
        (
            "patience-byte 0.99/10 k=20 B",
            arc(PatienceKnnVectorQuery::from_byte_query(
                delegate(knn_byte(&bb, 20, None)),
                0.99,
                10,
            )
            .unwrap()),
        ),
        (
            "patience-seeded default k=10 X seed=tag:t5",
            arc(PatienceKnnVectorQuery::from_query(delegate(
                SeededKnnVectorQuery::from_float_query(
                    delegate(knn_float(&TARGET_X, 10, None)),
                    tq("tag", "t5"),
                )
                .unwrap(),
            ))
            .unwrap()),
        ),
        (
            "fsim A result=0.5",
            arc(FloatVectorSimilarityQuery::new("fvec", ta.clone(), 0.5).unwrap()),
        ),
        (
            "fsim A result=0.002 decay=0",
            arc(
                FloatVectorSimilarityQuery::with_decay("fvec", ta.clone(), 0.002, 0.0, None)
                    .unwrap(),
            ),
        ),
        (
            "fsim A result=0.002 decay=1",
            arc(
                FloatVectorSimilarityQuery::with_decay("fvec", ta.clone(), 0.002, 1.0, None)
                    .unwrap(),
            ),
        ),
        (
            "fsim A result=0.002 filter=g:g3",
            arc(FloatVectorSimilarityQuery::with_filter(
                "fvec",
                ta.clone(),
                0.002,
                Some(tq("g", "g3")),
            )
            .unwrap()),
        ),
        (
            "fsim C result=0.01",
            arc(FloatVectorSimilarityQuery::new("fvec", tc, 0.01).unwrap()),
        ),
        (
            "fsim B result=0.002",
            arc(FloatVectorSimilarityQuery::new("fvec", tb, 0.002).unwrap()),
        ),
        (
            "bsim A result=0.55",
            arc(ByteVectorSimilarityQuery::new("bvec", ba.clone(), 0.55).unwrap()),
        ),
        (
            "bsim A result=0.55 decay=1",
            arc(
                ByteVectorSimilarityQuery::with_decay("bvec", ba.clone(), 0.55, 1.0, None).unwrap(),
            ),
        ),
        (
            "bsim B result=0.55 filter=tag:t3",
            arc(
                ByteVectorSimilarityQuery::with_filter("bvec", bb, 0.55, Some(tq("tag", "t3")))
                    .unwrap(),
            ),
        ),
        (
            "rescore knn-float k=20 X n=5",
            RescoreTopNQuery::create_full_precision_rescorer_query(
                arc(knn_float(&TARGET_X, 20, None)),
                TARGET_X.to_vec(),
                "fvec",
                5,
            )
            .unwrap(),
        ),
        (
            "rescore tag:t4 A n=7",
            RescoreTopNQuery::create_full_precision_rescorer_query(tq("tag", "t4"), ta, "fvec", 7)
                .unwrap(),
        ),
    ];
    q.extend(fallback_queries());
    q
}

/// The fallback query names, in `KnnVariantsFixture.fallbackQueries` order.
const FALLBACK_SHORT: &str = "exact-short knn-float k=10 A filter=rare:yes";
const FALLBACK_FLOAT: &str = "exact-fallback knn-float k=10 X filter=tag:t0|t1|t2";
const FALLBACK_BYTE: &str = "exact-fallback knn-byte k=10 A filter=tag:t0|t1|t2";

/// `KnnVariantsFixture.fallbackQueries(false)`.
fn fallback_queries() -> Vec<(&'static str, Arc<dyn Query>)> {
    let ta = fvec(123);
    let ba = bvec(123);
    vec![
        (
            FALLBACK_SHORT,
            arc(knn_float(&ta, 10, Some(tq("rare", "yes")))),
        ),
        (
            FALLBACK_FLOAT,
            arc(knn_float(&TARGET_X, 10, Some(tags012()))),
        ),
        (FALLBACK_BYTE, arc(knn_byte(&ba, 10, Some(tags012())))),
    ]
}

// ---------------------------------------------------------------------------
// Report: the output of KnnVariantsFixture.query, line for line
// ---------------------------------------------------------------------------

/// The first line of the oracle's report under the required JVM flags.
const ENV_LINE: &str = "env hasFastScalarFma=false incubatorVector=false";

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

/// `KnnVariantsFixture.printReader`.
fn print_reader(out: &mut String, reader: &Arc<dyn IndexReader>) {
    writeln!(
        out,
        "reader maxDoc={} numDocs={}",
        reader.max_doc(),
        reader.num_docs()
    )
    .unwrap();
    for ctx in &Arc::clone(reader).leaves().unwrap() {
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
}

/// Rucene's run of the query set over the index at `path`: the same lines
/// `KnnVariantsFixture query` prints.
fn query_report(path: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let mut out = String::new();
    writeln!(out, "{ENV_LINE}").unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    print_reader(&mut out, &reader);
    for (name, query) in queries() {
        let top = searcher
            .search(query, TOP_N)
            .unwrap_or_else(|e| panic!("query {name}: {e:?}"));
        writeln!(
            out,
            "query {name} total={} {}",
            top.total_hits.value(),
            relation(&top.total_hits)
        )
        .unwrap();
        for hit in top.score_docs.iter() {
            writeln!(
                out,
                "  hit doc={} score={} id={}",
                hit.doc,
                bits(hit.score),
                stored_id(stored.as_ref(), hit.doc)
            )
            .unwrap();
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

/// Runs `KnnVariantsFixture <mode> <root>` with Lucene's scalar, non-fused
/// vector path: `(success, stdout, stderr)`.
fn run_java(mode: &str, root: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         KnnVariantsFixture.java",
    );
    let output = Command::new("java")
        .arg("-Dlucene.useScalarFMA=false")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(mode)
        .arg(root)
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
        if a.starts_with("query ") {
            context = a;
        }
        if a != b {
            writeln!(
                out,
                "  line {} [{context}]\n    {left_name}: {a}\n    {right_name}: {b}",
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
fn reader_lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|l| l.starts_with("reader ") || l.starts_with("leaf "))
        .collect()
}

/// The probe lines of one fallback query: `leaf=<ord> exact` or
/// `leaf=<ord> approximate visitedLimit=<n> ...`.
fn probe_calls<'a>(probe: &'a str, name: &str) -> Vec<&'a str> {
    let prefix = format!("probe {name} | ");
    probe
        .lines()
        .filter_map(|l| l.strip_prefix(prefix.as_str()))
        .collect()
}

/// Whether some leaf ran a graph search bounded by a visit limit and then an
/// exact search: the visit-limit fallback of
/// `AbstractKnnVectorQuery.getLeafResults`.
fn has_visit_limit_fallback(calls: &[&str]) -> bool {
    calls.windows(2).any(|w| {
        let leaf = w[0].split_whitespace().next().unwrap_or("");
        w[0].contains(" approximate visitedLimit=")
            && !w[0].contains(&format!("visitedLimit={}", i32::MAX))
            && w[1] == format!("{leaf} exact")
    })
}

/// Whether some leaf ran an exact search without a graph search before it:
/// the short-circuit for a filter cost at most the per-leaf top k.
fn has_short_circuit(calls: &[&str]) -> bool {
    calls.iter().enumerate().any(|(k, call)| {
        call.ends_with(" exact") && {
            let leaf = call.split_whitespace().next().unwrap_or("");
            k == 0 || !calls[k - 1].starts_with(&format!("{leaf} approximate"))
        }
    })
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".to_string())
}

/// Puts the Rust side under the switch the JVM oracle gets: the environment
/// variable `lucene.useScalarFMA`, this port's form of the
/// `-Dlucene.useScalarFMA` system property (`Constants.getSysProp`), set to
/// `false` before any vector computation reads `Constants.HAS_FAST_SCALAR_FMA`
/// (`rucene::util::vector_util::use_fma`), which is evaluated once.
fn pin_scalar_fma_off() {
    static PIN: std::sync::Once = std::sync::Once::new();
    PIN.call_once(|| std::env::set_var("lucene.useScalarFMA", "false"));
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn knn_query_variants_match_lucene() {
    pin_scalar_fma_off();
    let tmp = tempfile::Builder::new()
        .prefix("rucene-knn-variants")
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

    // The query set, from both sides over both roots.
    let mut java_query = |root: &Path, name: &str| -> Option<String> {
        let (ok, out, err) = run_java("query", root);
        if ok {
            Some(out)
        } else {
            failures.push(format!("{name} failed:\n{out}\n{}", java_errors(&err)));
            None
        }
    };
    let lucene_on_r = java_query(&r, "Lucene query set on R");
    let lucene_on_l = java_query(&l, "Lucene query set on L");
    let rust_query = |root: &Path| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| query_report(root)))
            .map_err(panic_message)
    };
    let rucene_on_r = rust_query(&r)
        .map_err(|e| failures.push(format!("Rucene query set on R panicked: {e}")))
        .ok();
    let rucene_on_l = rust_query(&l)
        .map_err(|e| failures.push(format!("Rucene query set on L panicked: {e}")))
        .ok();

    // The oracle must run on the deterministic vector path, and every query
    // must have hits: an empty result would make the comparison vacuous.
    let oracle = lucene_on_l
        .as_ref()
        .expect("the Lucene report on L is required");
    assert_eq!(
        oracle.lines().next(),
        Some(ENV_LINE),
        "the oracle must run with -Dlucene.useScalarFMA=false and without jdk.incubator.vector"
    );
    assert_eq!(
        oracle.lines().filter(|l| l.starts_with("leaf ")).count(),
        BATCHES.len(),
        "the corpus must keep one segment per commit"
    );
    let mut lines = oracle.lines().peekable();
    let mut query_count = 0;
    while let Some(line) = lines.next() {
        if line.starts_with("query ") {
            query_count += 1;
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  hit ")),
                "the oracle query has no hits: {line}"
            );
        }
    }
    assert_eq!(
        query_count,
        queries().len(),
        "the oracle must report every query"
    );

    // Lucene must take both exact-search fallbacks on this corpus.
    let (ok, probe, err) = run_java("probe", &l);
    assert!(
        ok,
        "the Lucene probe failed:\n{probe}\n{}",
        java_errors(&err)
    );
    let short = probe_calls(&probe, FALLBACK_SHORT);
    assert!(
        has_short_circuit(&short),
        "{FALLBACK_SHORT}: no leaf short-circuited to exact search:\n{}",
        short.join("\n")
    );
    for name in [FALLBACK_FLOAT, FALLBACK_BYTE] {
        let calls = probe_calls(&probe, name);
        assert!(
            has_visit_limit_fallback(&calls),
            "{name}: no leaf fell back to exact search after a bounded graph search:\n{}",
            calls.join("\n")
        );
    }
    writeln!(
        report,
        "== oracle report ({} lines, {query_count} queries); probe:\n{}",
        oracle.lines().count(),
        probe.trim_end()
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

    // Positive control: moving one document's vectors must change the
    // report, with the same document counts.
    write_corpus(&p, true);
    let perturbed =
        rust_query(&p).unwrap_or_else(|e| panic!("Rucene query set on P panicked: {e}"));
    assert_eq!(
        reader_lines(&perturbed),
        reader_lines(oracle),
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
        "{} kNN variant failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
