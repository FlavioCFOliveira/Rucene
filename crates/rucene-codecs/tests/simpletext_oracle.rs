//! Oracle parity of the `SimpleText` codec (rmp #332, #333) against
//! Apache Lucene 10.5.0's `lucene-codecs`.
//!
//! The Java oracle (`SimpleTextOracle` and `SimpleTextReadOracle`, in the
//! workspace harness `crates/randomizedtesting/tests/fixtures/oracle/src/
//! org/apache/lucene/codecs/simpletext/`) writes three indexes with
//! `SimpleTextCodec` and records them in `tests/fixtures/`:
//!
//! * `nocfs` and `cfs`: the documents and operations of
//!   `tests/portability/index_interop.rs` (text with positions, offsets and
//!   payloads, term vectors, stored values of every type, the five doc-values
//!   types, points, float and byte vectors; deletes by term and by query,
//!   doc-values updates, several commits, `forceMerge(2)`,
//!   `forceMergeDeletes()`), without and with compound files;
//! * `edge`: twelve documents carrying every value kind at its limits
//!   (`Long.MIN_VALUE`/`MAX_VALUE`, NaN and signed zeros, empty and non-ASCII
//!   terms and strings, empty fields, every index option and term-vector
//!   option, the doc-values skip index, missing values, points of 1, 3, 4 and
//!   16 bytes per dimension with fewer index than data dimensions, vectors of
//!   every similarity), with a soft-deletes field and an index sort.
//!
//! The tests write the same indexes with Rucene and compare every file byte
//! for byte; read Lucene's files and Rucene's with Rucene and compare what is
//! read with what Lucene read; run Lucene's reader (`SimpleTextReadOracle`) on
//! Rucene's files; and compare the errors corrupted copies raise.
//!
//! # Determinism
//!
//! A `SimpleText` file carries no segment id, so the bytes are a function of
//! the documents and operations only, except two files: `segments_N`
//! (Lucene's own binary format, which holds the segment ids and is not
//! written by this codec) and the `.si`, whose id line holds the random
//! segment id and whose diagnostics hold the environment (operating system,
//! JVM, timestamp) and, in Java, iterate in an order drawn per JVM
//! (`Map.copyOf`; see `SegmentInfo::get_diagnostics`). The `.si` files are
//! therefore compared line by line with the id line skipped and the
//! diagnostics compared as a set of keys, with the values of the
//! environment keys skipped; the format-level `.si` test of
//! `simpletext_formats.rs` pins the id and the diagnostics and compares
//! bytes.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Once};

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PackedTokenAttributeImpl, PayloadAttribute,
    PayloadAttributeImpl, PositionIncrementAttribute,
};
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::codecs::Codec;
use rucene::document::{
    BinaryDocValuesField, BinaryPoint, Document, DoublePoint, Field, FieldType, FloatPoint,
    IntPoint, KnnByteVectorField, KnnFloatVectorField, LongPoint, NumericDocValuesField,
    NumericValue, SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField,
    Store, StoredField, StringField, TextField,
};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{MergePolicy, TieredMergePolicy};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, IndexOptions, IndexReader, LeafReader,
    PostingsEnum, SeekStatus, StoredFieldVisitor, StoredFieldVisitorStatus, Term, VectorEncoding,
    VectorSimilarityFunction, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::search::knn::TopKnnCollector;
use rucene::search::{BitsAcceptDocs, KnnCollector, Sort, SortField, SortFieldType, NO_MORE_DOCS};
use rucene::store::{Directory, NIOFSDirectory};
use rucene::util::{AttributeSource, BytesRef};
use rucene_codecs::simpletext::SimpleTextCodec;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn rows(name: &str) -> Vec<Vec<String>> {
    fixture(name)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// Inverse of the oracle's `escape`: `\\` and `\xHH`.
fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            if b[i + 1] == b'\\' {
                out.push(b'\\');
                i += 2;
            } else {
                assert_eq!(b[i + 1], b'x', "bad escape in {s}");
                out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).unwrap());
                i += 4;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// The oracle's `escape`.
fn escape(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &v in bytes {
        if v == b'\\' {
            s.push_str("\\\\");
        } else if (0x20..0x7f).contains(&v) {
            s.push(v as char);
        } else {
            s.push_str(&format!("\\x{v:02x}"));
        }
    }
    s
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The files Lucene wrote for `case`: name to bytes.
fn java_files(case: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for row in rows("simpletext_files.tsv") {
        if row[0] == case {
            let bytes = unescape(&row[4]);
            assert_eq!(bytes.len().to_string(), row[2], "{case} {}", row[1]);
            assert_eq!(format!("{:x}", fnv(&bytes)), row[3], "{case} {}", row[1]);
            files.insert(row[1].clone(), bytes);
        }
    }
    assert!(!files.is_empty(), "no fixture rows for {case}");
    files
}

/// What Lucene read from its own index `case`.
fn java_reads(case: &str) -> Vec<String> {
    rows("simpletext_reads.tsv")
        .into_iter()
        .filter(|r| r[0] == case)
        .map(|r| String::from_utf8(unescape(&r[1])).unwrap())
        .collect()
}

/// A fresh temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> TempDir {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "{prefix}-{}-{n}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p.canonicalize().unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Puts `lucene-codecs` on the "class path", as Java does at start-up.
fn install_codecs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        rucene::codecs::reload_codecs(rucene_codecs::class_loader()).unwrap();
    });
}

fn write_files(dir: &Path, files: &BTreeMap<String, Vec<u8>>) {
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

fn read_dir(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "write.lock" {
            continue;
        }
        files.insert(name, std::fs::read(entry.path()).unwrap());
    }
    files
}

// ---------------------------------------------------------------------------
// The corpus: SimpleTextOracle.doc and the operations of writeCorpus
// ---------------------------------------------------------------------------

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

fn byte_vector(i: i32) -> Vec<u8> {
    [(i * 3) % 50, (i * 7) % 60 - 30, i % 20, (i * 13) % 40 - 20]
        .iter()
        .map(|&v| v as i8 as u8)
        .collect()
}

fn field_type(f: impl FnOnce(&mut FieldType)) -> FieldType {
    let mut t = FieldType::new();
    f(&mut t);
    t.freeze();
    t
}

fn body_type() -> FieldType {
    field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_stored(true).unwrap();
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            .unwrap();
    })
}

fn tv_type() -> FieldType {
    field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            .unwrap();
        t.set_store_term_vectors(true).unwrap();
        t.set_store_term_vector_positions(true).unwrap();
        t.set_store_term_vector_offsets(true).unwrap();
        t.set_store_term_vector_payloads(true).unwrap();
    })
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

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

fn new_config() -> IndexWriterConfig {
    install_codecs();
    let mut config = IndexWriterConfig::new();
    config
        .set_codec(Arc::new(SimpleTextCodec::new()) as Arc<dyn Codec>)
        .unwrap();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    config
}

fn open_dir(path: &Path) -> Arc<dyn Directory> {
    Arc::new(NIOFSDirectory::open(path).unwrap())
}

/// `SimpleTextOracle.writeCorpus`.
fn write_corpus(path: &Path, compound: bool) {
    let mut config = new_config();
    if !compound {
        let policy = TieredMergePolicy::new();
        policy.set_no_cfs_ratio(0.0).unwrap();
        config.set_merge_policy(Arc::new(policy)).unwrap();
        config.set_use_compound_file(false).unwrap();
    }
    let w = IndexWriter::new(open_dir(path), config).unwrap();
    let add_batch = |batch: usize| {
        let (from, to) = BATCHES[batch];
        for i in from..to {
            w.add_document(&doc(i)).unwrap();
        }
    };
    add_batch(0);
    w.commit().unwrap();
    w.delete_documents_by_terms(vec![term("id", &id(3))])
        .unwrap();
    w.delete_documents_by_terms(vec![term("id", &id(17))])
        .unwrap();
    w.delete_documents_by_queries(vec![
        IntPoint::new_range_query("p_int", &[50], &[60]).unwrap()
    ])
    .unwrap();
    add_batch(1);
    w.update_numeric_doc_value(term("id", &id(5)), "dv_num", 777)
        .unwrap();
    w.update_binary_doc_value(term("id", &id(130)), "dv_bin", bytes("updated-130"))
        .unwrap();
    w.commit().unwrap();
    add_batch(2);
    w.commit().unwrap();
    w.force_merge(2).unwrap();
    add_batch(3);
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
    w.delete_documents_by_queries(vec![
        IntPoint::new_range_query("p_2d", &[0, 3], &[19, 4]).unwrap()
    ])
    .unwrap();
    w.force_merge_deletes().unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// The edge documents: SimpleTextOracle.edgeDoc and writeEdge
// ---------------------------------------------------------------------------

const EDGE_DOCS: i32 = 12;
const INTS: [i32; 5] = [i32::MIN, i32::MAX, 0, -1, 7];
const LONGS: [i64; 5] = [i64::MIN, i64::MAX, 0, -1, 1_000_000_007];
const FLOATS: [f32; 12] = [
    f32::NAN,
    -0.0,
    0.0,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::MAX,
    1.0E-30,
    123456.78,
    1.0E7,
    0.001,
    9.999999E-4,
    9999999.0,
];
const DOUBLES: [f64; 12] = [
    f64::NAN,
    -0.0,
    0.0,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::MAX,
    1.0E-300,
    123456.789,
    1.0E7,
    0.001,
    9.99999999999E-4,
    9999999.999,
];
const NUMS: [i64; 6] = [i64::MIN, i64::MAX, -1, 0, 42, i64::MIN + 1];
const UNIT: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.6, 0.8, 0.0],
    [0.0, -0.6, 0.8],
    [-1.0, 0.0, 0.0],
];

fn edge_text(i: i32) -> String {
    match i {
        0 => String::new(),
        1 => "h\u{e9}llo w\u{f6}rld \u{65e5}\u{672c}".to_string(),
        _ => format!(
            "edge{i} common words{}",
            if i % 3 == 0 { " common" } else { "" }
        ),
    }
}

fn edge_str(i: i32) -> String {
    match i {
        0 => String::new(),
        1 => "na\u{ef}ve-\u{65e5}\u{672c}-\u{1f600}".to_string(),
        2 => "line\nbreak\\slash".to_string(),
        _ => format!("s{}", i % 4),
    }
}

fn edge_stored_str(i: i32) -> String {
    match i {
        0 => String::new(),
        1 => "tab\there\nnew\\line \u{fc}n\u{ef}\u{1f600}".to_string(),
        _ => format!("v{i}"),
    }
}

fn edge_bytes(i: i32) -> Vec<u8> {
    match i {
        0 => Vec::new(),
        1 => vec![0x0a, 0x5c, 0xff, 0x00],
        _ => vec![i as u8, 0x0a],
    }
}

fn euclidean_vector(i: i32) -> [f32; 3] {
    let u = UNIT[(i % 6) as usize];
    [
        u[0] * (i + 1) as f32 / 4.0,
        if u[1] == 0.0 && i == 0 { -0.0 } else { u[1] },
        u[2] + 2.5E-5,
    ]
}

fn edge_byte_vector(i: i32, salt: i32) -> Vec<u8> {
    vec![
        (i * 3 - 5 + salt) as i8 as u8,
        (-i) as i8 as u8,
        ((i * i + salt) % 100) as i8 as u8,
    ]
}

fn edge_doc(i: i32) -> Document {
    let docs_type = field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_index_options(IndexOptions::DOCS).unwrap();
    });
    let freqs_type = field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_index_options(IndexOptions::DOCS_AND_FREQS).unwrap();
    });
    let tvp_type = field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            .unwrap();
        t.set_store_term_vectors(true).unwrap();
        t.set_store_term_vector_positions(true).unwrap();
    });
    let tvo_type = field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_index_options(IndexOptions::DOCS_AND_FREQS).unwrap();
        t.set_store_term_vectors(true).unwrap();
        t.set_store_term_vector_offsets(true).unwrap();
    });
    let pdata_type = field_type(|t| {
        t.set_dimensions_with_index_count(3, 2, 4).unwrap();
    });

    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("e_id", format!("e{i}"), Store::YES).unwrap(),
    ));
    d.add(Box::new(
        TextField::new("e_text", edge_text(i), Store::NO).unwrap(),
    ));
    d.add(Box::new(
        StringField::new("e_str", edge_str(i), Store::YES).unwrap(),
    ));
    d.add(Box::new(
        Field::new("e_docs", format!("d{} shared", i % 3), docs_type).unwrap(),
    ));
    d.add(Box::new(
        Field::new("e_freqs", format!("f{0} f{0} g", i % 2), freqs_type).unwrap(),
    ));
    d.add(Box::new(
        Field::new("e_tvp", format!("tv{} pos", i % 3), tvp_type).unwrap(),
    ));
    d.add(Box::new(
        Field::new("e_tvo", format!("off{} set", i % 2), tvo_type).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number("e_int", NumericValue::Int(INTS[i as usize % INTS.len()])).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number(
            "e_long",
            NumericValue::Long(LONGS[i as usize % LONGS.len()]),
        )
        .unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number(
            "e_float",
            NumericValue::Float(FLOATS[i as usize % FLOATS.len()]),
        )
        .unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_number(
            "e_double",
            NumericValue::Double(DOUBLES[i as usize % DOUBLES.len()]),
        )
        .unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_bytes("e_bytes", BytesRef::new(edge_bytes(i))).unwrap(),
    ));
    d.add(Box::new(
        StoredField::new_string("e_sstr", edge_stored_str(i)).unwrap(),
    ));
    if i % 7 != 3 {
        d.add(Box::new(NumericDocValuesField::indexed_field(
            "e_num",
            NUMS[i as usize % NUMS.len()],
        )));
    }
    d.add(Box::new(NumericDocValuesField::new(
        "e_num2",
        i64::from(i * 3 - 5),
    )));
    d.add(Box::new(NumericDocValuesField::new(
        "e_sortkey",
        i64::from((i * 7) % 12),
    )));
    if i != 3 {
        let bin = match i {
            0 => Vec::new(),
            1 => vec![0x0a, 0x5c],
            _ => format!("bin{i}").into_bytes(),
        };
        d.add(Box::new(BinaryDocValuesField::new(
            "e_bin",
            BytesRef::new(bin),
        )));
    }
    if i != 2 {
        d.add(Box::new(SortedDocValuesField::indexed_field(
            "e_sorted",
            bytes(&format!("\u{fc}{}", i % 3)),
        )));
    }
    if i != 5 {
        d.add(Box::new(SortedSetDocValuesField::indexed_field(
            "e_sset",
            bytes(&format!("x{}", i % 4)),
        )));
        d.add(Box::new(SortedSetDocValuesField::indexed_field(
            "e_sset",
            bytes(&format!("y{}", i % 2)),
        )));
        d.add(Box::new(SortedSetDocValuesField::indexed_field(
            "e_sset",
            bytes(&format!("x{}", i % 4)),
        )));
    }
    if i != 6 {
        let first = match i % 4 {
            0 => i64::MIN,
            1 => i64::MAX,
            _ => i64::from(i),
        };
        d.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "e_snum", first,
        )));
        d.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "e_snum",
            i64::from(-i),
        )));
    }
    if i == 7 {
        d.add(Box::new(NumericDocValuesField::new("e_soft", 1)));
    }
    d.add(Box::new(
        BinaryPoint::new("e_bp1", &[BytesRef::new(vec![(i * 20) as u8])]).unwrap(),
    ));
    d.add(Box::new(
        BinaryPoint::new(
            "e_bp3",
            &[
                BytesRef::new(vec![i as u8, (i * 2) as u8, (255 - i) as u8]),
                BytesRef::new(vec![7, i as u8, 0]),
            ],
        )
        .unwrap(),
    ));
    let b16: Vec<u8> = (0..16).map(|k| (i * 17 + k * 31) as u8).collect();
    d.add(Box::new(
        BinaryPoint::new("e_bp16", &[BytesRef::new(b16)]).unwrap(),
    ));
    d.add(Box::new(
        IntPoint::new("e_int3", &[i, -i, i * 1000]).unwrap(),
    ));
    d.add(Box::new(
        LongPoint::new("e_lp", &[LONGS[i as usize % LONGS.len()]]).unwrap(),
    ));
    let pdata: Vec<u8> = (0..12).map(|k| (i * 5 + k) as u8).collect();
    d.add(Box::new(
        BinaryPoint::new_with_packed("e_pdata", BytesRef::new(pdata), pdata_type).unwrap(),
    ));
    if i != 4 {
        d.add(Box::new(
            KnnFloatVectorField::new(
                "e_vf_euc",
                &euclidean_vector(i),
                VectorSimilarityFunction::EUCLIDEAN,
            )
            .unwrap(),
        ));
    }
    d.add(Box::new(
        KnnFloatVectorField::new(
            "e_vf_dot",
            &UNIT[(i % 6) as usize],
            VectorSimilarityFunction::DOT_PRODUCT,
        )
        .unwrap(),
    ));
    d.add(Box::new(
        KnnFloatVectorField::new(
            "e_vf_cos",
            &UNIT[((i + 1) % 6) as usize],
            VectorSimilarityFunction::COSINE,
        )
        .unwrap(),
    ));
    d.add(Box::new(
        KnnFloatVectorField::new(
            "e_vf_mip",
            &UNIT[((i + 2) % 6) as usize],
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        )
        .unwrap(),
    ));
    for (name, salt, sim) in [
        ("e_vb_euc", 0, VectorSimilarityFunction::EUCLIDEAN),
        ("e_vb_dot", 1, VectorSimilarityFunction::DOT_PRODUCT),
        ("e_vb_cos", 2, VectorSimilarityFunction::COSINE),
        (
            "e_vb_mip",
            3,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        ),
    ] {
        d.add(Box::new(
            KnnByteVectorField::new(name, &edge_byte_vector(i, salt), sim).unwrap(),
        ));
    }
    d
}

/// `SimpleTextOracle.writeEdge`.
fn write_edge(path: &Path) {
    let mut config = new_config();
    config.set_use_compound_file(false).unwrap();
    config.set_soft_deletes_field(Some("e_soft".to_string()));
    config
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("e_sortkey".to_string()),
                SortFieldType::Long,
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
    let w = IndexWriter::new(open_dir(path), config).unwrap();
    for i in 0..EDGE_DOCS {
        w.add_document(&edge_doc(i)).unwrap();
    }
    w.delete_documents_by_terms(vec![term("e_id", "e9")])
        .unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Format-level files: SimpleTextOracle.writeFormats
// ---------------------------------------------------------------------------

/// `SimpleTextOracle.SEGMENT_ID`.
const SEGMENT_ID: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

/// `SimpleTextOracle.ArrayTree`: a mutable point tree over arrays.
struct ArrayTree {
    values: Vec<Vec<u8>>,
    docs: Vec<i32>,
    saved_values: Vec<Vec<u8>>,
    saved_docs: Vec<i32>,
}

impl ArrayTree {
    fn new(values: Vec<Vec<u8>>, docs: Vec<i32>) -> Self {
        let n = values.len();
        Self {
            values,
            docs,
            saved_values: vec![Vec::new(); n],
            saved_docs: vec![0; n],
        }
    }
}

impl rucene::index::PointTree for ArrayTree {
    fn clone_tree(&self) -> rucene::error::Result<Box<dyn rucene::index::PointTree>> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }

    fn move_to_child(&mut self) -> rucene::error::Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> rucene::error::Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> rucene::error::Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> rucene::error::Result<&[u8]> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }

    fn max_packed_value(&self) -> rucene::error::Result<&[u8]> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }

    fn size(&self) -> rucene::error::Result<i64> {
        Ok(self.values.len() as i64)
    }

    fn visit_doc_ids(
        &mut self,
        _visitor: &mut dyn rucene::index::IntersectVisitor,
    ) -> rucene::error::Result<()> {
        Err(rucene::error::LuceneError::UnsupportedOperation(
            String::new(),
        ))
    }

    fn visit_doc_values(
        &mut self,
        visitor: &mut dyn rucene::index::IntersectVisitor,
    ) -> rucene::error::Result<()> {
        for i in 0..self.values.len() {
            visitor.visit_with_value(self.docs[i], &self.values[i])?;
        }
        Ok(())
    }

    fn as_mutable(&mut self) -> Option<&mut dyn rucene::index::MutablePointTree> {
        Some(self)
    }
}

impl rucene::index::MutablePointTree for ArrayTree {
    fn value(&self, i: i32) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(&self.values[i as usize])
    }

    fn byte_at(&self, i: i32, k: i32) -> u8 {
        self.values[i as usize][k as usize]
    }

    fn doc_id(&self, i: i32) -> i32 {
        self.docs[i as usize]
    }

    fn swap(&mut self, i: i32, j: i32) {
        self.values.swap(i as usize, j as usize);
        self.docs.swap(i as usize, j as usize);
    }

    fn save(&mut self, i: i32, j: i32) {
        self.saved_values[j as usize] = self.values[i as usize].clone();
        self.saved_docs[j as usize] = self.docs[i as usize];
    }

    fn restore(&mut self, i: i32, j: i32) {
        for k in i as usize..j as usize {
            self.values[k] = self.saved_values[k].clone();
            self.docs[k] = self.saved_docs[k];
        }
    }
}

/// `SimpleTextOracle.formatValue`.
fn format_value(dims: i32, i: i32) -> Vec<u8> {
    if dims == 1 {
        let v = (i * 7919) % 100003;
        return v.to_be_bytes().to_vec();
    }
    let d = [(i * 31) % 997, (i * 17) % 1009, (i * 13) % 251];
    d.iter().flat_map(|v| [(v >> 8) as u8, *v as u8]).collect()
}

fn format_config(dims: i32) -> rucene::util::bkd::BKDConfig {
    if dims == 1 {
        rucene::util::bkd::BKDConfig::new(1, 1, 4, 512).unwrap()
    } else {
        rucene::util::bkd::BKDConfig::new(3, 2, 2, 512).unwrap()
    }
}

/// `SimpleTextOracle.writeBkd`.
fn write_bkd(dir: &Arc<dyn Directory>, file: &str, dims: i32, n: i32, max_mb: f64, mutable: bool) {
    use rucene_codecs::simpletext::bkd_writer::SimpleTextBKDWriter;
    let mut out = dir
        .create_output(file, &*rucene::store::DEFAULT_IO_CONTEXT)
        .unwrap();
    let mut w = SimpleTextBKDWriter::new(
        n,
        Arc::clone(dir),
        "_9",
        format_config(dims),
        max_mb,
        i64::from(n),
    )
    .unwrap();
    let fp = if mutable {
        let values = (0..n).map(|i| format_value(dims, i)).collect();
        let docs = (0..n).map(|i| (i * 37) % n).collect();
        let mut tree = ArrayTree::new(values, docs);
        w.write_field(out.as_mut(), "f", &mut tree).unwrap()
    } else {
        for i in 0..n {
            w.add(&format_value(dims, i), (i * 37) % n).unwrap();
        }
        w.finish(out.as_mut()).unwrap()
    };
    w.close().unwrap();
    rucene_codecs::simpletext::util::write_str(out.as_mut(), &format!("fp {fp}")).unwrap();
    rucene_codecs::simpletext::util::write_newline(out.as_mut()).unwrap();
    out.close().unwrap();
}

#[allow(clippy::too_many_arguments)]
fn format_field_info(
    name: &str,
    number: i32,
    io: IndexOptions,
    tv: bool,
    omit_norms: bool,
    payloads: bool,
    dv: DocValuesType,
    skip: DocValuesSkipIndexType,
    dv_gen: i64,
    points: (i32, i32, i32),
    vector: (i32, VectorEncoding, VectorSimilarityFunction),
    soft: bool,
    parent: bool,
) -> FieldInfo {
    FieldInfo::new_full(
        name,
        number,
        tv,
        omit_norms,
        payloads,
        io,
        dv,
        skip,
        dv_gen,
        rucene::util::java_collections::JavaStringMap::default(),
        points.0,
        points.1,
        points.2,
        vector.0,
        vector.1,
        vector.2,
        soft,
        parent,
    )
    .unwrap()
}

/// The pinned segment info of `SimpleTextOracle.writeFormats`.
fn format_segment_info(dir: &Arc<dyn Directory>) -> rucene::index::SegmentInfo {
    let si = rucene::index::SegmentInfo::new(
        Arc::clone(dir),
        rucene::util::Version::LATEST,
        Some(rucene::util::Version::from_bits(9, 12, 0).unwrap()),
        "_9".to_string(),
        7,
        true,
        false,
        Arc::new(SimpleTextCodec::new()),
        [("source".to_string(), "flush".to_string())]
            .into_iter()
            .collect(),
        SEGMENT_ID,
        std::collections::HashMap::new(),
        Sort::new_fields(vec![
            SortField::new_reverse(Some("s".to_string()), SortFieldType::String, true).unwrap(),
            rucene::search::SortedNumericSortField::new("n", SortFieldType::Long)
                .unwrap()
                .into(),
        ])
        .unwrap(),
    )
    .unwrap();
    si.put_attribute("b", "2");
    si.put_attribute("a", "1");
    si.put_attribute("SimpleText.key", "line\nvalue");
    si.set_files(
        [
            "_9.inf", "_9.fld", "_9.pst", "_9.dat", "_9.tvc", "_9.len", "_9.vec",
        ]
        .iter()
        .map(|s| s.to_string()),
    )
    .unwrap();
    si
}

/// The field infos of `SimpleTextOracle.writeFormats`.
fn format_field_infos() -> rucene::index::FieldInfos {
    use DocValuesSkipIndexType as S;
    use DocValuesType as D;
    use VectorEncoding as E;
    use VectorSimilarityFunction as V;
    let all = format_field_info(
        "f_all",
        0,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
        true,
        false,
        true,
        D::SORTED_SET,
        S::RANGE,
        3,
        (3, 2, 4),
        (8, E::BYTE, V::COSINE),
        false,
        false,
    );
    all.put_attribute("k2", "v2");
    all.put_attribute("k1", "v1");
    all.put_attribute("PerFieldPostingsFormat.format", "SimpleText");
    rucene::index::FieldInfos::new(vec![
        all,
        format_field_info(
            "f_custom",
            1,
            IndexOptions::DOCS_AND_CUSTOM_FREQS,
            false,
            true,
            false,
            D::NUMERIC,
            S::NONE,
            -1,
            (0, 0, 0),
            (0, E::FLOAT32, V::EUCLIDEAN),
            false,
            false,
        ),
        format_field_info(
            "f_soft",
            2,
            IndexOptions::NONE,
            false,
            false,
            false,
            D::NUMERIC,
            S::NONE,
            -1,
            (0, 0, 0),
            (0, E::FLOAT32, V::EUCLIDEAN),
            true,
            false,
        ),
        format_field_info(
            "f_parent",
            3,
            IndexOptions::NONE,
            false,
            false,
            false,
            D::NUMERIC,
            S::NONE,
            -1,
            (0, 0, 0),
            (0, E::FLOAT32, V::EUCLIDEAN),
            false,
            true,
        ),
        format_field_info(
            "f_mip",
            4,
            IndexOptions::NONE,
            false,
            false,
            false,
            D::NONE,
            S::NONE,
            -1,
            (0, 0, 0),
            (16, E::FLOAT32, V::MAXIMUM_INNER_PRODUCT),
            false,
            false,
        ),
        format_field_info(
            "f_dot",
            5,
            IndexOptions::DOCS,
            false,
            false,
            false,
            D::BINARY,
            S::NONE,
            9,
            (0, 0, 0),
            (2, E::FLOAT32, V::DOT_PRODUCT),
            false,
            false,
        ),
    ])
    .unwrap()
}

/// `SimpleTextOracle.writeFormats`.
fn write_formats(path: &Path) {
    let dir = open_dir(path);
    let codec = SimpleTextCodec::new();
    let si = format_segment_info(&dir);
    codec
        .segment_info_format()
        .write(dir.as_ref(), &si, &*rucene::store::DEFAULT_IO_CONTEXT)
        .unwrap();
    let infos = format_field_infos();
    for suffix in ["", "3"] {
        codec
            .field_infos_format()
            .write(
                dir.as_ref(),
                &si,
                suffix,
                &infos,
                &*rucene::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap();
    }
    write_bkd(&dir, "_9.bkd1m", 1, 1200, 16.0, true);
    write_bkd(&dir, "_9.bkdnm", 3, 1100, 16.0, true);
    write_bkd(&dir, "_9.bkd1o", 1, 3000, 0.01, false);
    write_bkd(&dir, "_9.bkdno", 3, 2500, 0.02, false);
}

fn write_case(case: &str, path: &Path) {
    match case {
        "formats" => write_formats(path),
        "nocfs" => write_corpus(path, false),
        "cfs" => write_corpus(path, true),
        "edge" => write_edge(path),
        _ => unreachable!("{case}"),
    }
}

// ---------------------------------------------------------------------------
// Scripted token stream: SimpleTextOracle.ScriptedTokenStream
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
// The dump: SimpleTextReadOracle.dump, line for line
// ---------------------------------------------------------------------------

const ADVANCE_TARGETS: [i32; 9] = [0, 1, 5, 9, 17, 33, 65, 129, 257];

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

fn opt_hex(b: Option<&[u8]>) -> String {
    b.map_or_else(|| "-".to_string(), hex)
}

/// `Integer.toHexString(Float.floatToRawIntBits(v))`.
fn fbits(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

fn enum_name<T: std::fmt::Debug>(v: T) -> String {
    format!("{v:?}")
}

fn dump(path: &Path) -> Vec<String> {
    install_codecs();
    let reader: Arc<dyn IndexReader> = directory_reader::open(open_dir(path)).unwrap();
    let mut out = vec![format!(
        "reader maxDoc={} numDocs={}",
        reader.max_doc(),
        reader.num_docs()
    )];
    for ctx in Arc::clone(&reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        dump_leaf(ctx.ord(), leaf.as_ref(), &mut out);
    }
    let searcher = rucene::search::IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let merged = rucene::index::FieldInfos::get_merged_field_infos(Arc::clone(&reader)).unwrap();
    let boxes: [(&str, &[i32], &[i32]); 3] = [
        ("p_int", &[10], &[60]),
        ("p_2d", &[3, 2], &[10, 10]),
        ("e_int3", &[2, -9, 1000], &[9, -2, 9000]),
    ];
    for (name, lo, hi) in boxes {
        if merged.field_info(name).is_some() {
            let query = IntPoint::new_range_query(name, lo, hi).unwrap();
            out.push(format!("count {name}={}", searcher.count(query).unwrap()));
        }
    }
    out
}

fn positions(pe: &mut dyn PostingsEnum, sb: &mut String) {
    let freq = pe.freq().unwrap();
    sb.push_str(&format!("f={freq}"));
    for _ in 0..freq {
        let pos = pe.next_position().unwrap();
        sb.push_str(&format!(
            " [{pos},{},{},{}]",
            pe.start_offset(),
            pe.end_offset(),
            opt_hex(pe.get_payload().unwrap())
        ));
    }
}

fn dump_leaf(ord: i32, leaf: &dyn LeafReader, out: &mut Vec<String>) {
    let codec = leaf
        .as_segment_reader()
        .and_then(|s| s.get_segment_info().info.codec())
        .map_or_else(|| "null".to_string(), |c| c.name().to_string());
    out.push(format!(
        "leaf ord={ord} maxDoc={} numDocs={} codec={codec}",
        leaf.max_doc(),
        leaf.num_docs()
    ));
    match leaf.get_live_docs() {
        None => out.push(" live all".to_string()),
        Some(live) => {
            let mut sb = " live ".to_string();
            for i in 0..live.length() {
                sb.push(if live.get(i) { '1' } else { '0' });
            }
            out.push(sb);
        }
    }
    let infos = leaf.get_field_infos();
    let fis: Vec<FieldInfo> = infos.iter().cloned().collect();
    for fi in &fis {
        out.push(format!(
            " fi {} num={} io={} tv={} norms={} pay={} dv={} skip={} gen={} pts={},{},{} vec={},{},{} soft={} parent={} atts={}",
            fi.name,
            fi.number,
            enum_name(fi.get_index_options()),
            fi.has_term_vectors(),
            fi.has_norms(),
            fi.has_payloads(),
            enum_name(fi.get_doc_values_type()),
            enum_name(fi.doc_values_skip_index_type()),
            fi.get_doc_values_gen(),
            fi.get_point_dimension_count(),
            fi.get_point_index_dimension_count(),
            fi.get_point_num_bytes(),
            fi.get_vector_dimension(),
            enum_name(fi.get_vector_encoding()),
            enum_name(fi.get_vector_similarity_function()),
            fi.is_soft_deletes_field(),
            fi.is_parent_field(),
            fi.attributes().len()
        ));
    }
    for fi in &fis {
        if fi.get_index_options() != IndexOptions::NONE {
            dump_terms(leaf, fi, out);
        }
    }
    for fi in &fis {
        if fi.has_norms() {
            let mut sb = format!(" norms {}", fi.name);
            match leaf.get_norm_values(&fi.name).unwrap() {
                None => sb.push_str(" null"),
                Some(mut norms) => {
                    let mut d = norms.next_doc().unwrap();
                    while d != NO_MORE_DOCS {
                        sb.push_str(&format!(" {d}={}", norms.long_value().unwrap()));
                        d = norms.next_doc().unwrap();
                    }
                }
            }
            out.push(sb);
        }
    }
    let stored = leaf.stored_fields().unwrap();
    for doc in 0..leaf.max_doc() {
        let mut visitor = DumpVisitor {
            sb: format!(" stored doc={doc}"),
        };
        stored.document_with_visitor(doc, &mut visitor).unwrap();
        out.push(visitor.sb);
    }
    let tvs = leaf.term_vectors().unwrap();
    for doc in 0..leaf.max_doc() {
        if let Some(fields) = tvs.get(doc).unwrap() {
            out.push(format!(" tv doc={doc} size={}", fields.size()));
            let names: Vec<String> = fields.iterator().collect();
            for field in names {
                let terms = fields.terms(&field).unwrap().unwrap();
                out.push(format!(
                    "  tvf {field} size={} sumTTF={} sumDF={} docCount={} pos={} off={} pay={}",
                    terms.size().unwrap(),
                    terms.sum_total_term_freq().unwrap(),
                    terms.sum_doc_freq().unwrap(),
                    terms.doc_count().unwrap(),
                    terms.has_positions(),
                    terms.has_offsets(),
                    terms.has_payloads()
                ));
                let mut te = terms.iterator().unwrap();
                while let Some(t) = te.next().unwrap() {
                    let mut sb = format!(
                        "   t {} ttf={} ",
                        hex(t.slice()),
                        te.total_term_freq().unwrap()
                    );
                    let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap();
                    pe.next_doc().unwrap();
                    positions(pe.as_mut(), &mut sb);
                    out.push(sb);
                }
            }
        }
    }
    for fi in &fis {
        dump_doc_values(leaf, fi, out);
    }
    for fi in &fis {
        if fi.get_point_dimension_count() > 0 {
            dump_points(leaf, fi, out);
        }
    }
    for fi in &fis {
        if fi.get_vector_dimension() > 0 {
            dump_vectors(leaf, fi, out);
        }
    }
}

fn dump_terms(leaf: &dyn LeafReader, fi: &FieldInfo, out: &mut Vec<String>) {
    let Some(terms) = leaf.terms(&fi.name).unwrap() else {
        out.push(format!(" terms {} null", fi.name));
        return;
    };
    out.push(format!(
        " terms {} size={} sumTTF={} sumDF={} docCount={} freqs={} pos={} off={} pay={}",
        fi.name,
        terms.size().unwrap(),
        terms.sum_total_term_freq().unwrap(),
        terms.sum_doc_freq().unwrap(),
        terms.doc_count().unwrap(),
        terms.has_freqs(),
        terms.has_positions(),
        terms.has_offsets(),
        terms.has_payloads()
    ));
    let mut te = terms.iterator().unwrap();
    let mut all = Vec::new();
    while let Some(t) = te.next().unwrap() {
        all.push(t.slice().to_vec());
        out.push(format!(
            "  t {} df={} ttf={}",
            hex(t.slice()),
            te.doc_freq().unwrap(),
            te.total_term_freq().unwrap()
        ));
        let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap();
        let mut d = pe.next_doc().unwrap();
        while d != NO_MORE_DOCS {
            let mut sb = format!("   d={d} ");
            positions(pe.as_mut(), &mut sb);
            out.push(sb);
            d = pe.next_doc().unwrap();
        }
        let mut docs = te.postings(None, POSTINGS_ENUM_NONE).unwrap();
        let mut sb = "   docs".to_string();
        let mut d = docs.next_doc().unwrap();
        while d != NO_MORE_DOCS {
            sb.push_str(&format!(" {d}:{}", docs.freq().unwrap()));
            d = docs.next_doc().unwrap();
        }
        out.push(sb);
        if te.doc_freq().unwrap() > 8 {
            let mut adv = te.postings(None, POSTINGS_ENUM_FREQS).unwrap();
            let mut a = "   adv".to_string();
            let mut doc = -1;
            for target in ADVANCE_TARGETS {
                if target > doc {
                    doc = adv.advance(target).unwrap();
                    a.push_str(&format!(" {target}->{doc}"));
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    a.push_str(&format!(":{}", adv.freq().unwrap()));
                }
            }
            out.push(a);
            let mut ie = te.impacts(POSTINGS_ENUM_FREQS).unwrap();
            let mut im = "   imp".to_string();
            for target in ADVANCE_TARGETS {
                ie.advance_shallow(target).unwrap();
                let impacts = ie.get_impacts().unwrap();
                im.push_str(&format!(" @{target} levels={}", impacts.num_levels()));
                for level in 0..impacts.num_levels() {
                    im.push_str(&format!(" L{level} upTo={}", impacts.doc_id_up_to(level)));
                    let buf = impacts.get_impacts(level);
                    for i in 0..buf.size {
                        im.push_str(&format!(" {}/{}", buf.freqs[i], buf.norms[i]));
                    }
                }
                if ie.doc_id() < target {
                    ie.advance(target).unwrap();
                }
                if ie.doc_id() == NO_MORE_DOCS {
                    break;
                }
            }
            out.push(im);
        }
    }
    let mut targets: Vec<Vec<u8>> = vec![Vec::new()];
    for t in &all {
        targets.push(t.clone());
        let mut past = t.clone();
        past.push(0);
        targets.push(past);
    }
    targets.push(vec![0xff, 0xff]);
    let mut probe = terms.iterator().unwrap();
    let mut sb = "  seek".to_string();
    for target in &targets {
        let status = probe.seek_ceil(&BytesRef::new(target.clone())).unwrap();
        sb.push_str(&format!(" {}:{}", hex(target), enum_name(status)));
        if status != SeekStatus::END {
            sb.push_str(&format!(
                ":{}:{}",
                hex(probe.term().unwrap().slice()),
                probe.doc_freq().unwrap()
            ));
        }
    }
    out.push(sb);
    let mut exact = terms.iterator().unwrap();
    let mut ex = "  exact".to_string();
    for target in &targets {
        let found = exact.seek_exact(&BytesRef::new(target.clone())).unwrap();
        ex.push_str(&format!(" {found}"));
        if found {
            ex.push_str(&format!(":{}", exact.total_term_freq().unwrap()));
        }
    }
    out.push(ex);
}

struct DumpVisitor {
    sb: String,
}

impl StoredFieldVisitor for DumpVisitor {
    fn needs_field(
        &mut self,
        _field_info: &FieldInfo,
    ) -> rucene::error::Result<StoredFieldVisitorStatus> {
        Ok(StoredFieldVisitorStatus::Yes)
    }

    fn string_field(&mut self, fi: &FieldInfo, value: &str) -> rucene::error::Result<()> {
        self.sb
            .push_str(&format!(" {}=string:{}", fi.name, hex(value.as_bytes())));
        Ok(())
    }

    fn binary_field(&mut self, fi: &FieldInfo, value: &[u8]) -> rucene::error::Result<()> {
        self.sb
            .push_str(&format!(" {}=binary:{}", fi.name, hex(value)));
        Ok(())
    }

    fn int_field(&mut self, fi: &FieldInfo, value: i32) -> rucene::error::Result<()> {
        self.sb.push_str(&format!(" {}=int:{value}", fi.name));
        Ok(())
    }

    fn long_field(&mut self, fi: &FieldInfo, value: i64) -> rucene::error::Result<()> {
        self.sb.push_str(&format!(" {}=long:{value}", fi.name));
        Ok(())
    }

    fn float_field(&mut self, fi: &FieldInfo, value: f32) -> rucene::error::Result<()> {
        self.sb
            .push_str(&format!(" {}=float:{}", fi.name, fbits(value)));
        Ok(())
    }

    fn double_field(&mut self, fi: &FieldInfo, value: f64) -> rucene::error::Result<()> {
        self.sb
            .push_str(&format!(" {}=double:{:x}", fi.name, value.to_bits()));
        Ok(())
    }
}

fn dump_doc_values(leaf: &dyn LeafReader, fi: &FieldInfo, out: &mut Vec<String>) {
    let name = &fi.name;
    match fi.get_doc_values_type() {
        DocValuesType::NONE => {}
        DocValuesType::NUMERIC => {
            let mut v = leaf.get_numeric_doc_values(name).unwrap().unwrap();
            let mut sb = format!(" dv {name}");
            let mut d = v.next_doc().unwrap();
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}={}", v.long_value().unwrap()));
                d = v.next_doc().unwrap();
            }
            out.push(sb);
        }
        DocValuesType::BINARY => {
            let mut v = leaf.get_binary_doc_values(name).unwrap().unwrap();
            let mut sb = format!(" dv {name}");
            let mut d = v.next_doc().unwrap();
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}={}", hex(v.binary_value().unwrap().slice())));
                d = v.next_doc().unwrap();
            }
            out.push(sb);
        }
        DocValuesType::SORTED => {
            let mut v = leaf.get_sorted_doc_values(name).unwrap().unwrap();
            let count = v.get_value_count().unwrap();
            let mut sb = format!(" dv {name} count={count}");
            for ord in 0..count {
                sb.push_str(&format!(
                    " o{ord}={}",
                    hex(v.lookup_ord(ord).unwrap().slice())
                ));
            }
            let mut d = v.next_doc().unwrap();
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}={}", v.ord_value().unwrap()));
                d = v.next_doc().unwrap();
            }
            out.push(sb);
        }
        DocValuesType::SORTED_NUMERIC => {
            let mut v = leaf.get_sorted_numeric_doc_values(name).unwrap().unwrap();
            let mut sb = format!(" dv {name}");
            let mut d = v.next_doc().unwrap();
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}="));
                for i in 0..v.doc_value_count().unwrap() {
                    if i > 0 {
                        sb.push(',');
                    }
                    sb.push_str(&v.next_value().unwrap().to_string());
                }
                d = v.next_doc().unwrap();
            }
            out.push(sb);
        }
        DocValuesType::SORTED_SET => {
            let mut v = leaf.get_sorted_set_doc_values(name).unwrap().unwrap();
            let count = v.get_value_count().unwrap();
            let mut sb = format!(" dv {name} count={count}");
            for ord in 0..count {
                sb.push_str(&format!(
                    " o{ord}={}",
                    hex(v.lookup_ord(ord).unwrap().slice())
                ));
            }
            let mut d = v.next_doc().unwrap();
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}="));
                for i in 0..v.doc_value_count().unwrap() {
                    if i > 0 {
                        sb.push(',');
                    }
                    sb.push_str(&v.next_ord().unwrap().to_string());
                }
                d = v.next_doc().unwrap();
            }
            out.push(sb);
        }
    }
    if fi.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE {
        let mut s = leaf.get_doc_values_skipper(name).unwrap().unwrap();
        let mut sb = format!(
            " skip {name} min={} max={} docCount={} maxValueCount={} levels={}",
            s.global_min_value(),
            s.global_max_value(),
            s.global_doc_count(),
            s.max_value_count(),
            s.num_levels()
        );
        let mut target = 0;
        loop {
            s.advance(target).unwrap();
            sb.push_str(&format!(
                " [{},{},{},{},{}]",
                s.min_doc_id(0),
                s.max_doc_id(0),
                s.min_value(0),
                s.max_value(0),
                s.doc_count(0)
            ));
            if s.max_doc_id(0) == NO_MORE_DOCS {
                break;
            }
            target = s.max_doc_id(0) + 1;
        }
        out.push(sb);
    }
}

struct ValuesVisitor<'a> {
    sb: &'a mut String,
}

impl rucene::index::IntersectVisitor for ValuesVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> rucene::error::Result<()> {
        panic!("IllegalStateException")
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        self.sb
            .push_str(&format!(" {doc_id}:{}", hex(packed_value)));
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> rucene::error::Result<rucene::index::Relation> {
        Ok(rucene::index::Relation::CellCrossesQuery)
    }
}

struct IdsVisitor<'a> {
    sb: &'a mut String,
}

impl rucene::index::IntersectVisitor for IdsVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> rucene::error::Result<()> {
        self.sb.push_str(&format!(" {doc_id}"));
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed: &[u8]) -> rucene::error::Result<()> {
        panic!("IllegalStateException")
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> rucene::error::Result<rucene::index::Relation> {
        Ok(rucene::index::Relation::CellCrossesQuery)
    }
}

fn dump_points(leaf: &dyn LeafReader, fi: &FieldInfo, out: &mut Vec<String>) {
    let Some(pv) = leaf.get_point_values(&fi.name).unwrap() else {
        out.push(format!(" points {} null", fi.name));
        return;
    };
    out.push(format!(
        " points {} size={} docCount={} dims={},{},{} min={} max={} treeSize={}",
        fi.name,
        pv.size().unwrap(),
        pv.doc_count().unwrap(),
        pv.num_dimensions().unwrap(),
        pv.num_index_dimensions().unwrap(),
        pv.bytes_per_dimension().unwrap(),
        hex(&pv.min_packed_value().unwrap().unwrap()),
        hex(&pv.max_packed_value().unwrap().unwrap()),
        pv.point_tree().unwrap().size().unwrap()
    ));
    let mut sb = "  pv".to_string();
    pv.point_tree()
        .unwrap()
        .visit_doc_values(&mut ValuesVisitor { sb: &mut sb })
        .unwrap();
    out.push(sb);
    let mut ids = "  pids".to_string();
    pv.point_tree()
        .unwrap()
        .visit_doc_ids(&mut IdsVisitor { sb: &mut ids })
        .unwrap();
    out.push(ids);
    let mut tree = "  tree".to_string();
    let mut root = pv.point_tree().unwrap();
    walk(root.as_mut(), &mut tree);
    out.push(tree);
    out.push(format!(
        "  clone size={}",
        root.clone_tree().unwrap().size().unwrap()
    ));
}

/// `SimpleTextReadOracle.walk`: each node's bounds and size, then its
/// children, in pre-order.
fn walk(t: &mut dyn rucene::index::PointTree, sb: &mut String) {
    sb.push_str(&format!(
        " [{},{},{}",
        hex(t.min_packed_value().unwrap()),
        hex(t.max_packed_value().unwrap()),
        t.size().unwrap()
    ));
    if t.move_to_child().unwrap() {
        loop {
            walk(t, sb);
            if !t.move_to_sibling().unwrap() {
                break;
            }
        }
        t.move_to_parent().unwrap();
    }
    sb.push(']');
}

fn dump_vectors(leaf: &dyn LeafReader, fi: &FieldInfo, out: &mut Vec<String>) {
    let dim = fi.get_vector_dimension();
    let mut collector = TopKnnCollector::new(3, i64::from(i32::MAX));
    let mut accept_docs =
        BitsAcceptDocs::from_live_docs(leaf.get_live_docs(), leaf.max_doc()).unwrap();
    let mut sb = format!(" vec {}", fi.name);
    if fi.get_vector_encoding() == VectorEncoding::FLOAT32 {
        let Some(values) = leaf.get_float_vector_values(&fi.name).unwrap() else {
            out.push(format!(" vec {} null", fi.name));
            return;
        };
        sb.push_str(&format!(
            " size={} dim={}",
            values.size(),
            values.dimension()
        ));
        let mut it = values.iterator().unwrap();
        let mut d = it.next_doc().unwrap();
        while d != NO_MORE_DOCS {
            sb.push_str(&format!(" {d}:"));
            let v = values.vector_value(it.index()).unwrap();
            let parts: Vec<String> = v.iter().map(|x| fbits(*x)).collect();
            sb.push_str(&parts.join(","));
            d = it.next_doc().unwrap();
        }
        let target: Vec<f32> = (0..dim).map(|i| (i + 1) as f32 / 4.0).collect();
        leaf.search_nearest_vectors(&fi.name, &target, &mut collector, &mut accept_docs)
            .unwrap();
    } else {
        let Some(values) = leaf.get_byte_vector_values(&fi.name).unwrap() else {
            out.push(format!(" vec {} null", fi.name));
            return;
        };
        sb.push_str(&format!(
            " size={} dim={}",
            values.size(),
            values.dimension()
        ));
        let mut it = values.iterator().unwrap();
        let mut d = it.next_doc().unwrap();
        while d != NO_MORE_DOCS {
            let v = values.vector_value(it.index()).unwrap();
            sb.push_str(&format!(" {d}:{}", hex(&v)));
            d = it.next_doc().unwrap();
        }
        let target: Vec<u8> = (0..dim).map(|i| (i * 3 - 2) as i8 as u8).collect();
        leaf.search_nearest_vectors_byte(&fi.name, &target, &mut collector, &mut accept_docs)
            .unwrap();
    }
    out.push(sb);
    let visited = collector.visited_count();
    let top = collector.top_docs();
    let mut knn = format!("  knn visited={visited}");
    for sd in &top.score_docs {
        knn.push_str(&format!(" {}:{}", sd.doc, fbits(sd.score)));
    }
    out.push(knn);
}

// ---------------------------------------------------------------------------
// Comparisons
// ---------------------------------------------------------------------------

/// The diagnostics whose values depend on the environment that wrote the
/// segment rather than on the segment.
const ENVIRONMENT_KEYS: [&str; 6] = [
    "os",
    "os.arch",
    "os.version",
    "java.vendor",
    "java.runtime.version",
    "timestamp",
];

/// A `.si` without what varies between two writers of the same segment: the
/// id line and the checksum line are dropped, and the diagnostics block is
/// replaced by its sorted keys, each with its value unless the key is an
/// environment key.
fn normalized_si(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(n) = line.strip_prefix("    diagnostics ") {
            let n: usize = n.parse().unwrap();
            out.push(line.to_string());
            let mut diag = BTreeSet::new();
            for k in 0..n {
                let key = lines[i + 1 + 2 * k].strip_prefix("      key ").unwrap();
                let value = lines[i + 2 + 2 * k].strip_prefix("      value ").unwrap();
                if ENVIRONMENT_KEYS.contains(&key) {
                    diag.insert(format!("{key}=<env>"));
                } else {
                    diag.insert(format!("{key}={value}"));
                }
            }
            out.extend(diag);
            i += 1 + 2 * n;
            continue;
        }
        if line.starts_with("    id ") || line.starts_with("checksum ") {
            i += 1;
            continue;
        }
        out.push(line.to_string());
        i += 1;
    }
    out
}

fn assert_same_files(case: &str, rucene: &BTreeMap<String, Vec<u8>>) {
    let java = java_files(case);
    let java_names: Vec<&String> = java.keys().collect();
    let rucene_names: Vec<&String> = rucene.keys().collect();
    assert_eq!(rucene_names, java_names, "{case}: file listing");
    let mut compared = 0;
    for (name, java_bytes) in &java {
        let rust_bytes = &rucene[name];
        if name.starts_with("segments_") {
            continue;
        }
        if name.ends_with(".si") && case != "formats" {
            assert_eq!(
                normalized_si(rust_bytes),
                normalized_si(java_bytes),
                "{case}: {name} (normalized)"
            );
            continue;
        }
        if rust_bytes != java_bytes {
            let r = String::from_utf8_lossy(rust_bytes);
            let j = String::from_utf8_lossy(java_bytes);
            let line = r
                .lines()
                .zip(j.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            panic!(
                "{case}: {name} differs from Lucene's at line {}:\n rucene: {:?}\n lucene: {:?}",
                line + 1,
                r.lines().nth(line),
                j.lines().nth(line)
            );
        }
        compared += 1;
    }
    assert!(compared > 0);
}

fn assert_same_lines(what: &str, actual: &[String], expected: &[String]) {
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "{what}: line {} differs", i + 1);
    }
    assert_eq!(actual.len(), expected.len(), "{what}: line count");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Rucene writes the bytes Lucene writes, for every file of the three
/// indexes.
#[test]
fn rucene_writes_lucenes_bytes() {
    for case in ["formats", "nocfs", "cfs", "edge"] {
        let tmp = TempDir::new(&format!("simpletext-write-{case}"));
        write_case(case, &tmp.0);
        assert_same_files(case, &read_dir(&tmp.0));
    }
}

/// Rucene reads Lucene's files and its own and sees what Lucene sees.
#[test]
fn rucene_reads_what_lucene_reads() {
    for case in ["nocfs", "cfs", "edge"] {
        let expected = java_reads(case);
        let java_dir = TempDir::new(&format!("simpletext-java-{case}"));
        write_files(&java_dir.0, &java_files(case));
        assert_same_lines(
            &format!("{case}: Lucene's files"),
            &dump(&java_dir.0),
            &expected,
        );

        let rust_dir = TempDir::new(&format!("simpletext-rust-{case}"));
        write_case(case, &rust_dir.0);
        assert_same_lines(
            &format!("{case}: Rucene's files"),
            &dump(&rust_dir.0),
            &expected,
        );
    }
}

fn lucene_jar(artifact: &str) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home).join(format!(
        ".m2/repository/org/apache/lucene/{artifact}/10.5.0/{artifact}-10.5.0.jar"
    ));
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Lucene reads the files Rucene writes and sees what it sees in its own
/// (`java -cp lucene-core:lucene-codecs SimpleTextReadOracle.java <dir>`).
#[test]
fn lucene_reads_what_rucene_writes() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/codecs/simpletext/\
         SimpleTextReadOracle.java",
    );
    let classpath = format!(
        "{}:{}",
        lucene_jar("lucene-core").display(),
        lucene_jar("lucene-codecs").display()
    );
    for case in ["nocfs", "cfs", "edge"] {
        let rust_dir = TempDir::new(&format!("simpletext-forjava-{case}"));
        write_case(case, &rust_dir.0);
        let output = Command::new("java")
            .arg("-ea")
            .arg("-cp")
            .arg(&classpath)
            .arg(&source)
            .arg(&rust_dir.0)
            .output()
            .expect("a JDK is required to run the Lucene oracle");
        assert!(
            output.status.success(),
            "SimpleTextReadOracle failed on Rucene's {case} index:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_same_lines(
            &format!("{case}: Lucene on Rucene's files"),
            &lines,
            &java_reads(case),
        );
    }
}

/// `SimpleTextOracle.lastDigitBumped`.
fn last_digit_bumped(bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let at = b.len() - 2;
    b[at] = b'0' + ((b[at] - b'0' + 1) % 10);
    b
}

/// `SimpleTextOracle.replaceLast`.
fn replace_last(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .rposition(|w| w == from.as_bytes())
        .unwrap_or_else(|| panic!("no {from}"));
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(to.as_bytes());
    out.extend_from_slice(&bytes[at + from.len()..]);
    out
}

/// A corrupted copy of Lucene's `nocfs` index raises the exception Lucene
/// raises, with Lucene's message.
#[test]
fn corrupted_files_raise_lucenes_errors() {
    install_codecs();
    let expected: BTreeMap<String, (String, String)> = rows("simpletext_errors.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), (r[1].clone(), r[2].clone())))
        .collect();
    type Edit = fn(&[u8]) -> Vec<u8>;
    let cases: [(&str, &str, Edit); 6] = [
        ("pst-checksum", "pst", last_digit_bumped),
        ("fld-trailing", "fld", |b| {
            let mut c = b.to_vec();
            c.push(b'x');
            c
        }),
        ("inf-word", "inf", |b| {
            replace_last(b, "checksum ", "chksum   ")
        }),
        ("si-id", "si", |b| replace_last(b, "    id [", "    id [0 ")),
        ("si-version", "si", |b| {
            replace_last(b, "    version 1", "    version x")
        }),
        ("dat-checksum", "dat", last_digit_bumped),
    ];
    let java = java_files("nocfs");
    for (name, ext, edit) in cases {
        let tmp = TempDir::new(&format!("simpletext-corrupt-{name}"));
        write_files(&tmp.0, &java);
        let infos = rucene::index::SegmentInfos::read_latest_commit(open_dir(&tmp.0)).unwrap();
        let segment = infos.info(0).info.name.clone();
        let id =
            rucene::util::string_helper::StringHelper::id_to_string(Some(&infos.info(0).info.id()));
        let file = tmp.0.join(format!("{segment}.{ext}"));
        std::fs::write(&file, edit(&std::fs::read(&file).unwrap())).unwrap();
        let actual = match directory_reader::open(open_dir(&tmp.0)) {
            Ok(_) => ("none".to_string(), "-".to_string()),
            Err(e) => {
                let class = match &e {
                    rucene::error::LuceneError::CorruptIndex(_) => {
                        "org.apache.lucene.index.CorruptIndexException".to_string()
                    }
                    other => format!("rucene::{}", other.java_simple_name()),
                };
                let message = match &e {
                    rucene::error::LuceneError::CorruptIndex(m) => m.clone(),
                    other => other.to_string(),
                };
                (
                    class,
                    message
                        .replace(&tmp.0.to_string_lossy().into_owned(), "$D")
                        .replace(&id, "$ID")
                        .replace('\n', " "),
                )
            }
        };
        assert_eq!(&actual, &expected[name], "{name}");
    }
}

/// `Codec.forName("SimpleText")` resolves once the crate's service files are
/// loaded, and its formats carry Lucene's names.
#[test]
fn codec_for_name_resolves() {
    install_codecs();
    let expected: BTreeMap<String, String> = rows("simpletext_spi.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), r[1].clone()))
        .collect();
    let codec = rucene::codecs::for_name("SimpleText").expect("SimpleText resolves");
    assert_eq!(codec.name(), expected["Codec.forName(SimpleText)"]);
    assert_eq!(codec.postings_format().name(), expected["postingsFormat"]);
    assert_eq!(
        codec.doc_values_format().name(),
        expected["docValuesFormat"]
    );
    assert_eq!(
        codec.knn_vectors_format().name(),
        expected["knnVectorsFormat"]
    );
    // `SimpleTextPostingsFormat` is no service of its own in Lucene either.
    assert_eq!(
        expected["PostingsFormat.forName(SimpleText)"],
        "java.lang.IllegalArgumentException"
    );
    assert!(rucene::codecs::postings_for_name("SimpleText").is_none());
}

/// Escaping round trip of the fixture encoding, so that a fixture decoding
/// defect cannot hide a byte difference.
#[test]
fn fixture_escape_round_trips() {
    let all: Vec<u8> = (0..=255u8).collect();
    assert_eq!(unescape(&escape(&all)), all);
}

/// Rucene reads Lucene's pinned segment info and field infos back to the
/// values Lucene wrote.
#[test]
fn rucene_reads_lucenes_segment_and_field_infos() {
    let tmp = TempDir::new("simpletext-formats-read");
    write_files(&tmp.0, &java_files("formats"));
    let dir = open_dir(&tmp.0);
    let codec = SimpleTextCodec::new();
    let read = codec
        .segment_info_format()
        .read(
            Arc::clone(&dir),
            "_9",
            &SEGMENT_ID,
            &*rucene::store::DEFAULT_IO_CONTEXT,
        )
        .unwrap();
    let expected = format_segment_info(&dir);
    expected.add_file("_9.si".to_string()).unwrap();
    assert_eq!(read.version(), expected.version());
    assert_eq!(read.min_version(), expected.min_version());
    assert_eq!(read.max_doc().unwrap(), 7);
    assert!(read.get_use_compound_file());
    assert!(!read.get_has_blocks());
    assert_eq!(
        read.get_diagnostics().to_std(),
        expected.get_diagnostics().to_std()
    );
    assert_eq!(read.get_attributes(), expected.get_attributes());
    assert_eq!(read.files().unwrap(), expected.files().unwrap());
    assert_eq!(read.id(), SEGMENT_ID);
    assert_eq!(read.index_sort(), expected.index_sort());

    let infos = codec
        .field_infos_format()
        .read(
            dir.as_ref(),
            &read,
            "3",
            &*rucene::store::DEFAULT_IO_CONTEXT,
        )
        .unwrap();
    let expected = format_field_infos();
    let read_all: Vec<&FieldInfo> = infos.iter().collect();
    let expected_all: Vec<&FieldInfo> = expected.iter().collect();
    assert_eq!(read_all, expected_all);
    for (r, e) in read_all.iter().zip(expected_all.iter()) {
        assert_eq!(
            r.attributes().to_std(),
            e.attributes().to_std(),
            "{}",
            r.name
        );
    }
}
