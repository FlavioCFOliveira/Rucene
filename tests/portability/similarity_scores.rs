//! Similarity scores and explanations against Apache Lucene Core 10.5.0
//! (rmp task #404).
//!
//! The scoring corpus is defined by the same integer formulas here and in
//! `SimilarityScoresFixture.java` (it is shared with `multi_term_queries.rs`
//! and `scoring_queries.rs`). For every similarity of [`similarities`] the
//! corpus is indexed with that similarity set on the writer — norms depend on
//! it — into `<root>/<name>`, by Rucene under `R` and by Lucene under `L`, and
//! searched with the same similarity set on the searcher.
//!
//! * The query set (`SimilarityScoresFixture.query`, [`query_report`]) runs on
//!   both roots from both sides: per hit, the score bits
//!   (`Float.floatToIntBits`) and, for the first hits, the `explain()` value
//!   bits and text. The four reports are compared line by line: the same
//!   reader over the same index must agree exactly, and each side's report on
//!   its own index must agree with the other side's on its own index.
//! * Positive control: Rucene writes a perturbed corpus (document 0 gains the
//!   body token `w7` and the keyword `zebra`); its report must differ from
//!   Lucene's report on `L` in the scores, not in the document counts.

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
use rucene::index::{FieldInvertState, IndexOptions, IndexReader, StoredFields, Term};
use rucene::search::{
    per_field_compute_norm, per_field_scorer, AfterEffectB, AfterEffectL, AxiomaticF1EXP,
    AxiomaticF1LOG, AxiomaticF2EXP, AxiomaticF2LOG, AxiomaticF3EXP, AxiomaticF3LOG, BM25Similarity,
    BasicModelG, BasicModelIF, BasicModelIn, BasicModelIne, BooleanQueryBuilder, BooleanSimilarity,
    ClassicSimilarity, CollectionStatistics, DFISimilarity, DFRSimilarity, DistributionLL,
    DistributionSPL, IBSimilarity, IndependenceChiSquared, IndependenceSaturated,
    IndependenceStandardized, IndexSearcher, IndriDirichletSimilarity, LMDirichletSimilarity,
    LMJelinekMercerSimilarity, LambdaDF, LambdaTTF, MultiSimilarity, NoNormalization,
    NormalizationH1, NormalizationH2, NormalizationH3, NormalizationZ, Occur,
    PerFieldSimilarityWrapper, PhraseQuery, Query, RawTFSimilarity, SimScorer, Similarity,
    TermQuery, TermStatistics, TotalHits, TotalHitsRelation,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// Scoring corpus: the formulas of SimilarityScoresFixture
// ---------------------------------------------------------------------------

/// `SimilarityScoresFixture.BATCHES`.
const BATCHES: [(i32, i32); 3] = [(0, 100), (100, 200), (200, 300)];

/// `SimilarityScoresFixture.WORDS`.
const WORDS: [&str; 20] = [
    "apple", "apply", "ample", "maple", "apricot", "banana", "bandana", "cabana", "cherry",
    "sherry", "berry", "merry", "date", "data", "gate", "grape", "graph", "form", "from", "zebra",
];

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

/// `SimilarityScoresFixture.body(int)`; the perturbed corpus appends `w7` to
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

/// `SimilarityScoresFixture.TEXT_TYPE`.
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

/// `SimilarityScoresFixture.doc(int)`; the perturbed corpus gives document 0
/// a third keyword, `zebra`.
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

/// `SimilarityScoresFixture.write(Directory, Similarity)`: the same
/// operations, in order.
fn write_corpus(path: &Path, similarity: Arc<dyn Similarity>, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    std::fs::create_dir_all(path).unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config.set_similarity(Some(similarity)).unwrap();
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
// Similarities: SimilarityScoresFixture.similarities()
// ---------------------------------------------------------------------------

/// `SimilarityScoresFixture.PerFieldSim`: `body` with BM25(1.5, 0.5), every
/// other field with `ClassicSimilarity`.
#[derive(Debug)]
struct PerFieldSim {
    body: BM25Similarity,
    other: ClassicSimilarity,
}

impl PerFieldSim {
    fn new() -> Self {
        Self {
            body: BM25Similarity::with_parameters(1.5, 0.5).unwrap(),
            other: ClassicSimilarity::new(),
        }
    }
}

impl Similarity for PerFieldSim {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.rucene.codec.SimilarityScoresFixture$PerFieldSim"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> rucene::error::Result<i64> {
        per_field_compute_norm(self, state)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        per_field_scorer(self, boost, collection_stats, term_stats)
    }
}

impl PerFieldSimilarityWrapper for PerFieldSim {
    fn get(&self, name: &str) -> &dyn Similarity {
        if name == "body" {
            &self.body
        } else {
            &self.other
        }
    }
}

type SimilarityFactory = fn() -> Arc<dyn Similarity>;

/// `SimilarityScoresFixture.similarities()`, in the same order.
fn similarities() -> Vec<(&'static str, SimilarityFactory)> {
    vec![
        ("classic", || Arc::new(ClassicSimilarity::new())),
        ("bm25-default", || Arc::new(BM25Similarity::new())),
        ("bm25-1.5-0.5", || {
            Arc::new(BM25Similarity::with_parameters(1.5, 0.5).unwrap())
        }),
        ("boolean", || Arc::new(BooleanSimilarity::new())),
        ("dfr-G-B-H2", || {
            Arc::new(DFRSimilarity::new(
                Box::new(BasicModelG::new()),
                Box::new(AfterEffectB::new()),
                Box::new(NormalizationH2::default()),
            ))
        }),
        ("dfr-In-L-H1", || {
            Arc::new(DFRSimilarity::new(
                Box::new(BasicModelIn::new()),
                Box::new(AfterEffectL::new()),
                Box::new(NormalizationH1::default()),
            ))
        }),
        ("dfr-IF-B-H3", || {
            Arc::new(DFRSimilarity::new(
                Box::new(BasicModelIF::new()),
                Box::new(AfterEffectB::new()),
                Box::new(NormalizationH3::default()),
            ))
        }),
        ("dfr-Ine-L-Z", || {
            Arc::new(DFRSimilarity::new(
                Box::new(BasicModelIne::new()),
                Box::new(AfterEffectL::new()),
                Box::new(NormalizationZ::default()),
            ))
        }),
        ("dfr-G-L-none", || {
            Arc::new(DFRSimilarity::new(
                Box::new(BasicModelG::new()),
                Box::new(AfterEffectL::new()),
                Box::new(NoNormalization::new()),
            ))
        }),
        ("ib-LL-DF-H2", || {
            Arc::new(IBSimilarity::new(
                Box::new(DistributionLL::new()),
                Box::new(LambdaDF::new()),
                Box::new(NormalizationH2::default()),
            ))
        }),
        ("ib-SPL-TTF-H3", || {
            Arc::new(IBSimilarity::new(
                Box::new(DistributionSPL::new()),
                Box::new(LambdaTTF::new()),
                Box::new(NormalizationH3::default()),
            ))
        }),
        ("ib-LL-TTF-Z", || {
            Arc::new(IBSimilarity::new(
                Box::new(DistributionLL::new()),
                Box::new(LambdaTTF::new()),
                Box::new(NormalizationZ::default()),
            ))
        }),
        ("ib-SPL-DF-H1", || {
            Arc::new(IBSimilarity::new(
                Box::new(DistributionSPL::new()),
                Box::new(LambdaDF::new()),
                Box::new(NormalizationH1::default()),
            ))
        }),
        ("lm-dirichlet-default", || {
            Arc::new(LMDirichletSimilarity::new())
        }),
        ("lm-dirichlet-500", || {
            Arc::new(LMDirichletSimilarity::with_mu(500.0).unwrap())
        }),
        ("lm-jm-0.7", || {
            Arc::new(LMJelinekMercerSimilarity::with_lambda(0.7).unwrap())
        }),
        ("lm-jm-0.1", || {
            Arc::new(LMJelinekMercerSimilarity::with_lambda(0.1).unwrap())
        }),
        (
            "indri-default",
            || Arc::new(IndriDirichletSimilarity::new()),
        ),
        ("indri-1000", || {
            Arc::new(IndriDirichletSimilarity::with_mu(1000.0))
        }),
        ("axiomatic-F1EXP", || Arc::new(AxiomaticF1EXP::new())),
        ("axiomatic-F1LOG", || Arc::new(AxiomaticF1LOG::new())),
        ("axiomatic-F2EXP", || Arc::new(AxiomaticF2EXP::new())),
        ("axiomatic-F2LOG", || Arc::new(AxiomaticF2LOG::new())),
        ("axiomatic-F3EXP", || {
            Arc::new(AxiomaticF3EXP::with_s_and_query_len(0.25, 3).unwrap())
        }),
        ("axiomatic-F3LOG", || {
            Arc::new(AxiomaticF3LOG::with_s_and_query_len(0.25, 3).unwrap())
        }),
        ("dfi-standardized", || {
            Arc::new(DFISimilarity::new(
                Box::new(IndependenceStandardized::new()),
            ))
        }),
        ("dfi-saturated", || {
            Arc::new(DFISimilarity::new(Box::new(IndependenceSaturated::new())))
        }),
        ("dfi-chisquared", || {
            Arc::new(DFISimilarity::new(Box::new(IndependenceChiSquared::new())))
        }),
        ("rawtf", || Arc::new(RawTFSimilarity::new())),
        ("multi-bm25-classic", || {
            Arc::new(MultiSimilarity::new(vec![
                Arc::new(BM25Similarity::new()) as Arc<dyn Similarity>,
                Arc::new(ClassicSimilarity::new()),
            ]))
        }),
        ("perfield-bm25-classic", || Arc::new(PerFieldSim::new())),
    ]
}

// ---------------------------------------------------------------------------
// Query set: the output of SimilarityScoresFixture.query, line for line
// ---------------------------------------------------------------------------

/// `SimilarityScoresFixture.TOP_N`.
const TOP_N: i32 = 5;

/// `SimilarityScoresFixture.EXPLAINED`.
const EXPLAINED: usize = 3;

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

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `SimilarityScoresFixture.queries()`, in the same order.
fn queries() -> Vec<(&'static str, Arc<dyn Query>)> {
    let mut disj = BooleanQueryBuilder::new();
    disj.add(tq("body", "w3"), Occur::SHOULD).unwrap();
    disj.add(tq("body", "w11"), Occur::SHOULD).unwrap();
    disj.add(tq("title", "w3"), Occur::SHOULD).unwrap();
    let mut conj = BooleanQueryBuilder::new();
    conj.add(tq("body", "w1"), Occur::MUST).unwrap();
    conj.add(tq("body", "w2"), Occur::SHOULD).unwrap();
    vec![
        ("term body:w7", tq("body", "w7")),
        ("term title:w13", tq("title", "w13")),
        ("bool body:w3 body:w11 title:w3", Arc::new(disj.build())),
        ("bool +body:w1 body:w2", Arc::new(conj.build())),
        (
            "phrase body:\"w5 w22\"~1",
            Arc::new(PhraseQuery::with_slop(1, "body", &["w5", "w22"]).unwrap()),
        ),
    ]
}

/// Rucene's run of the query set over every index under `root`: the same
/// lines `SimilarityScoresFixture query` prints.
fn query_report(root: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let mut out = String::new();
    for (name, similarity) in similarities() {
        writeln!(out, "similarity {name}").unwrap();
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(root.join(name)).unwrap());
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
        let mut searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
        searcher.set_similarity(similarity());
        let stored = searcher.stored_fields().unwrap();
        writeln!(
            out,
            "reader maxDoc={} numDocs={}",
            reader.max_doc(),
            reader.num_docs()
        )
        .unwrap();
        for (qname, query) in queries() {
            let top = searcher.search(Arc::clone(&query), TOP_N).unwrap();
            writeln!(
                out,
                "query {qname} total={} {}",
                top.total_hits.value(),
                relation(&top.total_hits)
            )
            .unwrap();
            for (k, hit) in top.score_docs.iter().enumerate() {
                writeln!(
                    out,
                    "  hit doc={} score={} id={}",
                    hit.doc,
                    bits(hit.score),
                    stored_id(stored.as_ref(), hit.doc)
                )
                .unwrap();
                if k < EXPLAINED {
                    let explanation = searcher.explain(Arc::clone(&query), hit.doc).unwrap();
                    writeln!(
                        out,
                        "    explain value={}",
                        bits(explanation.value().float_value())
                    )
                    .unwrap();
                    for line in explanation.to_string().lines() {
                        writeln!(out, "    | {line}").unwrap();
                    }
                }
            }
        }
    }
    out
}

/// Rucene writes the corpus once per similarity under `root`.
fn write_all(root: &Path, perturbed: bool) {
    for (name, similarity) in similarities() {
        write_corpus(&root.join(name), similarity(), perturbed);
    }
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

/// Runs `SimilarityScoresFixture <mode> <root>`: `(success, stdout, stderr)`.
fn run_java(mode: &str, root: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         SimilarityScoresFixture.java",
    );
    let output = Command::new("java")
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
    let mut section = "";
    let mut context = "";
    for k in 0..l.len().max(r.len()) {
        let a = l.get(k).copied().unwrap_or("<end>");
        let b = r.get(k).copied().unwrap_or("<end>");
        if a.starts_with("similarity ") {
            section = a;
        }
        if a.starts_with("query ") {
            context = a;
        }
        if a != b {
            writeln!(
                out,
                "  line {} [{section} / {context}]\n    {left_name}: {a}\n    {right_name}: {b}",
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

/// The `reader` lines of a report: the document counts of every index.
fn reader_lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|l| l.starts_with("reader "))
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
fn similarity_scores_and_explanations_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-similarity-scores")
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

    // Both sides write the corpus once per similarity.
    write_all(&r, false);
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

    // The oracle's own report must exercise every similarity and every
    // query: an empty result would make the comparisons vacuous.
    let oracle = lucene_on_l
        .as_ref()
        .expect("the Lucene report on L is required");
    let similarity_count = oracle
        .lines()
        .filter(|l| l.starts_with("similarity "))
        .count();
    assert_eq!(
        similarity_count,
        similarities().len(),
        "the oracle must report every similarity"
    );
    let mut lines = oracle.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("query ") {
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  hit ")),
                "the oracle query has no hits: {line}"
            );
        }
    }
    assert!(
        oracle.contains("    | "),
        "no explanations in the oracle report"
    );
    writeln!(
        report,
        "== oracle report ({} lines)",
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
    // change the scores the comparison sees, with the same document counts.
    write_all(&p, true);
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
        "{} similarity failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
