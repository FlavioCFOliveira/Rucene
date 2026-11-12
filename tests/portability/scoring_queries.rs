//! Positional and compound queries, counting, paging and shard merging
//! against Apache Lucene Core 10.5.0 (rmp task #407).
//!
//! The scoring corpus of `similarity_scores.rs` is defined by the same integer
//! formulas here and in `ScoringQueriesFixture.java`, and indexed with the
//! default similarity by Rucene into `R` and by Lucene into `L`.
//!
//! * The query set (`ScoringQueriesFixture.query`, [`query_report`]) prints
//!   the hits and score bits of sloppy `PhraseQuery` (slop 1–3, repeated
//!   terms), `MultiPhraseQuery`, `NGramPhraseQuery`, `SynonymQuery` with
//!   boosts, `CombinedFieldQuery`, `BlendedTermQuery` (boolean and dismax
//!   rewrites), `DisjunctionMaxQuery` with a tie-breaker, `BooleanQuery`
//!   `minimumShouldMatch`, `BoostQuery` and `ConstantScoreQuery`; then
//!   `IndexSearcher.count`, `searchAfter` paging by score and by a field sort
//!   (with and without scores), and `TopDocs.merge` of per-leaf searchers'
//!   results, by score and by the field sort.
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
    boolean_rewrite, BlendedRewriteMethod, BlendedTermQueryBuilder, BooleanQueryBuilder,
    BoostQuery, CombinedFieldQueryBuilder, ConstantScoreQuery, DisjunctionMaxQuery,
    DisjunctionMaxRewrite, FieldDoc, IndexSearcher, MatchAllDocsQuery, MultiPhraseQueryBuilder,
    NGramPhraseQuery, Occur, PhraseQuery, Query, ScoreDoc, Sort, SortField, SortFieldType,
    SortValue, SynonymQueryBuilder, TermQuery, TopDocs, TotalHits, TotalHitsRelation,
    WildcardQuery,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// Scoring corpus: the formulas of ScoringQueriesFixture
// ---------------------------------------------------------------------------

/// `ScoringQueriesFixture.BATCHES`.
const BATCHES: [(i32, i32); 3] = [(0, 100), (100, 200), (200, 300)];

/// `ScoringQueriesFixture.WORDS`.
const WORDS: [&str; 20] = [
    "apple", "apply", "ample", "maple", "apricot", "banana", "bandana", "cabana", "cherry",
    "sherry", "berry", "merry", "date", "data", "gate", "grape", "graph", "form", "from", "zebra",
];

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

/// `ScoringQueriesFixture.body(int)`; the perturbed corpus appends `w7` to
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

/// `ScoringQueriesFixture.TEXT_TYPE`.
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

/// `ScoringQueriesFixture.doc(int)`; the perturbed corpus gives document 0 a
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

/// `ScoringQueriesFixture.write(Directory)`: the same operations, in order.
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
// Query set: the output of ScoringQueriesFixture.query, line for line
// ---------------------------------------------------------------------------

/// `ScoringQueriesFixture.TOP_N`.
const TOP_N: i32 = 10;

/// `ScoringQueriesFixture.PAGE`.
const PAGE: i32 = 7;

/// `ScoringQueriesFixture.PAGES`.
const PAGES: usize = 3;

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

fn phrase(slop: i32, terms: &[&str]) -> Arc<dyn Query> {
    Arc::new(PhraseQuery::with_slop(slop, "body", terms).unwrap())
}

/// `ScoringQueriesFixture.sortValue(FieldDoc)`.
fn sort_value(fd: &FieldDoc) -> String {
    match &fd.fields.as_ref().expect("sort values")[0] {
        SortValue::Null => "null".to_string(),
        SortValue::Bytes(b) => format!("bytes:{}", String::from_utf8_lossy(b.slice())),
        SortValue::Int(v) => format!("Integer:{v}"),
        SortValue::Long(v) => format!("Long:{v}"),
        SortValue::Float(v) => format!("Float:{v}"),
        SortValue::Double(v) => format!("Double:{v}"),
    }
}

/// `ScoringQueriesFixture.printHits`: `sort` is set for a field-sorted hit.
fn print_hits(
    out: &mut String,
    header: &str,
    total: &TotalHits,
    hits: &[(ScoreDoc, Option<String>)],
    stored: &dyn StoredFields,
) {
    writeln!(out, "{header} total={} {}", total.value(), relation(total)).unwrap();
    for (hit, sort) in hits {
        let mut line = format!("  hit doc={} score={}", hit.doc, bits(hit.score));
        if let Some(sort) = sort {
            write!(line, " sort={sort}").unwrap();
        }
        write!(line, " id={}", stored_id(stored, hit.doc)).unwrap();
        writeln!(out, "{line}").unwrap();
    }
}

fn score_hits(top: &TopDocs) -> Vec<(ScoreDoc, Option<String>)> {
    top.score_docs.iter().map(|h| (*h, None)).collect()
}

fn field_hits(docs: &[FieldDoc]) -> Vec<(ScoreDoc, Option<String>)> {
    docs.iter()
        .map(|fd| (fd.score_doc, Some(sort_value(fd))))
        .collect()
}

fn msm_query() -> Arc<dyn Query> {
    let mut msm = BooleanQueryBuilder::new();
    for w in ["w1", "w2", "w3", "w4"] {
        msm.add(tq("body", w), Occur::SHOULD).unwrap();
    }
    msm.set_minimum_number_should_match(2);
    Arc::new(msm.build())
}

fn dismax_query() -> Arc<dyn Query> {
    Arc::new(
        DisjunctionMaxQuery::new(
            vec![tq("body", "w3"), tq("title", "w3"), tq("body", "w13")],
            0.3,
        )
        .unwrap(),
    )
}

fn synonym_query() -> Arc<dyn Query> {
    let mut b = SynonymQueryBuilder::new(Some("body"));
    b.add_term(&term("body", "w3")).unwrap();
    b.add_term_with_boost(&term("body", "w13"), 0.5).unwrap();
    b.add_term_with_boost(&term("body", "w23"), 0.25).unwrap();
    Arc::new(b.build())
}

fn blended(rewrite: Arc<dyn BlendedRewriteMethod>) -> Arc<dyn Query> {
    let mut b = BlendedTermQueryBuilder::new();
    b.set_rewrite_method(rewrite);
    b.add_term(term("body", "w7")).unwrap();
    b.add_boosted(term("title", "w7"), 2.0).unwrap();
    b.add_term(term("body", "w8")).unwrap();
    Arc::new(b.build())
}

/// `ScoringQueriesFixture.queries()`, in the same order.
fn queries() -> Vec<(&'static str, Arc<dyn Query>)> {
    let mut mp = MultiPhraseQueryBuilder::new();
    mp.add_terms(vec![term("body", "w0"), term("body", "w3")])
        .unwrap();
    mp.add_terms(vec![term("body", "w17"), term("body", "w20")])
        .unwrap();
    let mut mps = MultiPhraseQueryBuilder::new();
    mps.add_terms(vec![term("body", "w17"), term("body", "w20")])
        .unwrap();
    mps.add_terms(vec![term("body", "w0"), term("body", "w3")])
        .unwrap();
    mps.set_slop(2).unwrap();
    let mut combined = CombinedFieldQueryBuilder::new("w7");
    combined.add_field_with_weight("body", 1.0).unwrap();
    combined.add_field_with_weight("title", 2.0).unwrap();
    vec![
        ("phrase body:\"w17 w0\"~2", phrase(2, &["w17", "w0"])),
        ("phrase body:\"w3 w37\"~3", phrase(3, &["w3", "w37"])),
        ("phrase body:\"w12 w6\"~1", phrase(1, &["w12", "w6"])),
        ("phrase body:\"w21 w21\"~1", phrase(1, &["w21", "w21"])),
        ("phrase body:\"w21 w21\"~2", phrase(2, &["w21", "w21"])),
        (
            "phrase body:\"w9 w26 w9\"~2",
            phrase(2, &["w9", "w26", "w9"]),
        ),
        (
            "multiphrase body:\"(w0|w3) (w17|w20)\"",
            Arc::new(mp.build()),
        ),
        (
            "multiphrase body:\"(w17|w20) (w0|w3)\"~2",
            Arc::new(mps.build()),
        ),
        (
            "ngram2 body:\"w12 w29 w6\"",
            Arc::new(NGramPhraseQuery::new(
                2,
                PhraseQuery::from_strings("body", &["w12", "w29", "w6"]).unwrap(),
            )),
        ),
        ("synonym body:{w3 w13^0.5 w23^0.25}", synonym_query()),
        (
            "combined w7 body^1 title^2",
            Arc::new(combined.build().unwrap()),
        ),
        (
            "blended boolean body:w7 title:w7^2 body:w8",
            blended(boolean_rewrite()),
        ),
        (
            "blended dismax0.1 body:w7 title:w7^2 body:w8",
            blended(Arc::new(DisjunctionMaxRewrite::new(0.1))),
        ),
        ("dismax (body:w3 | title:w3 | body:w13)~0.3", dismax_query()),
        ("bool body:(w1 w2 w3 w4) msm=2", msm_query()),
        (
            "boost body:w5^2.5",
            Arc::new(BoostQuery::new(tq("body", "w5"), 2.5).unwrap()),
        ),
        (
            "constant body:w6",
            Arc::new(ConstantScoreQuery::new(tq("body", "w6"))),
        ),
        (
            "boost constant(body:w6)^3",
            Arc::new(
                BoostQuery::new(Arc::new(ConstantScoreQuery::new(tq("body", "w6"))), 3.0).unwrap(),
            ),
        ),
    ]
}

/// `ScoringQueriesFixture.countQueries()`, in the same order.
fn count_queries() -> Vec<(&'static str, Arc<dyn Query>)> {
    vec![
        ("term body:w7", tq("body", "w7")),
        ("phrase body:\"w0 w17\"", phrase(0, &["w0", "w17"])),
        ("matchall", Arc::new(MatchAllDocsQuery::instance())),
        (
            "wildcard body:w1*",
            Arc::new(WildcardQuery::new(term("body", "w1*")).unwrap()),
        ),
        ("bool body:(w1 w2 w3 w4) msm=2", msm_query()),
        ("dismax (body:w3 | title:w3 | body:w13)~0.3", dismax_query()),
        ("synonym body:{w3 w13^0.5 w23^0.25}", synonym_query()),
    ]
}

/// `ScoringQueriesFixture.pagingQuery()`.
fn paging_query() -> Arc<dyn Query> {
    let mut b = BooleanQueryBuilder::new();
    for w in ["w1", "w2", "w3"] {
        b.add(tq("body", w), Occur::SHOULD).unwrap();
    }
    Arc::new(b.build())
}

/// `ScoringQueriesFixture.rankSort()`.
fn rank_sort() -> Sort {
    Sort::new_fields(vec![SortField::new(
        Some("rank".to_string()),
        SortFieldType::Long,
    )
    .unwrap()])
    .unwrap()
}

/// `ScoringQueriesFixture.printMerged`: each hit's shard, shard-local doc,
/// score bits, sort value when field-sorted, and id from its shard.
fn print_merged(
    out: &mut String,
    header: &str,
    total: &TotalHits,
    hits: &[(ScoreDoc, Option<String>)],
    shards: &[IndexSearcher],
) {
    writeln!(out, "{header} total={} {}", total.value(), relation(total)).unwrap();
    for (hit, sort) in hits {
        let mut line = format!(
            "  hit shard={} doc={} score={}",
            hit.shard_index,
            hit.doc,
            bits(hit.score)
        );
        if let Some(sort) = sort {
            write!(line, " sort={sort}").unwrap();
        }
        let stored = shards[hit.shard_index as usize].stored_fields().unwrap();
        write!(line, " id={}", stored_id(stored.as_ref(), hit.doc)).unwrap();
        writeln!(out, "{line}").unwrap();
    }
}

/// Rucene's run of the query set over the index at `path`: the same lines
/// `ScoringQueriesFixture query` prints.
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

    for (name, query) in queries() {
        let top = searcher.search(query, TOP_N).unwrap();
        print_hits(
            &mut out,
            &format!("query {name}"),
            &top.total_hits,
            &score_hits(&top),
            stored,
        );
    }

    for (name, query) in count_queries() {
        writeln!(out, "count {name} = {}", searcher.count(query).unwrap()).unwrap();
    }

    let paging = paging_query();
    let mut after: Option<ScoreDoc> = None;
    for page in 0..PAGES {
        let top = searcher
            .search_after(after, Arc::clone(&paging), PAGE)
            .unwrap();
        print_hits(
            &mut out,
            &format!("page score {page}"),
            &top.total_hits,
            &score_hits(&top),
            stored,
        );
        after = Some(*top.score_docs.last().expect("a full page"));
    }
    let mut field_after: Option<FieldDoc> = None;
    for page in 0..PAGES {
        let top = searcher
            .search_after_with_sort(field_after, Arc::clone(&paging), PAGE, &rank_sort())
            .unwrap();
        print_hits(
            &mut out,
            &format!("page rank {page}"),
            &top.total_hits,
            &field_hits(&top.score_docs),
            stored,
        );
        field_after = Some(top.score_docs.last().expect("a full page").clone());
    }
    field_after = None;
    for page in 0..PAGES {
        let top = searcher
            .search_after_with_sort_and_scores(
                field_after,
                Arc::clone(&paging),
                PAGE,
                &rank_sort(),
                true,
            )
            .unwrap();
        print_hits(
            &mut out,
            &format!("page rank+scores {page}"),
            &top.total_hits,
            &field_hits(&top.score_docs),
            stored,
        );
        field_after = Some(top.score_docs.last().expect("a full page").clone());
    }

    let shards: Vec<IndexSearcher> = leaves
        .iter()
        .map(|ctx| IndexSearcher::new(ctx.leaf_reader().as_index_reader()).unwrap())
        .collect();
    let mut score_shards = Vec::new();
    let mut sort_shards = Vec::new();
    for (s, shard) in shards.iter().enumerate() {
        let mut score_top = shard.search(Arc::clone(&paging), TOP_N).unwrap();
        let mut sort_top = shard
            .search_with_sort(Arc::clone(&paging), TOP_N, &rank_sort())
            .unwrap();
        // `TopDocs.merge` requires every hit's shard index to be set (or all
        // unset); it does not set them itself, and `print_merged` resolves
        // each hit through its shard.
        let s = i32::try_from(s).unwrap();
        for hit in &mut score_top.score_docs {
            hit.shard_index = s;
        }
        for hit in &mut sort_top.score_docs {
            hit.score_doc.shard_index = s;
        }
        score_shards.push(score_top);
        sort_shards.push(sort_top);
    }
    let merged = TopDocs::merge(TOP_N as usize, &score_shards).unwrap();
    print_merged(
        &mut out,
        "merge score",
        &merged.total_hits,
        &score_hits(&merged),
        &shards,
    );
    let merged = TopDocs::merge_sorted(&rank_sort(), TOP_N as usize, &sort_shards).unwrap();
    print_merged(
        &mut out,
        "merge rank",
        &merged.total_hits,
        &field_hits(&merged.score_docs),
        &shards,
    );
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

/// Runs `ScoringQueriesFixture <mode> <dir>`: `(success, stdout, stderr)`.
fn run_java(mode: &str, dir: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         ScoringQueriesFixture.java",
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
        if !a.starts_with("  ") {
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
fn scoring_queries_paging_and_merge_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-scoring-queries")
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

    // The oracle's own report must exercise every section: an empty result
    // would make the comparisons vacuous.
    let oracle = lucene_on_l
        .as_ref()
        .expect("the Lucene report on L is required");
    let mut lines = oracle.lines().peekable();
    let mut sections = 0;
    while let Some(line) = lines.next() {
        if line.starts_with("query ") || line.starts_with("page ") || line.starts_with("merge ") {
            sections += 1;
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  hit ")),
                "the oracle section has no hits: {line}"
            );
        } else if let Some(count) = line.strip_prefix("count ") {
            assert!(!count.ends_with(" = 0"), "the oracle count is zero: {line}");
        }
    }
    assert_eq!(
        sections,
        queries().len() + 3 * PAGES + 2,
        "the oracle must report every query, page and merge"
    );
    assert!(
        oracle
            .lines()
            .any(|l| l.starts_with("  hit shard=") && !l.starts_with("  hit shard=0 ")),
        "the merged results must draw on more than one shard"
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
        "{} scoring query failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
