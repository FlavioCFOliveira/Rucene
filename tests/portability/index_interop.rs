//! Bidirectional index acceptance between Rucene and Apache Lucene Core
//! 10.5.0 (rmp task #270).
//!
//! The same fixed corpus and operations are written by both sides into two
//! on-disk indexes: `R` by Rucene with its default codec, `L` by Lucene
//! (`IndexInteropFixture write`). The corpus is defined by the same integer
//! formulas here and in `IndexInteropFixture.java`; every field kind is
//! present: text with positions and offsets (stored, with norms), a token
//! stream field with positions, offsets and payloads in its postings and term
//! vectors, stored numbers, bytes and strings, the five doc-values types,
//! int/long/float/double and two-dimensional points, float and byte kNN
//! vectors. The operations delete by term and by query, update numeric and
//! binary doc values, commit five times, force-merge and force-merge deletes,
//! so the final commit holds several segments with deletions and doc-values
//! generations.
//!
//! * Direction 1: Lucene opens `R`, runs `CheckIndex` at `Level.MAX_VALUE`
//!   (checksums, integrity and slow checks) and runs the query set.
//! * Direction 2: Rucene opens `L`, runs its own `CheckIndex` at
//!   `Level::MAX_VALUE` and runs the query set.
//! * The query set (`IndexInteropFixture.query`, [`query_report`]) runs on
//!   both indexes from both sides. The four reports are compared line by
//!   line: the same reader over the same index must agree exactly (doc ids,
//!   score bits, values), and each side's report on its own index must agree
//!   with the other side's on its own index.
//! * The file listing of the latest commit of both indexes is compared and
//!   reported; it does not fail the test, as file bytes carry segment ids.
//! * Positive control: one flipped byte in a copy of `R` makes Lucene's
//!   `CheckIndex` report the index as not clean.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PackedTokenAttributeImpl, PayloadAttribute,
    PayloadAttributeImpl, PositionIncrementAttribute,
};
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{
    BinaryDocValuesField, Document, DoublePoint, Field, FieldType, FloatPoint, IntPoint,
    KnnByteVectorField, KnnFloatVectorField, LongPoint, NumericDocValuesField, NumericValue,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    StringField,
};
use rucene::index::check_index::{CheckIndex, Level};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::TieredMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{
    sub_index_from_leaves, IndexOptions, IndexReader, LeafReaderContext, StoredFields, Term,
    VectorSimilarityFunction, POSTINGS_ENUM_ALL,
};
use rucene::search::{
    BooleanQueryBuilder, FieldDoc, IndexSearcher, KnnByteVectorQuery, KnnFloatVectorQuery,
    MatchAllDocsQuery, Occur, PhraseQuery, Query, ScoreDoc, Sort, SortField, SortFieldType,
    SortValue, TermQuery, TopFieldCollectorManager, TotalHits, TotalHitsRelation, NO_MORE_DOCS,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::{AttributeSource, BytesRef, InfoStream};

// ---------------------------------------------------------------------------
// Corpus: the formulas of IndexInteropFixture
// ---------------------------------------------------------------------------

/// `IndexInteropFixture.BATCHES`.
const BATCHES: [(i32, i32); 4] = [(0, 120), (120, 240), (240, 320), (320, 360)];

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

fn body(i: i32) -> String {
    (0..i % 7 + 1)
        .map(|j| format!("w{}", (i * 31 + j * 17) % 50))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug)]
struct Tok {
    term: String,
    start: i32,
    end: i32,
    payload: Option<Vec<u8>>,
}

fn tv_tokens(i: i32) -> Vec<Tok> {
    (0..i % 4 + 1)
        .map(|k| Tok {
            term: format!("t{}", (i + k * 3) % 9),
            start: k * 4,
            end: k * 4 + 2,
            payload: if (i + k) % 3 == 0 {
                None
            } else {
                Some(vec![(i + k) as u8, k as u8])
            },
        })
        .collect()
}

fn float_vector(i: i32) -> [f32; 4] {
    [
        ((i * 3) % 17 - 8) as f32 / 2.0,
        ((i * 5) % 13 - 6) as f32 / 4.0,
        (i % 7) as f32 / 2.0,
        ((i * 11) % 19) as f32 / 8.0,
    ]
}

/// Java `byte`s are signed; the port's byte vectors carry the same bits.
fn byte_vector(i: i32) -> Vec<u8> {
    [(i * 3) % 50, (i * 7) % 60 - 30, i % 20, (i * 13) % 40 - 20]
        .iter()
        .map(|&v| v as i8 as u8)
        .collect()
}

/// `IndexInteropFixture.BODY_TYPE`.
fn body_type() -> FieldType {
    let mut t = FieldType::new();
    t.set_tokenized(true).unwrap();
    t.set_stored(true).unwrap();
    t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    t.freeze();
    t
}

/// `IndexInteropFixture.TV_TYPE`.
fn tv_type() -> FieldType {
    let mut t = FieldType::new();
    t.set_tokenized(true).unwrap();
    t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    t.set_store_term_vectors(true).unwrap();
    t.set_store_term_vector_positions(true).unwrap();
    t.set_store_term_vector_offsets(true).unwrap();
    t.set_store_term_vector_payloads(true).unwrap();
    t.freeze();
    t
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

/// `IndexInteropFixture.doc(int)`.
fn doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    d.add(Box::new(Field::new("body", body(i), body_type()).unwrap()));
    let stream: Rc<RefCell<dyn TokenStream>> =
        Rc::new(RefCell::new(ScriptedTokenStream::new(tv_tokens(i))));
    d.add(Box::new(
        Field::new_with_token_stream("tv", stream, tv_type()).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number("s_int", NumericValue::Int(i * 3 - 100)).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number("s_long", NumericValue::Long(i as i64 * 1_000_000_007)).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number("s_float", NumericValue::Float(i as f32 / 4.0)).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number("s_double", NumericValue::Double(i as f64 / 8.0)).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_bytes("s_bytes", BytesRef::new(vec![i as u8, (i >> 8) as u8, 7])).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_string("s_str", format!("stored-{i}")).unwrap(),
    ));
    if i % 11 != 0 {
        d.add(Box::new(NumericDocValuesField::new(
            "dv_num",
            ((i * 37) % 101 - 50) as i64,
        )));
    }
    if i % 5 != 0 {
        d.add(Box::new(BinaryDocValuesField::new(
            "dv_bin",
            bytes(&format!("bin-{}", i * 7 % 13)),
        )));
    }
    if i % 6 != 0 {
        d.add(Box::new(SortedDocValuesField::new(
            "dv_sorted",
            bytes(&format!("s{}", i * 13 % 29)),
        )));
    }
    d.add(Box::new(SortedNumericDocValuesField::new(
        "dv_snum",
        (i % 10) as i64,
    )));
    d.add(Box::new(SortedNumericDocValuesField::new(
        "dv_snum",
        ((i * 3) % 17) as i64,
    )));
    d.add(Box::new(SortedSetDocValuesField::new(
        "dv_sset",
        bytes(&format!("a{}", i % 5)),
    )));
    d.add(Box::new(SortedSetDocValuesField::new(
        "dv_sset",
        bytes(&format!("b{}", i % 3)),
    )));
    d.add(Box::new(IntPoint::new("p_int", &[(i * 7) % 200]).unwrap()));
    d.add(Box::new(
        LongPoint::new("p_long", &[i as i64 * 100_000 - 5_000_000]).unwrap(),
    ));
    d.add(Box::new(
        FloatPoint::new("p_float", &[i as f32 * 0.5 - 40.0]).unwrap(),
    ));
    d.add(Box::new(
        DoublePoint::new("p_double", &[i as f64 * 0.25 - 10.0]).unwrap(),
    ));
    d.add(Box::new(IntPoint::new("p_2d", &[i % 20, i / 20]).unwrap()));
    if i % 9 != 0 {
        d.add(Box::new(
            KnnFloatVectorField::new("vf", &float_vector(i), VectorSimilarityFunction::EUCLIDEAN)
                .unwrap(),
        ));
    }
    d.add(Box::new(
        KnnByteVectorField::new("vb", &byte_vector(i), VectorSimilarityFunction::EUCLIDEAN)
            .unwrap(),
    ));
    d
}

fn add_batch(w: &IndexWriter, batch: usize) {
    let (from, to) = BATCHES[batch];
    for i in from..to {
        w.add_document(&doc(i)).unwrap();
    }
}

/// `IndexInteropFixture.write(Directory)`: the same operations, in order.
fn write_corpus(path: &Path) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let mut config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let w = IndexWriter::new(dir, config).unwrap();
    add_batch(&w, 0);
    w.commit().unwrap();
    w.delete_documents_by_terms(vec![term("id", &id(3))])
        .unwrap();
    w.delete_documents_by_terms(vec![term("id", &id(17))])
        .unwrap();
    w.delete_documents_by_queries(vec![
        IntPoint::new_range_query("p_int", &[50], &[60]).unwrap()
    ])
    .unwrap();
    add_batch(&w, 1);
    w.update_numeric_doc_value(term("id", &id(5)), "dv_num", 777)
        .unwrap();
    w.update_binary_doc_value(term("id", &id(130)), "dv_bin", bytes("updated-130"))
        .unwrap();
    w.commit().unwrap();
    add_batch(&w, 2);
    w.commit().unwrap();
    w.force_merge(2).unwrap();
    add_batch(&w, 3);
    w.delete_documents_by_terms(vec![term("id", &id(250))])
        .unwrap();
    w.delete_documents_by_queries(vec![
        IntPoint::new_range_query("p_int", &[190], &[192]).unwrap()
    ])
    .unwrap();
    w.update_numeric_doc_value(term("id", &id(300)), "dv_num", -999)
        .unwrap();
    w.update_binary_doc_value(term("id", &id(10)), "dv_bin", bytes("updated-10"))
        .unwrap();
    w.commit().unwrap();
    // Documents 60-99 (p_2d second dimension 3 or 4): more than
    // forceMergeDeletesPctAllowed (10%) of the merged segment holding them,
    // and none of the documents the query set reports.
    w.delete_documents_by_queries(vec![
        IntPoint::new_range_query("p_2d", &[0, 3], &[19, 4]).unwrap()
    ])
    .unwrap();
    w.force_merge_deletes().unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Query set: the output of IndexInteropFixture.query, line for line
// ---------------------------------------------------------------------------

fn hex(slice: &[u8]) -> String {
    slice.iter().map(|b| format!("{b:02x}")).collect()
}

fn utf8(b: &BytesRef) -> String {
    String::from_utf8_lossy(b.slice()).into_owned()
}

fn stored_id(stored: &dyn StoredFields, doc: i32) -> String {
    let document = stored.document(doc).unwrap();
    document
        .get_field("id")
        .and_then(|f| f.string_value())
        .unwrap_or_else(|| "null".to_string())
}

fn relation(total: &TotalHits) -> &'static str {
    match total.relation() {
        TotalHitsRelation::EQUAL_TO => "EQUAL_TO",
        TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO => "GREATER_THAN_OR_EQUAL_TO",
    }
}

fn print_top(
    out: &mut String,
    name: &str,
    total: &TotalHits,
    hits: &[(ScoreDoc, Option<String>)],
    stored: &dyn StoredFields,
) {
    writeln!(
        out,
        "query {name} total={} {}",
        total.value(),
        relation(total)
    )
    .unwrap();
    for (hit, sort) in hits {
        let mut line = format!("  hit doc={} score={:08x}", hit.doc, hit.score.to_bits());
        if let Some(sort) = sort {
            write!(line, " sort={sort}").unwrap();
        }
        write!(line, " id={}", stored_id(stored, hit.doc)).unwrap();
        writeln!(out, "{line}").unwrap();
    }
}

fn search(
    out: &mut String,
    searcher: &IndexSearcher,
    stored: &dyn StoredFields,
    name: &str,
    query: Arc<dyn Query>,
    n: i32,
) -> Vec<ScoreDoc> {
    let top = searcher.search(query, n).unwrap();
    let hits: Vec<_> = top.score_docs.iter().map(|h| (*h, None)).collect();
    print_top(out, name, &top.total_hits, &hits, stored);
    top.score_docs
}

fn sort_value(fd: &FieldDoc) -> String {
    match &fd.fields.as_ref().expect("sort values")[0] {
        SortValue::Null => "null".to_string(),
        SortValue::Bytes(b) => format!("bytes:{}", utf8(b)),
        SortValue::Int(v) => format!("Integer:{v}"),
        SortValue::Long(v) => format!("Long:{v}"),
        SortValue::Float(v) => format!("Float:{v}"),
        SortValue::Double(v) => format!("Double:{v}"),
    }
}

/// `IndexSearcher.search(Query, int, Sort)`: a `TopFieldCollectorManager`
/// with `TOTAL_HITS_THRESHOLD` (1000) and no scores.
fn search_sorted(
    out: &mut String,
    searcher: &IndexSearcher,
    stored: &dyn StoredFields,
    name: &str,
    sort: Sort,
    n: i32,
) {
    let manager = TopFieldCollectorManager::new(sort, n, None, 1000).unwrap();
    let top = searcher
        .search_with_manager(Arc::new(MatchAllDocsQuery::instance()), &manager)
        .unwrap();
    let hits: Vec<_> = top
        .score_docs
        .iter()
        .map(|fd| (fd.score_doc, Some(sort_value(fd))))
        .collect();
    print_top(out, name, &top.total_hits, &hits, stored);
}

fn print_stored(out: &mut String, stored: &dyn StoredFields, doc: i32) {
    let document = stored.document(doc).unwrap();
    for f in document.get_fields() {
        let value = if let Some(n) = f.numeric_value() {
            match n {
                NumericValue::Int(v) => format!("Integer:{v}"),
                NumericValue::Long(v) => format!("Long:{v}"),
                NumericValue::Float(v) => format!("Float:{:08x}", v.to_bits()),
                NumericValue::Double(v) => format!("Double:{:016x}", v.to_bits()),
            }
        } else if let Some(b) = f.binary_value() {
            format!("bytes:{}", hex(b.slice()))
        } else {
            format!("string:{}", f.string_value().unwrap_or_default())
        };
        writeln!(out, "  stored {} {value}", f.name()).unwrap();
    }
}

fn print_doc_values(out: &mut String, leaves: &[Arc<LeafReaderContext>], doc: i32) {
    let ctx = &leaves[sub_index_from_leaves(doc, leaves)];
    let leaf = ctx.leaf_reader();
    let local = doc - ctx.doc_base();

    let mut num = "none".to_string();
    if let Some(mut v) = leaf.get_numeric_doc_values("dv_num").unwrap() {
        if v.advance_exact(local).unwrap() {
            num = v.long_value().unwrap().to_string();
        }
    }
    writeln!(out, "  dv dv_num {num}").unwrap();
    let mut bin = "none".to_string();
    if let Some(mut v) = leaf.get_binary_doc_values("dv_bin").unwrap() {
        if v.advance_exact(local).unwrap() {
            bin = utf8(&v.binary_value().unwrap());
        }
    }
    writeln!(out, "  dv dv_bin {bin}").unwrap();
    let mut sorted = "none".to_string();
    if let Some(mut v) = leaf.get_sorted_doc_values("dv_sorted").unwrap() {
        if v.advance_exact(local).unwrap() {
            let ord = v.ord_value().unwrap();
            sorted = format!("{ord}:{}", utf8(&v.lookup_ord(ord).unwrap()));
        }
    }
    writeln!(out, "  dv dv_sorted {sorted}").unwrap();
    let mut line = "  dv dv_snum".to_string();
    let mut present = false;
    if let Some(mut v) = leaf.get_sorted_numeric_doc_values("dv_snum").unwrap() {
        if v.advance_exact(local).unwrap() {
            present = true;
            for _ in 0..v.doc_value_count().unwrap() {
                write!(line, " {}", v.next_value().unwrap()).unwrap();
            }
        }
    }
    if !present {
        line.push_str(" none");
    }
    writeln!(out, "{line}").unwrap();
    let mut line = "  dv dv_sset".to_string();
    let mut present = false;
    if let Some(mut v) = leaf.get_sorted_set_doc_values("dv_sset").unwrap() {
        if v.advance_exact(local).unwrap() {
            present = true;
            for _ in 0..v.doc_value_count().unwrap() {
                let ord = v.next_ord().unwrap();
                write!(line, " {ord}:{}", utf8(&v.lookup_ord(ord).unwrap())).unwrap();
            }
        }
    }
    if !present {
        line.push_str(" none");
    }
    writeln!(out, "{line}").unwrap();
}

fn print_term_vectors(out: &mut String, reader: &Arc<dyn IndexReader>, doc: i32) {
    let Some(fields) = reader.term_vectors().unwrap().get(doc).unwrap() else {
        writeln!(out, "  tv none").unwrap();
        return;
    };
    let names: Vec<String> = fields.iterator().collect();
    for field in names {
        let terms = fields
            .terms(&field)
            .unwrap()
            .expect("terms of a listed field");
        writeln!(
            out,
            "  tv field {field} size={} positions={} offsets={} payloads={}",
            terms.size().unwrap(),
            terms.has_positions(),
            terms.has_offsets(),
            terms.has_payloads()
        )
        .unwrap();
        let mut te = terms.iterator().unwrap();
        while let Some(text) = te.next().unwrap() {
            let mut line = format!(
                "   term {} freq={}",
                utf8(&text),
                te.total_term_freq().unwrap()
            );
            let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap();
            pe.next_doc().unwrap();
            for _ in 0..pe.freq().unwrap() {
                let pos = pe.next_position().unwrap();
                let payload = match pe.get_payload().unwrap() {
                    Some(p) => hex(p),
                    None => "-".to_string(),
                };
                write!(
                    line,
                    " [{pos},{},{},{payload}]",
                    pe.start_offset(),
                    pe.end_offset()
                )
                .unwrap();
            }
            writeln!(out, "{line}").unwrap();
        }
    }
}

/// `IndexInteropFixture.POSTINGS_TERMS`.
const POSTINGS_TERMS: [(&str, &str); 4] =
    [("body", "w7"), ("body", "w22"), ("tv", "t1"), ("tv", "t4")];

/// `IndexInteropFixture.printPostings`: every leaf's postings of
/// `field:text` with `PostingsEnum.ALL`, deleted documents included.
fn print_postings(out: &mut String, leaves: &[Arc<LeafReaderContext>], field: &str, text: &str) {
    writeln!(out, "postings {field}:{text}").unwrap();
    for ctx in leaves {
        let Some(terms) = ctx.leaf_reader().terms(field).unwrap() else {
            continue;
        };
        let mut te = terms.iterator().unwrap();
        if !te.seek_exact(&bytes(text)).unwrap() {
            continue;
        }
        let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap();
        let mut d = pe.next_doc().unwrap();
        while d != NO_MORE_DOCS {
            let freq = pe.freq().unwrap();
            let mut line = format!("  doc={} freq={freq}", ctx.doc_base() + d);
            for _ in 0..freq {
                let pos = pe.next_position().unwrap();
                let payload = match pe.get_payload().unwrap() {
                    Some(p) => hex(p),
                    None => "-".to_string(),
                };
                write!(
                    line,
                    " [{pos},{},{},{payload}]",
                    pe.start_offset(),
                    pe.end_offset()
                )
                .unwrap();
            }
            writeln!(out, "{line}").unwrap();
            d = pe.next_doc().unwrap();
        }
    }
}

/// Rucene's run of the query set over the index at `path`: the same lines
/// `IndexInteropFixture query` prints.
fn query_report(path: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    let stored = stored.as_ref();
    let leaves = Arc::clone(&reader).leaves();
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

    let tq = |f: &str, t: &str| -> Arc<dyn Query> { Arc::new(TermQuery::new(term(f, t))) };

    let term_top = search(
        &mut out,
        &searcher,
        stored,
        "term body:w7",
        tq("body", "w7"),
        10,
    );

    let mut bool_q = BooleanQueryBuilder::new();
    bool_q.add(tq("body", "w3"), Occur::MUST).unwrap();
    bool_q.add(tq("body", "w10"), Occur::SHOULD).unwrap();
    bool_q.add(tq("body", "w20"), Occur::MUST_NOT).unwrap();
    search(
        &mut out,
        &searcher,
        stored,
        "bool +w3 w10 -w20",
        Arc::new(bool_q.build()),
        10,
    );
    let mut disj = BooleanQueryBuilder::new();
    disj.add(tq("body", "w1"), Occur::SHOULD).unwrap();
    disj.add(tq("body", "w2"), Occur::SHOULD).unwrap();
    disj.add(tq("body", "w4"), Occur::SHOULD).unwrap();
    search(
        &mut out,
        &searcher,
        stored,
        "bool w1 w2 w4",
        Arc::new(disj.build()),
        10,
    );

    let phrase =
        PhraseQuery::new(0, vec![term("body", "w5"), term("body", "w22")], vec![0, 1]).unwrap();
    search(
        &mut out,
        &searcher,
        stored,
        "phrase body:\"w5 w22\"",
        Arc::new(phrase),
        10,
    );

    search(
        &mut out,
        &searcher,
        stored,
        "point p_int [10,60]",
        IntPoint::new_range_query("p_int", &[10], &[60]).unwrap(),
        20,
    );
    search(
        &mut out,
        &searcher,
        stored,
        "point p_long [0,8000000]",
        LongPoint::new_range_query("p_long", &[0], &[8_000_000]).unwrap(),
        20,
    );
    search(
        &mut out,
        &searcher,
        stored,
        "point p_float [-10,25.5]",
        FloatPoint::new_range_query("p_float", &[-10.0], &[25.5]).unwrap(),
        20,
    );
    search(
        &mut out,
        &searcher,
        stored,
        "point p_double [0,40]",
        DoublePoint::new_range_query("p_double", &[0.0], &[40.0]).unwrap(),
        20,
    );
    search(
        &mut out,
        &searcher,
        stored,
        "point p_2d [3,2]-[10,10]",
        IntPoint::new_range_query("p_2d", &[3, 2], &[10, 10]).unwrap(),
        20,
    );

    search_sorted(
        &mut out,
        &searcher,
        stored,
        "sort dv_num LONG",
        Sort::new_fields(vec![SortField::new(
            Some("dv_num".to_string()),
            SortFieldType::Long,
        )
        .unwrap()])
        .unwrap(),
        20,
    );
    search_sorted(
        &mut out,
        &searcher,
        stored,
        "sort dv_sorted STRING",
        Sort::new_fields(vec![SortField::new(
            Some("dv_sorted".to_string()),
            SortFieldType::String,
        )
        .unwrap()])
        .unwrap(),
        20,
    );

    search(
        &mut out,
        &searcher,
        stored,
        "knn vf",
        Arc::new(KnnFloatVectorQuery::new("vf", vec![0.5, -0.25, 1.0, 1.125], 10).unwrap()),
        10,
    );
    search(
        &mut out,
        &searcher,
        stored,
        "knn vb",
        Arc::new(
            KnnByteVectorQuery::new("vb", [10i8, -5, 7, 3].map(|v| v as u8).to_vec(), 10).unwrap(),
        ),
        10,
    );

    for (field, text) in POSTINGS_TERMS {
        print_postings(&mut out, &leaves, field, text);
    }

    for hit in term_top.iter().take(3) {
        writeln!(out, "hitdoc {}", hit.doc).unwrap();
        print_stored(&mut out, stored, hit.doc);
        print_doc_values(&mut out, &leaves, hit.doc);
    }

    for target in [id(42), id(331), id(10), id(130), id(5), id(300)] {
        let found = searcher.search(tq("id", &target), 1).unwrap();
        let Some(first) = found.score_docs.first() else {
            writeln!(out, "doc {target} missing").unwrap();
            continue;
        };
        let doc = first.doc;
        writeln!(out, "doc {target} = {doc}").unwrap();
        print_stored(&mut out, stored, doc);
        print_doc_values(&mut out, &leaves, doc);
        print_term_vectors(&mut out, &reader, doc);
    }
    out
}

// ---------------------------------------------------------------------------
// Scripted token stream: IndexInteropFixture.ScriptedTokenStream
// ---------------------------------------------------------------------------

struct ScriptedTokenStream {
    tokens: Vec<Tok>,
    upto: usize,
    final_offset: i32,
    attributes: AttributeSource,
}

impl ScriptedTokenStream {
    fn new(tokens: Vec<Tok>) -> Self {
        let mut attributes = AttributeSource::new_with_factory(default_token_attribute_factory());
        attributes
            .add_attribute::<PackedTokenAttributeImpl>()
            .expect("packed token attribute");
        // As in `portability/term_vectors.rs`: the default factory does not
        // build a payload attribute, so the stream installs the instance.
        attributes.add_attribute_impl_instance(Box::new(PayloadAttributeImpl::new()));
        Self {
            tokens,
            upto: 0,
            final_offset: 0,
            attributes,
        }
    }
}

impl std::fmt::Debug for ScriptedTokenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedTokenStream")
            .field("tokens", &self.tokens.len())
            .finish()
    }
}

impl TokenStream for ScriptedTokenStream {
    fn increment_token(&mut self) -> rucene::error::Result<bool> {
        if self.upto == self.tokens.len() {
            return Ok(false);
        }
        self.attributes.clear_attributes();
        let token = self.tokens[self.upto].clone();
        self.upto += 1;
        {
            let mut packed = self
                .attributes
                .get_attribute_mut::<PackedTokenAttributeImpl>()
                .expect("packed attribute");
            packed.append_string(&token.term);
            packed.set_position_increment(1);
            packed.set_offset(token.start, token.end);
        }
        {
            let mut payload = self
                .attributes
                .get_attribute_mut::<PayloadAttributeImpl>()
                .expect("payload attribute");
            payload.set_payload(token.payload.clone().map(BytesRef::new));
        }
        self.final_offset = token.end;
        Ok(true)
    }

    fn reset(&mut self) -> rucene::error::Result<()> {
        self.upto = 0;
        self.final_offset = 0;
        Ok(())
    }

    fn end(&mut self) -> rucene::error::Result<()> {
        self.attributes.end_attributes();
        let mut packed = self
            .attributes
            .get_attribute_mut::<PackedTokenAttributeImpl>()
            .expect("packed attribute");
        packed.set_offset(self.final_offset, self.final_offset);
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.attributes
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }
}

// ---------------------------------------------------------------------------
// Rucene CheckIndex
// ---------------------------------------------------------------------------

/// Collects the messages `CheckIndex` logs, for the failure report.
#[derive(Default)]
struct CollectingInfoStream(Mutex<Vec<String>>);

impl InfoStream for CollectingInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.PrintStreamInfoStream"
    }

    fn message(&self, component: &str, message: &str) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{component}: {message}"));
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

/// What a `CheckIndex` run reported, from either side.
#[derive(Debug, Default)]
struct CheckOutcome {
    /// The run completed and printed its status.
    completed: bool,
    clean: bool,
    num_bad_segments: i32,
    /// Each checked segment's name and error, `None` when it passed.
    segments: Vec<(String, Option<String>)>,
    report: String,
}

impl CheckOutcome {
    /// The error the check reported for `segment`, if any.
    fn segment_error(&self, segment: &str) -> Option<&str> {
        self.segments
            .iter()
            .find(|(name, _)| name == segment)
            .and_then(|(_, error)| error.as_deref())
    }
}

/// Rucene's `CheckIndex` at `Level::MAX_VALUE`.
fn rucene_check(path: &Path) -> CheckOutcome {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let log = Arc::new(CollectingInfoStream::default());
    let mut checker = match CheckIndex::new(dir) {
        Ok(checker) => checker,
        Err(e) => {
            return CheckOutcome {
                report: format!("CheckIndex::new failed: {e:?}"),
                ..CheckOutcome::default()
            }
        }
    };
    checker.set_level(Level::MAX_VALUE).unwrap();
    checker.set_info_stream_verbose(Some(log.clone() as Arc<dyn InfoStream>), true);
    let result = checker.check_index();
    let _ = checker.close();
    let messages = log.0.lock().unwrap().join("\n");
    match result {
        Ok(status) => {
            let mut report = format!(
                "clean={} numSegments={} numBadSegments={} totLoseDocCount={}\n",
                status.clean,
                status.num_segments,
                status.num_bad_segments,
                status.tot_lose_doc_count
            );
            let mut segments = Vec::new();
            for seg in &status.segment_infos {
                writeln!(
                    report,
                    "segment {} maxDoc={} error={:?}",
                    seg.name, seg.max_doc, seg.error
                )
                .unwrap();
                segments.push((
                    seg.name.clone(),
                    seg.error.as_ref().map(|e| format!("{e:?}")),
                ));
            }
            report.push_str(&messages);
            CheckOutcome {
                completed: true,
                clean: status.clean,
                num_bad_segments: status.num_bad_segments as i32,
                segments,
                report,
            }
        }
        Err(e) => CheckOutcome {
            report: format!("check_index failed: {e:?}\n{messages}"),
            ..CheckOutcome::default()
        },
    }
}

/// Parses the output of `IndexInteropFixture check`.
fn java_check(path: &Path) -> CheckOutcome {
    let (ok, out, err) = run_java("check", path);
    let mut outcome = CheckOutcome {
        report: format!("{out}\n{}", java_errors(&err)),
        ..CheckOutcome::default()
    };
    let mut saw_clean = false;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("clean=") {
            saw_clean = true;
            outcome.clean = v == "true";
        } else if let Some(v) = line.strip_prefix("numBadSegments=") {
            outcome.num_bad_segments = v.parse().unwrap_or(-1);
        } else if let Some(rest) = line.strip_prefix("segment ") {
            let name = rest.split(' ').next().unwrap_or("").to_string();
            let error = rest
                .split_once(" error=")
                .map(|(_, e)| e)
                .filter(|e| *e != "none")
                .map(str::to_string);
            outcome.segments.push((name, error));
        }
    }
    outcome.completed = ok && saw_clean;
    outcome
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

/// Runs `IndexInteropFixture <mode> <dir>`: `(success, stdout, stderr)`.
fn run_java(mode: &str, dir: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         IndexInteropFixture.java",
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
        if a.starts_with("query ")
            || a.starts_with("doc ")
            || a.starts_with("hitdoc ")
            || a.starts_with("postings ")
        {
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

/// One `segment` header with its `file name size` lines, from
/// `IndexInteropFixture files`.
fn file_listing_report(r_path: &Path, r_files: &str, l_path: &Path, l_files: &str) -> String {
    let mut out = String::new();
    if let Some(d) = diff("files(R)", r_files, "files(L)", l_files) {
        out.push_str(&d);
    } else {
        out.push_str("file listings (names, sizes, segment metadata) are identical\n");
    }
    let names = |listing: &str| -> Vec<String> {
        listing
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                l.strip_prefix("file ")
                    .or_else(|| l.strip_prefix("commit "))
                    .and_then(|rest| rest.split(' ').next())
                    .map(str::to_string)
            })
            .collect()
    };
    let (mut same, mut different) = (0, Vec::new());
    for name in names(r_files) {
        if !names(l_files).contains(&name) {
            continue;
        }
        let a = std::fs::read(r_path.join(&name)).unwrap();
        let b = std::fs::read(l_path.join(&name)).unwrap();
        if a == b {
            same += 1;
        } else {
            different.push(name);
        }
    }
    writeln!(
        out,
        "byte-identical common files: {same}; differing: {} {:?}",
        different.len(),
        different
    )
    .unwrap();
    out
}

fn fresh_dir(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Copies `from` into a fresh `to` and flips one byte in the middle of the
/// largest file of the latest commit other than `segments_N`. Returns a
/// description of the flip and the name of the segment owning the file.
fn corrupt_copy(from: &Path, to: &Path) -> (String, String) {
    let mut largest: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        std::fs::copy(entry.path(), to.join(&name)).unwrap();
        let len = entry.metadata().unwrap().len();
        if !name.starts_with("segments")
            && name != "write.lock"
            && largest.as_ref().is_none_or(|(l, _)| len > *l)
        {
            largest = Some((len, to.join(&name)));
        }
    }
    let (len, path) = largest.expect("an index file");
    let mut data = std::fs::read(&path).unwrap();
    let at = (len / 2) as usize;
    data[at] ^= 0x5a;
    std::fs::write(&path, data).unwrap();
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    let segment = format!(
        "_{}",
        file[1..]
            .split(['.', '_'])
            .next()
            .expect("a segment file name")
    );
    (format!("{} byte {at} of {len}", path.display()), segment)
}

/// `IndexInteropFixture.writeForcedDeletes(Directory)`: ten segments of 20
/// documents, 3 deleted in each even one, then `forceMergeDeletes()` under a
/// `TieredMergePolicy` with `targetSearchConcurrency` 2.
fn write_forced_deletes(path: &Path) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let mut policy = TieredMergePolicy::new();
    policy.set_floor_segment_mb(0.001).unwrap();
    policy.set_segments_per_tier(50.0).unwrap();
    policy.set_target_search_concurrency(2).unwrap();
    let mut config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config.set_merge_policy(Arc::new(policy)).unwrap();
    let w = IndexWriter::new(dir, config).unwrap();
    for s in 0..10 {
        for i in s * 20..s * 20 + 20 {
            w.add_document(&doc(i)).unwrap();
        }
        w.commit().unwrap();
    }
    for s in (0..10).step_by(2) {
        for k in 0..3 {
            w.delete_documents_by_terms(vec![term("id", &id(s * 20 + k * 7))])
                .unwrap();
        }
    }
    w.commit().unwrap();
    w.force_merge_deletes().unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

/// The `segment` lines of an `IndexInteropFixture files` listing: names,
/// document and deletion counts, generations, compound state and codec.
fn segment_lines(listing: &str) -> String {
    listing
        .lines()
        .filter(|l| l.starts_with("segment "))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn rucene_and_lucene_accept_each_others_indexes() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("index_interop");
    let r = fresh_dir(&root, "rucene");
    let l = fresh_dir(&root, "lucene");
    let corrupt = fresh_dir(&root, "rucene-corrupt");
    let corrupt_l = fresh_dir(&root, "lucene-corrupt");
    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    // Both sides write the corpus.
    write_corpus(&r);
    let (ok, _, err) = run_java("write", &l);
    assert!(ok, "Lucene failed to write the corpus:\n{err}");

    // Direction 1: Lucene's CheckIndex on the Rucene index.
    let check = java_check(&r);
    writeln!(
        report,
        "== Lucene CheckIndex on R\n{}",
        check
            .report
            .lines()
            .take_while(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    )
    .unwrap();
    if !check.completed || !check.clean {
        failures.push(format!(
            "Lucene CheckIndex rejects the Rucene index:\n{}",
            check.report
        ));
    }

    // Direction 2: Rucene's CheckIndex on the Lucene index.
    let check = rucene_check(&l);
    writeln!(
        report,
        "== Rucene CheckIndex on L\n{}",
        check.report.lines().next().unwrap_or("")
    )
    .unwrap();
    if !check.completed || !check.clean {
        failures.push(format!(
            "Rucene CheckIndex rejects the Lucene index:\n{}",
            check.report
        ));
    }

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
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| query_report(dir))).map_err(
            |panic| {
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".to_string())
            },
        )
    };
    let rucene_on_r = rust_query(&r);
    let rucene_on_l = rust_query(&l);
    let rucene_on_r = rucene_on_r
        .map_err(|e| failures.push(format!("Rucene query set on R panicked: {e}")))
        .ok();
    let rucene_on_l = rucene_on_l
        .map_err(|e| failures.push(format!("Rucene query set on L panicked: {e}")))
        .ok();

    // The oracle's own report must exercise every query: an empty result
    // would make the comparisons vacuous.
    if let Some(oracle) = &lucene_on_l {
        let mut query_lines = oracle.lines().peekable();
        while let Some(line) = query_lines.next() {
            if line.starts_with("query ") {
                let has_hit = query_lines
                    .peek()
                    .is_some_and(|next| next.starts_with("  hit "));
                assert!(has_hit, "the oracle query has no hits: {line}");
            }
        }
        assert!(
            oracle.contains("  tv field tv"),
            "no term vectors in the oracle report"
        );
        // The doc-values updates made before `forceMerge(2)` and carried
        // through it must be reported.
        for (target, value) in [
            (id(5), "  dv dv_num 777"),
            (id(10), "  dv dv_bin updated-10"),
        ] {
            let section: Vec<&str> = oracle
                .lines()
                .skip_while(|l| !l.starts_with(&format!("doc {target} = ")))
                .take_while(|l| !l.starts_with("doc ") || l.contains(&target))
                .collect();
            assert!(
                section.contains(&value),
                "the oracle report must show {target} with `{}`: {section:?}",
                value.trim()
            );
        }
        writeln!(
            report,
            "== oracle report ({} lines)",
            oracle.lines().count()
        )
        .unwrap();
    }

    // The index shape the corpus is built to have: several segments, one
    // with deletions.
    for (name, rep) in [
        ("Lucene on R", &lucene_on_r),
        ("Lucene on L", &lucene_on_l),
        ("Rucene on R", &rucene_on_r),
        ("Rucene on L", &rucene_on_l),
    ] {
        let Some(rep) = rep else { continue };
        let leaves: Vec<(i64, i64)> = rep
            .lines()
            .filter(|l| l.starts_with("leaf "))
            .map(|l| {
                let field = |key: &str| -> i64 {
                    l.split(' ')
                        .find_map(|t| t.strip_prefix(key))
                        .and_then(|v| v.parse().ok())
                        .expect("a leaf field")
                };
                (field("maxDoc="), field("numDocs="))
            })
            .collect();
        if leaves.len() < 2 || !leaves.iter().any(|(max, num)| num < max) {
            failures.push(format!(
                "{name}: the index must have at least 2 leaves and one with deletions, got (maxDoc, numDocs) {leaves:?}"
            ));
        }
    }

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

    // File listings: reported, never failed on.
    let (ok_r, files_r, err_r) = run_java("files", &r);
    let (ok_l, files_l, err_l) = run_java("files", &l);
    if ok_r && ok_l {
        for (name, listing) in [("R", &files_r), ("L", &files_l)] {
            let has_dv_gen = listing.lines().any(|l| {
                l.split(' ')
                    .find_map(|t| t.strip_prefix("dvGen="))
                    .and_then(|v| v.parse::<i64>().ok())
                    .is_some_and(|g| g > 0)
            });
            if !has_dv_gen {
                failures.push(format!(
                    "files({name}): no segment has doc-values updates (dvGen > 0):\n{listing}"
                ));
            }
        }
        writeln!(
            report,
            "== file listing R vs L\n{}",
            file_listing_report(&r, &files_r, &l, &files_l)
        )
        .unwrap();
    } else {
        failures.push(format!(
            "Lucene cannot list the files of a commit:\nR: {}\nL: {}",
            java_errors(&err_r),
            java_errors(&err_l)
        ));
    }

    // Positive control: a flipped byte in the Rucene index fails Lucene's
    // CheckIndex. The run must complete and blame the corrupted segment, so a
    // JVM that fails to run at all cannot pass.
    let (flipped, segment) = corrupt_copy(&r, &corrupt);
    let check = java_check(&corrupt);
    writeln!(
        report,
        "== positive control (Lucene CheckIndex): flipped {flipped}\n  completed={} clean={} numBadSegments={} {segment} error={:?}",
        check.completed,
        check.clean,
        check.num_bad_segments,
        check.segment_error(&segment)
    )
    .unwrap();
    assert!(
        check.completed
            && !check.clean
            && check.num_bad_segments >= 1
            && check.segment_error(&segment).is_some(),
        "Lucene CheckIndex must complete and report segment {segment} as bad ({flipped}):\n{}",
        check.report
    );

    // Positive control: a flipped byte in the Lucene index fails Rucene's
    // CheckIndex, which verifies every file's checksum at `Level::MAX_VALUE`
    // (`CheckIndex.java:4412, 4418`).
    let (flipped, segment) = corrupt_copy(&l, &corrupt_l);
    let check = rucene_check(&corrupt_l);
    writeln!(
        report,
        "== positive control (Rucene CheckIndex): flipped {flipped}\n  completed={} clean={} numBadSegments={} {segment} error={:?}",
        check.completed,
        check.clean,
        check.num_bad_segments,
        check.segment_error(&segment)
    )
    .unwrap();
    assert!(
        check.completed
            && !check.clean
            && check.num_bad_segments >= 1
            && check.segment_error(&segment).is_some(),
        "Rucene CheckIndex must complete and report segment {segment} as bad ({flipped}):\n{}",
        check.report
    );

    println!("{report}");
    assert!(
        failures.is_empty(),
        "{} interop failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// `IndexWriter.forceMergeDeletes()` under `TieredMergePolicy` selects the
/// merges Lucene selects (`TieredMergePolicy.java:964-1018`). Only the five
/// segments with deletions are eligible; `findForcedDeletesMerges` bounds each
/// merge by the live documents of the whole index divided by
/// `targetSearchConcurrency` (`getMaxAllowedDocs(infos.totalMaxDoc(),
/// totalDelCount)`), so the five merge into one. A budget computed over the
/// eligible segments alone would split them.
#[test]
fn force_merge_deletes_selects_the_merges_lucene_selects() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("index_interop");
    let r = fresh_dir(&root, "forced-deletes-rucene");
    let l = fresh_dir(&root, "forced-deletes-lucene");

    let (ok, java_listing, err) = run_java("forced-deletes", &l);
    assert!(
        ok,
        "Lucene failed to write the forced-deletes corpus:\n{err}"
    );
    write_forced_deletes(&r);
    let (ok, rucene_listing, err) = run_java("files", &r);
    assert!(
        ok,
        "Lucene cannot list the Rucene commit:\n{}",
        java_errors(&err)
    );

    let expected = segment_lines(&java_listing);
    assert!(
        expected.contains("maxDoc=85 delCount=0"),
        "the oracle merges the five segments with deletions into one: {expected}"
    );
    assert_eq!(
        segment_lines(&rucene_listing),
        expected,
        "the segments after forceMergeDeletes differ from Lucene's"
    );
}
