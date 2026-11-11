//! The helpers every oracle-parity test of this crate shares: the fixture
//! decoding, the corpus of `SimpleTextOracle` (`nocfs`, `cfs`, `edge`) written
//! with a given codec, and the dump of `SimpleTextReadOracle`, line for line.
//!
//! The test binary that includes this module defines
//! `fn test_class_loader() -> &'static Arc<ClassLoader>` at its root: the
//! loader whose service files [`install_codecs`] puts on the "class path".

#![allow(dead_code)]

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
use rucene::util::resource_loader::ClassLoader;
use rucene::util::{AttributeSource, BytesRef};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The fixture `name` of the crate that includes this module, or, when it has
/// none of that name, `rucene-codecs`' (the formats `lucene-test-framework`
/// ports from `lucene-codecs`' oracles, such as the blockterms assemblies,
/// read the fixtures `PostingsFormatOracle` wrote for `rucene-codecs`).
pub fn fixture(name: &str) -> String {
    let own = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let path = if Path::new(&own).exists() {
        own
    } else {
        format!(
            "{}/../rucene-codecs/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

pub fn rows(name: &str) -> Vec<Vec<String>> {
    fixture(name)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// Inverse of the oracle's `escape`: `\\` and `\xHH`.
pub fn unescape(s: &str) -> Vec<u8> {
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
pub fn escape(bytes: &[u8]) -> String {
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

pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The files Lucene wrote for `case`, from `<prefix>_files.tsv`: name to
/// bytes.
pub fn java_files(prefix: &str, case: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for row in rows(&format!("{prefix}_files.tsv")) {
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

/// What Lucene read from its own index `case`, from `<prefix>_reads.tsv`.
pub fn java_reads(prefix: &str, case: &str) -> Vec<String> {
    rows(&format!("{prefix}_reads.tsv"))
        .into_iter()
        .filter(|r| r[0] == case)
        .map(|r| String::from_utf8(unescape(&r[1])).unwrap())
        .collect()
}

/// A fresh temporary directory, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(prefix: &str) -> TempDir {
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

/// Puts the test binary's class loader (`crate::test_class_loader`) on the
/// "class path", as Java does at start-up: its codecs, postings formats and
/// kNN-vectors formats become visible to `forName`.
pub fn install_codecs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let loader: &Arc<ClassLoader> = crate::test_class_loader();
        rucene::codecs::reload_codecs(loader).unwrap();
        rucene::codecs::postings::reload_postings_formats(loader).unwrap();
        rucene::codecs::knn_vectors::reload_knn_vectors_format(loader).unwrap();
    });
}

pub fn write_files(dir: &Path, files: &BTreeMap<String, Vec<u8>>) {
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

pub fn read_dir(dir: &Path) -> BTreeMap<String, Vec<u8>> {
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

pub const BATCHES: [(i32, i32); 4] = [(0, 120), (120, 240), (240, 320), (320, 360)];

pub fn id(i: i32) -> String {
    format!("id-{i:04}")
}

pub fn body(i: i32) -> String {
    (0..i % 7 + 1)
        .map(|j| format!("w{}", (i * 31 + j * 17) % 50))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug)]
pub struct Tok {
    term: String,
    start: i32,
    end: i32,
    payload: Option<Vec<u8>>,
}

pub fn tv_tokens(i: i32) -> Vec<Tok> {
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

pub fn float_vector(i: i32) -> [f32; 4] {
    [
        ((i * 3) % 17 - 8) as f32 / 2.0,
        ((i * 5) % 13 - 6) as f32 / 4.0,
        (i % 7) as f32 / 2.0,
        ((i * 11) % 19) as f32 / 8.0,
    ]
}

pub fn byte_vector(i: i32) -> Vec<u8> {
    [(i * 3) % 50, (i * 7) % 60 - 30, i % 20, (i * 13) % 40 - 20]
        .iter()
        .map(|&v| v as i8 as u8)
        .collect()
}

pub fn field_type(f: impl FnOnce(&mut FieldType)) -> FieldType {
    let mut t = FieldType::new();
    f(&mut t);
    t.freeze();
    t
}

pub fn body_type() -> FieldType {
    field_type(|t| {
        t.set_tokenized(true).unwrap();
        t.set_stored(true).unwrap();
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            .unwrap();
    })
}

pub fn tv_type() -> FieldType {
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

pub fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

pub fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

pub fn doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    d.add(Box::new(Field::new("body", body(i), body_type()).unwrap()));
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(ScriptedTokenStream::new(tv_tokens(i))));
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

pub fn new_config(codec: Arc<dyn Codec>) -> IndexWriterConfig {
    install_codecs();
    let config = IndexWriterConfig::new();
    config.set_codec(codec).unwrap();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config
}

pub fn open_dir(path: &Path) -> Arc<dyn Directory> {
    Arc::new(NIOFSDirectory::open(path).unwrap())
}

/// `SimpleTextOracle.writeCorpus`.
pub fn write_corpus(path: &Path, compound: bool, codec: Arc<dyn Codec>) {
    let config = new_config(codec);
    if !compound {
        let policy = TieredMergePolicy::new();
        policy.set_no_cfs_ratio(0.0).unwrap();
        config.set_merge_policy(Some(Arc::new(policy))).unwrap();
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
// Bloom filter files (`.blm`)
// ---------------------------------------------------------------------------

/// A `.blm` file cut into its header and delegate name, its filters
/// (`FieldNumber, FuzzySet`, each with its field number) and its footer.
pub struct Blm {
    pub head: Vec<u8>,
    pub filters: Vec<(i32, Vec<u8>)>,
    pub footer: Vec<u8>,
}

pub fn parse_blm(bytes: &[u8]) -> Blm {
    let header_end = id_offset(bytes) + 16;
    let header_end = header_end + 1 + bytes[header_end] as usize;
    let mut input = rucene::store::ByteArrayDataInput::new(&bytes[header_end..]);
    rucene::store::DataInput::read_string(&mut input).unwrap();
    let num = rucene::store::DataInput::read_int(&mut input).unwrap();
    let head = bytes[..header_end + input.position()].to_vec();
    let mut filters = Vec::new();
    for _ in 0..num {
        let start = header_end + input.position();
        let field = rucene::store::DataInput::read_int(&mut input).unwrap();
        rucene_codecs::bloom::FuzzySet::deserialize(&mut input).unwrap();
        filters.push((field, bytes[start..header_end + input.position()].to_vec()));
    }
    let end = header_end + input.position();
    assert_eq!(bytes.len() - end, 16, "the footer follows the filters");
    Blm {
        head,
        filters,
        footer: bytes[end..].to_vec(),
    }
}

/// Lucene's `.blm` with its filters in the order of Rucene's: Lucene writes
/// them in `HashMap<FieldInfo, FuzzySet>` order, which follows identity
/// hash codes (the `nocfs` and `cfs` fixtures hold segment `_5`'s three
/// filters in two orders). Every filter of one side must be byte for byte a
/// filter of the other; the footer's CRC-32 is recomputed.
pub fn blm_in_rucenes_order(name: &str, java: Vec<u8>, rust: &[u8]) -> Vec<u8> {
    if !name.ends_with(".blm") {
        return java;
    }
    let java_blm = parse_blm(&java);
    let rust_blm = parse_blm(rust);
    let mut reordered = java_blm.head.clone();
    assert_eq!(
        java_blm.filters.len(),
        rust_blm.filters.len(),
        "{name}: filter count"
    );
    for (field, _) in &rust_blm.filters {
        let (_, filter) = java_blm
            .filters
            .iter()
            .find(|(f, _)| f == field)
            .unwrap_or_else(|| panic!("{name}: Lucene wrote no filter for field {field}"));
        reordered.extend_from_slice(filter);
    }
    reordered.extend_from_slice(&java_blm.footer);
    with_fixed_footer(reordered)
}

// ---------------------------------------------------------------------------
// The `shared` case: terms in several fields (`PostingsFormatOracle.writeShared`)
// ---------------------------------------------------------------------------

/// `PostingsFormatOracle.sharedType(IndexOptions)`.
pub fn shared_type(options: IndexOptions) -> FieldType {
    field_type(|t| {
        t.set_tokenized(false).unwrap();
        t.set_omit_norms(true).unwrap();
        t.set_index_options(options).unwrap();
    })
}

/// `PostingsFormatOracle.writeShared(Directory, Codec)`.
pub fn write_shared(path: &Path, codec: Arc<dyn Codec>) {
    let config = new_config(codec);
    let policy = TieredMergePolicy::new();
    policy.set_no_cfs_ratio(0.0).unwrap();
    config.set_merge_policy(Some(Arc::new(policy))).unwrap();
    config.set_use_compound_file(false).unwrap();
    let docs = shared_type(IndexOptions::DOCS);
    let freqs = shared_type(IndexOptions::DOCS_AND_FREQS);
    let positions = shared_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
    let offsets = shared_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    let w = IndexWriter::new(open_dir(path), config).unwrap();
    for batch in 0..3 {
        for i in batch * 100..batch * 100 + 100 {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("s_id", format!("{i:04}"), Store::NO).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sa", format!("t{}", i % 37), docs.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sb", format!("t{}", i % 23), positions.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sc", format!("t{}", i % 11), offsets.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sc", format!("u{}", i % 53), offsets.clone()).unwrap(),
            ));
            d.add(Box::new(
                Field::new("sd", format!("t{}", i % 37), freqs.clone()).unwrap(),
            ));
            if i % 3 == 0 {
                d.add(Box::new(
                    Field::new("se", format!("t{}", i % 7), docs.clone()).unwrap(),
                ));
            }
            w.add_document(&d).unwrap();
        }
        w.commit().unwrap();
        if batch == 1 {
            for i in (0..200).step_by(9) {
                w.delete_documents_by_terms(vec![term("s_id", &format!("{i:04}"))])
                    .unwrap();
            }
        }
    }
    w.force_merge(1).unwrap();
    w.commit().unwrap();
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// The edge documents: SimpleTextOracle.edgeDoc and writeEdge
// ---------------------------------------------------------------------------

pub const EDGE_DOCS: i32 = 12;
pub const INTS: [i32; 5] = [i32::MIN, i32::MAX, 0, -1, 7];
pub const LONGS: [i64; 5] = [i64::MIN, i64::MAX, 0, -1, 1_000_000_007];
pub const FLOATS: [f32; 12] = [
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
pub const DOUBLES: [f64; 12] = [
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
pub const NUMS: [i64; 6] = [i64::MIN, i64::MAX, -1, 0, 42, i64::MIN + 1];
pub const UNIT: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.6, 0.8, 0.0],
    [0.0, -0.6, 0.8],
    [-1.0, 0.0, 0.0],
];

pub fn edge_text(i: i32) -> String {
    match i {
        0 => String::new(),
        1 => "h\u{e9}llo w\u{f6}rld \u{65e5}\u{672c}".to_string(),
        _ => format!(
            "edge{i} common words{}",
            if i % 3 == 0 { " common" } else { "" }
        ),
    }
}

pub fn edge_str(i: i32) -> String {
    match i {
        0 => String::new(),
        1 => "na\u{ef}ve-\u{65e5}\u{672c}-\u{1f600}".to_string(),
        2 => "line\nbreak\\slash".to_string(),
        _ => format!("s{}", i % 4),
    }
}

pub fn edge_stored_str(i: i32) -> String {
    match i {
        0 => String::new(),
        1 => "tab\there\nnew\\line \u{fc}n\u{ef}\u{1f600}".to_string(),
        _ => format!("v{i}"),
    }
}

pub fn edge_bytes(i: i32) -> Vec<u8> {
    match i {
        0 => Vec::new(),
        1 => vec![0x0a, 0x5c, 0xff, 0x00],
        _ => vec![i as u8, 0x0a],
    }
}

pub fn euclidean_vector(i: i32) -> [f32; 3] {
    let u = UNIT[(i % 6) as usize];
    [
        u[0] * (i + 1) as f32 / 4.0,
        if u[1] == 0.0 && i == 0 { -0.0 } else { u[1] },
        u[2] + 2.5E-5,
    ]
}

pub fn edge_byte_vector(i: i32, salt: i32) -> Vec<u8> {
    vec![
        (i * 3 - 5 + salt) as i8 as u8,
        (-i) as i8 as u8,
        ((i * i + salt) % 100) as i8 as u8,
    ]
}

pub fn edge_doc(i: i32) -> Document {
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
pub fn write_edge(path: &Path, codec: Arc<dyn Codec>) {
    let config = new_config(codec);
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
// Scripted token stream: SimpleTextOracle.ScriptedTokenStream
// ---------------------------------------------------------------------------

pub struct ScriptedTokenStream {
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
            packed.append_string(Some(&token.term));
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

pub const ADVANCE_TARGETS: [i32; 9] = [0, 1, 5, 9, 17, 33, 65, 129, 257];

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}

pub fn opt_hex(b: Option<&[u8]>) -> String {
    b.map_or_else(|| "-".to_string(), hex)
}

/// `Integer.toHexString(Float.floatToRawIntBits(v))`.
pub fn fbits(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

pub fn enum_name<T: std::fmt::Debug>(v: T) -> String {
    format!("{v:?}")
}

pub fn dump(path: &Path) -> Vec<String> {
    install_codecs();
    let reader: Arc<dyn IndexReader> = directory_reader::open(open_dir(path)).unwrap();
    let mut out = vec![format!(
        "reader maxDoc={} numDocs={}",
        reader.max_doc(),
        reader.num_docs()
    )];
    for ctx in Arc::clone(&reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        dump_leaf(ctx.ord(), leaf.as_ref(), &mut out).unwrap();
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

/// The `NullPointerException` Java throws where the dump dereferences a
/// `null` the Rust signature returns as `None`.
pub fn null_value() -> rucene::error::LuceneError {
    rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
        class: "java.lang.NullPointerException",
        message: None,
        cause: None,
    })
}

pub fn positions(pe: &mut dyn PostingsEnum, sb: &mut String) -> rucene::error::Result<()> {
    let freq = pe.freq()?;
    sb.push_str(&format!("f={freq}"));
    for _ in 0..freq {
        let pos = pe.next_position()?;
        sb.push_str(&format!(
            " [{pos},{},{},{}]",
            pe.start_offset(),
            pe.end_offset(),
            opt_hex(pe.get_payload()?)
        ));
    }
    Ok(())
}

pub fn dump_leaf(
    ord: i32,
    leaf: &dyn LeafReader,
    out: &mut Vec<String>,
) -> rucene::error::Result<()> {
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
            dump_terms(leaf, fi, out)?;
        }
    }
    for fi in &fis {
        if fi.has_norms() {
            let mut sb = format!(" norms {}", fi.name);
            match leaf.get_norm_values(&fi.name)? {
                None => sb.push_str(" null"),
                Some(mut norms) => {
                    let mut d = norms.next_doc()?;
                    while d != NO_MORE_DOCS {
                        sb.push_str(&format!(" {d}={}", norms.long_value()?));
                        d = norms.next_doc()?;
                    }
                }
            }
            out.push(sb);
        }
    }
    let stored = leaf.stored_fields()?;
    for doc in 0..leaf.max_doc() {
        let mut visitor = DumpVisitor {
            sb: format!(" stored doc={doc}"),
        };
        stored.document_with_visitor(doc, &mut visitor)?;
        out.push(visitor.sb);
    }
    let tvs = leaf.term_vectors()?;
    for doc in 0..leaf.max_doc() {
        if let Some(fields) = tvs.get(doc)? {
            out.push(format!(" tv doc={doc} size={}", fields.size()));
            let names: Vec<String> = fields.iterator().collect();
            for field in names {
                let terms = fields.terms(&field)?.ok_or_else(null_value)?;
                out.push(format!(
                    "  tvf {field} size={} sumTTF={} sumDF={} docCount={} pos={} off={} pay={}",
                    terms.size()?,
                    terms.sum_total_term_freq()?,
                    terms.sum_doc_freq()?,
                    terms.doc_count()?,
                    terms.has_positions(),
                    terms.has_offsets(),
                    terms.has_payloads()
                ));
                let mut te = terms.iterator()?;
                while let Some(t) = te.next()? {
                    let mut sb = format!("   t {} ttf={} ", hex(t.slice()), te.total_term_freq()?);
                    let mut pe = te
                        .postings(None, POSTINGS_ENUM_ALL)?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                    pe.next_doc()?;
                    positions(pe.as_mut(), &mut sb)?;
                    out.push(sb);
                }
            }
        }
    }
    for fi in &fis {
        dump_doc_values(leaf, fi, out)?;
    }
    for fi in &fis {
        if fi.get_point_dimension_count() > 0 {
            dump_points(leaf, fi, out)?;
        }
    }
    for fi in &fis {
        if fi.get_vector_dimension() > 0 {
            dump_vectors(leaf, fi, out)?;
        }
    }
    Ok(())
}

pub fn dump_terms(
    leaf: &dyn LeafReader,
    fi: &FieldInfo,
    out: &mut Vec<String>,
) -> rucene::error::Result<()> {
    let Some(terms) = leaf.terms(&fi.name)? else {
        out.push(format!(" terms {} null", fi.name));
        return Ok(());
    };
    out.push(format!(
        " terms {} size={} sumTTF={} sumDF={} docCount={} freqs={} pos={} off={} pay={}",
        fi.name,
        terms.size()?,
        terms.sum_total_term_freq()?,
        terms.sum_doc_freq()?,
        terms.doc_count()?,
        terms.has_freqs(),
        terms.has_positions(),
        terms.has_offsets(),
        terms.has_payloads()
    ));
    let mut te = terms.iterator()?;
    let mut all = Vec::new();
    while let Some(t) = te.next()? {
        all.push(t.slice().to_vec());
        out.push(format!(
            "  t {} df={} ttf={}",
            hex(t.slice()),
            te.doc_freq()?,
            te.total_term_freq()?
        ));
        let mut pe = te
            .postings(None, POSTINGS_ENUM_ALL)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let mut d = pe.next_doc()?;
        while d != NO_MORE_DOCS {
            let mut sb = format!("   d={d} ");
            positions(pe.as_mut(), &mut sb)?;
            out.push(sb);
            d = pe.next_doc()?;
        }
        let mut docs = te
            .postings(None, POSTINGS_ENUM_NONE)?
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        let mut sb = "   docs".to_string();
        let mut d = docs.next_doc()?;
        while d != NO_MORE_DOCS {
            sb.push_str(&format!(" {d}:{}", docs.freq()?));
            d = docs.next_doc()?;
        }
        out.push(sb);
        if te.doc_freq()? > 8 {
            let mut adv = te
                .postings(None, POSTINGS_ENUM_FREQS)?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            let mut a = "   adv".to_string();
            let mut doc = -1;
            for target in ADVANCE_TARGETS {
                if target > doc {
                    doc = adv.advance(target)?;
                    a.push_str(&format!(" {target}->{doc}"));
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    a.push_str(&format!(":{}", adv.freq()?));
                }
            }
            out.push(a);
            let mut ie = te.impacts(POSTINGS_ENUM_FREQS)?;
            let mut im = "   imp".to_string();
            for target in ADVANCE_TARGETS {
                ie.advance_shallow(target)?;
                let impacts = ie.get_impacts()?;
                im.push_str(&format!(" @{target} levels={}", impacts.num_levels()));
                for level in 0..impacts.num_levels() {
                    im.push_str(&format!(" L{level} upTo={}", impacts.doc_id_up_to(level)));
                    let buf = impacts.get_impacts(level);
                    for i in 0..buf.size {
                        im.push_str(&format!(" {}/{}", buf.freqs[i], buf.norms[i]));
                    }
                }
                if ie.doc_id() < target {
                    ie.advance(target)?;
                }
                if ie.doc_id() == NO_MORE_DOCS {
                    break;
                }
            }
            out.push(im);
            // The `Impacts` `get_impacts` returns is a view of the enum: one
            // taken before the first `advance_shallow` reports the state each
            // later one leaves.
            let mut held = te.impacts(POSTINGS_ENUM_FREQS)?;
            let view = held.get_impacts()?;
            let mut lv = "   live".to_string();
            for target in ADVANCE_TARGETS {
                held.advance_shallow(target)?;
                lv.push_str(&format!(" @{target} levels={}", view.num_levels()));
                for level in 0..view.num_levels() {
                    lv.push_str(&format!(" L{level} upTo={}", view.doc_id_up_to(level)));
                    let buf = view.get_impacts(level);
                    for i in 0..buf.size {
                        lv.push_str(&format!(" {}/{}", buf.freqs[i], buf.norms[i]));
                    }
                }
                if held.doc_id() < target {
                    held.advance(target)?;
                }
                if held.doc_id() == NO_MORE_DOCS {
                    break;
                }
            }
            out.push(lv);
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
    let mut probe = terms.iterator()?;
    let mut sb = "  seek".to_string();
    for target in &targets {
        let status = probe.seek_ceil(&BytesRef::new(target.clone()))?;
        sb.push_str(&format!(" {}:{}", hex(target), enum_name(status)));
        if status != SeekStatus::END {
            sb.push_str(&format!(
                ":{}:{}",
                hex(probe
                    .term()?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?
                    .slice()),
                probe.doc_freq()?
            ));
        }
    }
    out.push(sb);
    let mut exact = terms.iterator()?;
    let mut ex = "  exact".to_string();
    for target in &targets {
        let found = exact.seek_exact(&BytesRef::new(target.clone()))?;
        ex.push_str(&format!(" {found}"));
        if found {
            ex.push_str(&format!(":{}", exact.total_term_freq()?));
        }
    }
    out.push(ex);
    Ok(())
}

pub struct DumpVisitor {
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

pub fn dump_doc_values(
    leaf: &dyn LeafReader,
    fi: &FieldInfo,
    out: &mut Vec<String>,
) -> rucene::error::Result<()> {
    let name = &fi.name;
    match fi.get_doc_values_type() {
        DocValuesType::NONE => {}
        DocValuesType::NUMERIC => {
            let mut v = leaf.get_numeric_doc_values(name)?.ok_or_else(null_value)?;
            let mut sb = format!(" dv {name}");
            let mut d = v.next_doc()?;
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}={}", v.long_value()?));
                d = v.next_doc()?;
            }
            out.push(sb);
        }
        DocValuesType::BINARY => {
            let mut v = leaf.get_binary_doc_values(name)?.ok_or_else(null_value)?;
            let mut sb = format!(" dv {name}");
            let mut d = v.next_doc()?;
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}={}", hex(v.binary_value()?.slice())));
                d = v.next_doc()?;
            }
            out.push(sb);
        }
        DocValuesType::SORTED => {
            let mut v = leaf.get_sorted_doc_values(name)?.ok_or_else(null_value)?;
            let count = v.get_value_count()?;
            let mut sb = format!(" dv {name} count={count}");
            for ord in 0..count {
                sb.push_str(&format!(" o{ord}={}", hex(v.lookup_ord(ord)?.slice())));
            }
            let mut d = v.next_doc()?;
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}={}", v.ord_value()?));
                d = v.next_doc()?;
            }
            out.push(sb);
        }
        DocValuesType::SORTED_NUMERIC => {
            let mut v = leaf
                .get_sorted_numeric_doc_values(name)?
                .ok_or_else(null_value)?;
            let mut sb = format!(" dv {name}");
            let mut d = v.next_doc()?;
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}="));
                for i in 0..v.doc_value_count()? {
                    if i > 0 {
                        sb.push(',');
                    }
                    sb.push_str(&v.next_value()?.to_string());
                }
                d = v.next_doc()?;
            }
            out.push(sb);
        }
        DocValuesType::SORTED_SET => {
            let mut v = leaf
                .get_sorted_set_doc_values(name)?
                .ok_or_else(null_value)?;
            let count = v.get_value_count()?;
            let mut sb = format!(" dv {name} count={count}");
            for ord in 0..count {
                sb.push_str(&format!(" o{ord}={}", hex(v.lookup_ord(ord)?.slice())));
            }
            let mut d = v.next_doc()?;
            while d != NO_MORE_DOCS {
                sb.push_str(&format!(" {d}="));
                for i in 0..v.doc_value_count()? {
                    if i > 0 {
                        sb.push(',');
                    }
                    sb.push_str(&v.next_ord()?.to_string());
                }
                d = v.next_doc()?;
            }
            out.push(sb);
        }
    }
    if fi.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE {
        let mut s = leaf.get_doc_values_skipper(name)?.ok_or_else(null_value)?;
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
            s.advance(target)?;
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
    Ok(())
}

pub struct ValuesVisitor<'a> {
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

pub struct IdsVisitor<'a> {
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

pub fn dump_points(
    leaf: &dyn LeafReader,
    fi: &FieldInfo,
    out: &mut Vec<String>,
) -> rucene::error::Result<()> {
    let Some(pv) = leaf.get_point_values(&fi.name)? else {
        out.push(format!(" points {} null", fi.name));
        return Ok(());
    };
    out.push(format!(
        " points {} size={} docCount={} dims={},{},{} min={} max={} treeSize={}",
        fi.name,
        pv.size()?,
        pv.doc_count()?,
        pv.num_dimensions()?,
        pv.num_index_dimensions()?,
        pv.bytes_per_dimension()?,
        hex(&pv.min_packed_value()?.ok_or_else(null_value)?),
        hex(&pv.max_packed_value()?.ok_or_else(null_value)?),
        pv.point_tree()?.size()?
    ));
    let mut sb = "  pv".to_string();
    pv.point_tree()?
        .visit_doc_values(&mut ValuesVisitor { sb: &mut sb })?;
    out.push(sb);
    let mut ids = "  pids".to_string();
    pv.point_tree()?
        .visit_doc_ids(&mut IdsVisitor { sb: &mut ids })?;
    out.push(ids);
    let mut tree = "  tree".to_string();
    let mut root = pv.point_tree()?;
    walk(root.as_mut(), &mut tree)?;
    out.push(tree);
    out.push(format!("  clone size={}", root.clone_tree()?.size()?));
    Ok(())
}

/// `SimpleTextReadOracle.walk`: each node's bounds and size, then its
/// children, in pre-order.
pub fn walk(t: &mut dyn rucene::index::PointTree, sb: &mut String) -> rucene::error::Result<()> {
    sb.push_str(&format!(
        " [{},{},{}",
        hex(t.min_packed_value()?),
        hex(t.max_packed_value()?),
        t.size()?
    ));
    if t.move_to_child()? {
        loop {
            walk(t, sb)?;
            if !t.move_to_sibling()? {
                break;
            }
        }
        t.move_to_parent()?;
    }
    sb.push(']');
    Ok(())
}

pub fn dump_vectors(
    leaf: &dyn LeafReader,
    fi: &FieldInfo,
    out: &mut Vec<String>,
) -> rucene::error::Result<()> {
    let dim = fi.get_vector_dimension();
    let mut collector = TopKnnCollector::new(3, i64::from(i32::MAX));
    let mut accept_docs = BitsAcceptDocs::from_live_docs(leaf.get_live_docs(), leaf.max_doc())?;
    let mut sb = format!(" vec {}", fi.name);
    if fi.get_vector_encoding() == VectorEncoding::FLOAT32 {
        let Some(values) = leaf.get_float_vector_values(&fi.name)? else {
            out.push(format!(" vec {} null", fi.name));
            return Ok(());
        };
        sb.push_str(&format!(
            " size={} dim={}",
            values.size(),
            values.dimension()
        ));
        let mut it = values.iterator()?;
        let mut d = it.next_doc()?;
        while d != NO_MORE_DOCS {
            sb.push_str(&format!(" {d}:"));
            let v = values.vector_value(it.index())?;
            let parts: Vec<String> = v.iter().map(|x| fbits(*x)).collect();
            sb.push_str(&parts.join(","));
            d = it.next_doc()?;
        }
        let target: Vec<f32> = (0..dim).map(|i| (i + 1) as f32 / 4.0).collect();
        leaf.search_nearest_vectors(&fi.name, &target, &mut collector, &mut accept_docs)?;
    } else {
        let Some(values) = leaf.get_byte_vector_values(&fi.name)? else {
            out.push(format!(" vec {} null", fi.name));
            return Ok(());
        };
        sb.push_str(&format!(
            " size={} dim={}",
            values.size(),
            values.dimension()
        ));
        let mut it = values.iterator()?;
        let mut d = it.next_doc()?;
        while d != NO_MORE_DOCS {
            let v = values.vector_value(it.index())?;
            sb.push_str(&format!(" {d}:{}", hex(&v)));
            d = it.next_doc()?;
        }
        let target: Vec<u8> = (0..dim).map(|i| (i * 3 - 2) as i8 as u8).collect();
        leaf.search_nearest_vectors_byte(&fi.name, &target, &mut collector, &mut accept_docs)?;
    }
    out.push(sb);
    let visited = collector.visited_count();
    let top = collector.top_docs();
    let mut knn = format!("  knn visited={visited}");
    for sd in &top.score_docs {
        knn.push_str(&format!(" {}:{}", sd.doc, fbits(sd.score)));
    }
    out.push(knn);
    Ok(())
}

// ---------------------------------------------------------------------------
// Comparisons
// ---------------------------------------------------------------------------

/// The diagnostics whose values depend on the environment that wrote the
/// segment rather than on the segment.
pub fn assert_same_lines(what: &str, actual: &[String], expected: &[String]) {
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "{what}: line {} differs", i + 1);
    }
    assert_eq!(actual.len(), expected.len(), "{what}: line count");
}

/// `lucene-test-framework` 10.5.0 and the jars its service providers load
/// (`AssertingCodec` needs randomizedtesting and JUnit), as `extra_jars`.
pub const TEST_FRAMEWORK_JARS: &[&str] = &[
    "org/apache/lucene/lucene-test-framework/10.5.0/lucene-test-framework-10.5.0.jar",
    "com/carrotsearch/randomizedtesting/randomizedtesting-runner/2.8.1/randomizedtesting-runner-2.8.1.jar",
    "junit/junit/4.13.1/junit-4.13.1.jar",
];

/// A jar of the local Maven repository (`~/.m2/repository/<path>`).
pub fn m2_jar(path: &str) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home).join(".m2/repository").join(path);
    assert!(jar.is_file(), "the jar is required: {}", jar.display());
    jar
}

pub fn lucene_jar(artifact: &str) -> PathBuf {
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

// ---------------------------------------------------------------------------
// The generic format oracle: PostingsFormatOracle
// ---------------------------------------------------------------------------

/// `PostingsFormatOracle.codec("postings", format)`: `Lucene104Codec` whose
/// per-field postings format is `format` for every field but `e_id`.
pub fn per_field_postings_codec(format: Arc<dyn rucene::codecs::PostingsFormat>) -> Arc<dyn Codec> {
    let default: Arc<dyn rucene::codecs::PostingsFormat> =
        Arc::new(rucene::codecs::Lucene104PostingsFormat::new());
    let postings = rucene::codecs::PerFieldPostingsFormat::new(move |field| {
        if field == "e_id" {
            Arc::clone(&default)
        } else {
            Arc::clone(&format)
        }
    });
    Arc::new(OracleCodec {
        inner: rucene::codecs::lucene104::Lucene104Codec::new(),
        postings: Some(postings),
        knn: None,
    })
}

/// `PostingsFormatOracle.codec("knn", format)`: `Lucene104Codec` whose
/// per-field kNN-vectors format is `format` for the fields `vb` and
/// `e_vb_*`.
pub fn per_field_knn_codec(format: Arc<dyn rucene::codecs::KnnVectorsFormat>) -> Arc<dyn Codec> {
    let default: Arc<dyn rucene::codecs::KnnVectorsFormat> =
        Arc::new(rucene::codecs::Lucene99HnswVectorsFormat::new());
    let knn = rucene::codecs::PerFieldKnnVectorsFormat::new(move |field| {
        if field == "vb" || field.starts_with("e_vb") {
            Arc::clone(&format)
        } else {
            Arc::clone(&default)
        }
    });
    Arc::new(OracleCodec {
        inner: rucene::codecs::lucene104::Lucene104Codec::new(),
        postings: None,
        knn: Some(knn),
    })
}

/// Java's anonymous `Lucene104Codec` subclass: named `Lucene104`, with one of
/// its per-field formats replaced.
#[derive(Debug)]
struct OracleCodec {
    inner: rucene::codecs::lucene104::Lucene104Codec,
    postings: Option<rucene::codecs::PerFieldPostingsFormat>,
    knn: Option<rucene::codecs::PerFieldKnnVectorsFormat>,
}

impl Codec for OracleCodec {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn postings_format(&self) -> &dyn rucene::codecs::PostingsFormat {
        match &self.postings {
            Some(p) => p,
            None => self.inner.postings_format(),
        }
    }
    fn doc_values_format(&self) -> &dyn rucene::codecs::DocValuesFormat {
        self.inner.doc_values_format()
    }
    fn stored_fields_format(&self) -> &dyn rucene::codecs::StoredFieldsFormat {
        self.inner.stored_fields_format()
    }
    fn term_vectors_format(&self) -> &dyn rucene::codecs::TermVectorsFormat {
        self.inner.term_vectors_format()
    }
    fn field_infos_format(&self) -> &dyn rucene::codecs::FieldInfosFormat {
        self.inner.field_infos_format()
    }
    fn segment_info_format(&self) -> &dyn rucene::codecs::SegmentInfoFormat {
        self.inner.segment_info_format()
    }
    fn norms_format(&self) -> &dyn rucene::codecs::NormsFormat {
        self.inner.norms_format()
    }
    fn live_docs_format(&self) -> &dyn rucene::codecs::LiveDocsFormat {
        self.inner.live_docs_format()
    }
    fn compound_format(&self) -> &dyn rucene::codecs::CompoundFormat {
        self.inner.compound_format()
    }
    fn points_format(&self) -> &dyn rucene::codecs::PointsFormat {
        self.inner.points_format()
    }
    fn knn_vectors_format(&self) -> &dyn rucene::codecs::KnnVectorsFormat {
        match &self.knn {
            Some(k) => k,
            None => self.inner.knn_vectors_format(),
        }
    }
}

/// The offset of the segment id in a file that starts with
/// `CodecUtil.writeIndexHeader` (`PostingsFormatOracle.idOffset`).
pub fn id_offset(bytes: &[u8]) -> usize {
    4 + 1 + bytes[4] as usize + 4
}

/// The files of a Lucene90 compound file: `.cfe`'s entries (name without
/// the segment, offset, length) cut out of `.cfs`.
pub fn compound_entries(cfe: &[u8], cfs: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut input = rucene::store::ByteArrayDataInput::new(cfe);
    let skip = id_offset(cfe) + 16;
    let suffix_len = cfe[skip] as usize;
    input.set_position(skip + 1 + suffix_len);
    use rucene::store::DataInput;
    let n = input.read_v_int().unwrap();
    let mut out = BTreeMap::new();
    for _ in 0..n {
        let name = input.read_string().unwrap();
        let offset = input.read_long().unwrap() as usize;
        let length = input.read_long().unwrap() as usize;
        out.insert(name, cfs[offset..offset + length].to_vec());
    }
    out
}

/// `bytes` with its CRC-32 footer recomputed over everything before it.
pub fn with_fixed_footer(mut bytes: Vec<u8>) -> Vec<u8> {
    let n = bytes.len() - 8;
    let mut crc = rucene::store::BufferedChecksum::new();
    crc.update_bytes(&bytes, 0, n).unwrap();
    let value = crc.get_value() as u64;
    bytes[n..].copy_from_slice(&value.to_be_bytes());
    bytes
}

/// Lucene's bytes of a file with Rucene's segment id in its header and the
/// footer recomputed: the bytes Rucene must write for the same input.
///
/// Every occurrence of Lucene's id is replaced: a format file can hold more
/// than one index header (the postings writer writes its own into the terms
/// dictionary).
pub fn java_bytes_with_rust_id(java: &[u8], rust: &[u8]) -> Vec<u8> {
    let at = id_offset(java);
    let mut bytes = java.to_vec();
    if rust.len() >= at + 16 && java.len() >= at + 16 + 16 {
        let java_id = java[at..at + 16].to_vec();
        let rust_id = &rust[at..at + 16];
        let mut i = 0;
        while i + 16 <= bytes.len() {
            if bytes[i..i + 16] == java_id[..] {
                bytes[i..i + 16].copy_from_slice(rust_id);
                i += 16;
            } else {
                i += 1;
            }
        }
        bytes = with_fixed_footer(bytes);
    }
    bytes
}

/// The index of the first byte where `a` and `b` differ.
pub fn first_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

/// Java's `getClass().getName()` and `String.valueOf(getMessage())` of `e`:
/// the shared mapping of the test framework, with `null` for a `null`
/// message.
pub fn java_error_of(e: &rucene::error::LuceneError) -> (String, String) {
    use rucene_test_framework::analysis::base_token_stream_test_case::java_message;
    (
        rucene_test_framework::util::java_exceptions::java_class_name(e),
        java_message(e).unwrap_or_else(|| "null".to_string()),
    )
}

/// `PostingsFormatOracle`'s masking of the two CRC-32 values of a checksum
/// mismatch: `expected=<hex> actual=<hex>` becomes `expected=$CRC
/// actual=$CRC`.
pub fn mask_crc(message: &str) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(at) = rest.find("expected=") {
        let after = &rest[at + "expected=".len()..];
        let hex1 = after.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        let tail = &after[hex1..];
        if hex1 > 0 && tail.starts_with(" actual=") {
            let after2 = &tail[" actual=".len()..];
            let hex2 = after2.chars().take_while(|c| c.is_ascii_hexdigit()).count();
            if hex2 > 0 {
                out.push_str(&rest[..at]);
                out.push_str("expected=$CRC actual=$CRC");
                rest = &after2[hex2..];
                continue;
            }
        }
        out.push_str(&rest[..at + "expected=".len()]);
        rest = after;
    }
    out.push_str(rest);
    out
}

// A defect of Rucene's core that these tests meet but do not own
// (rmp #334 reports it; the task that owns the core class fixes it):
// `BlockPostingsEnum::get_impacts` (`src/codecs/lucene104/postings_reader.rs`,
// `Ok(Box::new(self.impacts.clone()))`) returns a snapshot, where Lucene's
// `getImpacts()` returns the enum's own `Impacts` object, a view of every
// later `advanceShallow`. The dump's `   live` lines read such a view; every
// format here keeps its postings in `Lucene104PostingsWriter`'s files, so
// those lines differ. [`assert_same_lines_but_live`] compares every other
// line and requires the `live` lines to still differ, so the exclusion fails
// once the core is fixed.

/// [`assert_same_lines`] except the dump's `   live` lines (the core defect
/// above), which must still differ somewhere when `live_must_differ`.
pub fn assert_same_lines_but_live(
    what: &str,
    actual: &[String],
    expected: &[String],
    live_must_differ: bool,
) {
    assert_eq!(actual.len(), expected.len(), "{what}: line count");
    let mut live_differences = 0;
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if e.starts_with("   live") && a.starts_with("   live") {
            if a != e {
                live_differences += 1;
            }
            continue;
        }
        assert_eq!(a, e, "{what}: line {} differs", i + 1);
    }
    if live_must_differ {
        assert!(
            live_differences > 0,
            "{what}: every `live` line matches: the core's live impacts are fixed, remove the exclusion"
        );
    }
}

/// `PostingsFormatOracle.REGEXPS`.
pub const REGEXPS: [&str; 7] = ["w1.*", "w[2-4]", ".*7", "t.", "id-00[0-3].", "e.*", ".*"];

fn hex_or_null(b: Option<&BytesRef>) -> String {
    b.map_or_else(|| "null".to_string(), |b| hex(b.slice()))
}

/// `PostingsFormatOracle.error`: `!` and the exception's simple name.
fn error_name(e: &rucene::error::LuceneError) -> String {
    let (class, _) = java_error_of(e);
    match class.rsplit_once('.') {
        Some((_, simple)) => format!("!{simple}"),
        None => format!("!{}", e.java_simple_name()),
    }
}

fn first_doc(te: &mut dyn rucene::index::TermsEnum) -> rucene::error::Result<i32> {
    te.postings(None, POSTINGS_ENUM_NONE)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?
        .next_doc()
}

/// `PostingsFormatOracle.enumTrace`, line for line.
pub fn enum_trace(path: &Path) -> Vec<String> {
    enum_trace_with(path, None, &mut 0)
}

/// [`enum_trace`], with an intersection that panics with `known_panic` (a
/// core `debug_assert!` that Java runs as an `assert`, raising
/// `AssertionError` under `-ea`) listed as Java lists that error,
/// `!AssertionError`; `panics` counts them.
pub fn enum_trace_with(path: &Path, known_panic: Option<&str>, panics: &mut usize) -> Vec<String> {
    use rucene::error::Result;
    install_codecs();
    let reader: Arc<dyn IndexReader> = directory_reader::open(open_dir(path)).unwrap();
    let mut out = Vec::new();
    for ctx in Arc::clone(&reader).leaves().unwrap() {
        out.push(format!("leaf {}", ctx.ord()));
        let leaf = ctx.leaf_reader();
        let fis: Vec<FieldInfo> = leaf.get_field_infos().iter().cloned().collect();
        for fi in &fis {
            // e_id keeps Lucene104PostingsFormat: its enum is the core's.
            if fi.name == "e_id" {
                continue;
            }
            let Some(terms) = leaf.terms(&fi.name).unwrap() else {
                continue;
            };
            let size = terms.size().unwrap();
            out.push(format!(" field {} size={size}", fi.name));
            let mut sb = "  ord".to_string();
            let ords = (|| -> Result<()> {
                let mut te = terms.iterator()?;
                te.next()?;
                let first = te.ord();
                sb.push_str(" first=");
                sb.push_str(&first?.to_string());
                for ord in [0, size / 3, size / 2, size - 1] {
                    te.seek_ord(ord)?;
                    // Appended piece by piece, as Java's `StringBuilder`
                    // chain leaves what precedes a call that throws.
                    sb.push_str(&format!(" {ord}="));
                    sb.push_str(&hex(te
                        .term()?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?
                        .slice()));
                    sb.push('/');
                    sb.push_str(&te.doc_freq()?.to_string());
                    sb.push('/');
                    sb.push_str(&te.total_term_freq()?.to_string());
                    let next = te.next()?;
                    sb.push_str(&format!(" next={}", hex_or_null(next.as_ref())));
                    if next.is_some() {
                        sb.push('@');
                        sb.push_str(&te.ord()?.to_string());
                    }
                }
                Ok(())
            })();
            if let Err(e) = ords {
                sb.push(' ');
                sb.push_str(&error_name(&e));
            }
            out.push(sb);

            let mut kept = Vec::new();
            let mut states = Vec::new();
            let mut te = terms.iterator().unwrap();
            let mut i = 0;
            while let Some(t) = te.next().unwrap() {
                if i % 5 == 0 {
                    kept.push(t.clone());
                    states.push(te.term_state().unwrap());
                }
                i += 1;
            }
            let mut sb = "  state".to_string();
            let mut fresh = terms.iterator().unwrap();
            for k in (0..kept.len()).rev() {
                // Appended piece by piece, as Java's `StringBuilder` chain
                // leaves what precedes a call that throws.
                let step = (|sb: &mut String| -> Result<()> {
                    fresh.seek_term_state(&kept[k], states[k].as_ref())?;
                    sb.push(' ');
                    sb.push_str(&hex(fresh
                        .term()?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?
                        .slice()));
                    sb.push('/');
                    sb.push_str(&fresh.doc_freq()?.to_string());
                    sb.push('/');
                    sb.push_str(&fresh.total_term_freq()?.to_string());
                    sb.push('/');
                    sb.push_str(&first_doc(fresh.as_mut())?.to_string());
                    sb.push_str(" next=");
                    let next = fresh.next()?;
                    sb.push_str(&hex_or_null(next.as_ref()));
                    Ok(())
                })(&mut sb);
                if let Err(e) = step {
                    sb.push(' ');
                    sb.push_str(&error_name(&e));
                }
            }
            out.push(sb);

            let mut it = terms.iterator().unwrap();
            let second = if it.next().unwrap().is_some() {
                it.next().unwrap()
            } else {
                None
            };
            for r in REGEXPS {
                let automaton = rucene::util::automaton::reg_exp::RegExp::new(r)
                    .unwrap()
                    .to_automaton()
                    .unwrap();
                let compiled = rucene::util::automaton::compiled_automaton::CompiledAutomaton::new(
                    automaton, false, false, false,
                )
                .unwrap();
                for start in [None, second.as_ref()] {
                    let mut sb = format!("  intersect {r} from={}", hex_or_null(start));
                    let mut listed_terms = String::new();
                    let mut list = || -> Result<()> {
                        let mut ie = terms.intersect(&compiled, start)?;
                        while let Some(t) = ie.next()? {
                            // Appended piece by piece, as Java's
                            // `StringBuilder` chain leaves what precedes a
                            // call that throws.
                            listed_terms.push(' ');
                            listed_terms.push_str(&hex(t.slice()));
                            listed_terms.push('/');
                            listed_terms.push_str(&ie.doc_freq()?.to_string());
                            listed_terms.push('/');
                            listed_terms.push_str(&ie.total_term_freq()?.to_string());
                            listed_terms.push('/');
                            listed_terms.push_str(&first_doc(ie.as_mut())?.to_string());
                        }
                        Ok(())
                    };
                    let listed = match known_panic {
                        None => list(),
                        Some(known) => {
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(list)) {
                                Ok(listed) => listed,
                                Err(payload) => {
                                    let message = payload
                                        .downcast_ref::<&str>()
                                        .map(|m| m.to_string())
                                        .or_else(|| payload.downcast_ref::<String>().cloned())
                                        .unwrap_or_default();
                                    if !message.contains(known) {
                                        std::panic::resume_unwind(payload);
                                    }
                                    *panics += 1;
                                    Err(rucene::error::LuceneError::from(
                                        rucene::error::LuceneErrorKind::AssertionError(message),
                                    ))
                                }
                            }
                        }
                    };
                    sb.push_str(&listed_terms);
                    if let Err(e) = listed {
                        sb.push(' ');
                        sb.push_str(&error_name(&e));
                    }
                    out.push(sb);
                }
            }
        }
    }
    out
}

/// One format under the generic oracle (`PostingsFormatOracle <prefix>
/// <kind> <format> <extensions>`).
pub struct FormatOracle {
    /// The fixture prefix.
    pub prefix: &'static str,
    /// The format's name, as it appears in its files' names.
    pub format_name: &'static str,
    /// The extensions `PostingsFormatOracle` corrupted.
    pub extensions: &'static [&'static str],
    /// The codec of the corpus.
    pub codec: fn() -> Arc<dyn Codec>,
    /// The extra jars `SimpleTextReadOracle.java` needs to resolve the
    /// format, beyond `lucene-core` and `lucene-codecs`: paths under the
    /// local Maven repository.
    pub extra_jars: &'static [&'static str],
    /// Whether the dump's `live` lines meet the core defect of
    /// [`assert_same_lines_but_live`].
    pub live_impacts_differ: bool,
}

impl FormatOracle {
    pub fn write_case(&self, case: &str, path: &Path) {
        match case {
            "nocfs" => write_corpus(path, false, (self.codec)()),
            "cfs" => write_corpus(path, true, (self.codec)()),
            "edge" => write_edge(path, (self.codec)()),
            _ => unreachable!("{case}"),
        }
    }

    /// Whether `name` is a file of the format under test.
    fn is_format_file(&self, name: &str) -> bool {
        name.contains(&format!("_{}_", self.format_name))
    }

    /// The format's files of a directory listing, the files of compound
    /// files included (named `<cfs>/<entry>`).
    fn format_files(&self, files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for (name, bytes) in files {
            if let Some(segment) = name.strip_suffix(".cfs") {
                let cfe = &files[&format!("{segment}.cfe")];
                for (entry, sub) in compound_entries(cfe, bytes) {
                    if self.is_format_file(&entry) {
                        out.insert(format!("{name}/{entry}"), sub);
                    }
                }
            } else if self.is_format_file(name) {
                out.insert(name.clone(), bytes.clone());
            }
        }
        out
    }

    /// Rucene writes the files Lucene writes (the same listing), and every
    /// file of the format under test byte for byte, once Lucene's segment id
    /// is replaced by Rucene's; returns the number of format files compared.
    pub fn assert_writes_lucenes_bytes(&self) -> usize {
        self.assert_writes_lucenes_bytes_with(|_, java, _| java)
    }

    /// [`assert_writes_lucenes_bytes`](Self::assert_writes_lucenes_bytes)
    /// with Lucene's bytes of each format file passed through `normalize(name,
    /// java_with_rust_id, rust)` before the comparison, for a file whose
    /// bytes Lucene does not determine (an order that follows identity hash
    /// codes).
    pub fn assert_writes_lucenes_bytes_with(
        &self,
        normalize: impl Fn(&str, Vec<u8>, &[u8]) -> Vec<u8>,
    ) -> usize {
        let mut compared = 0;
        for case in ["nocfs", "cfs", "edge"] {
            compared += self.assert_case_writes_lucenes_bytes(
                case,
                |path: &Path| self.write_case(case, path),
                &normalize,
            );
        }
        assert!(compared > 0, "{}: no format file compared", self.prefix);
        compared
    }

    /// One case of
    /// [`assert_writes_lucenes_bytes_with`](Self::assert_writes_lucenes_bytes_with):
    /// `write` writes the case's index with Rucene into the directory it is
    /// given; returns the number of format files compared.
    pub fn assert_case_writes_lucenes_bytes(
        &self,
        case: &str,
        write: impl Fn(&Path),
        normalize: impl Fn(&str, Vec<u8>, &[u8]) -> Vec<u8>,
    ) -> usize {
        let mut compared = 0;
        let tmp = TempDir::new(&format!("{}-write-{case}", self.prefix));
        write(&tmp.0);
        let rust = read_dir(&tmp.0);
        let java = java_files(self.prefix, case);
        let rust_names: Vec<&String> = rust.keys().collect();
        let java_names: Vec<&String> = java.keys().collect();
        assert_eq!(
            rust_names, java_names,
            "{}: {case}: file listing",
            self.prefix
        );
        let rust_files = self.format_files(&rust);
        let java_files = self.format_files(&java);
        assert_eq!(
            rust_files.keys().collect::<Vec<_>>(),
            java_files.keys().collect::<Vec<_>>(),
            "{}: {case}: format files",
            self.prefix
        );
        for (name, java_bytes) in &java_files {
            let rust_bytes = &rust_files[name];
            let expected = normalize(
                name,
                java_bytes_with_rust_id(java_bytes, rust_bytes),
                rust_bytes,
            );
            assert!(
                *rust_bytes == expected,
                "{}: {case}: {name} differs from Lucene's at byte {} (lengths {} and {})",
                self.prefix,
                first_difference(rust_bytes, &expected),
                rust_bytes.len(),
                expected.len()
            );
            compared += 1;
        }
        compared
    }

    /// Rucene reads Lucene's files and its own and sees what Lucene sees.
    pub fn assert_reads_what_lucene_reads(&self) {
        for case in ["nocfs", "cfs", "edge"] {
            self.assert_case_reads_what_lucene_reads(case, |path: &Path| {
                self.write_case(case, path)
            });
        }
    }

    /// One case of
    /// [`assert_reads_what_lucene_reads`](Self::assert_reads_what_lucene_reads),
    /// whose Rucene index `write` writes.
    pub fn assert_case_reads_what_lucene_reads(&self, case: &str, write: impl Fn(&Path)) {
        let expected = java_reads(self.prefix, case);
        assert!(!expected.is_empty(), "{}: {case}: no reads", self.prefix);
        let java_dir = TempDir::new(&format!("{}-java-{case}", self.prefix));
        write_files(&java_dir.0, &java_files(self.prefix, case));
        assert_same_lines_but_live(
            &format!("{}: {case}: Lucene's files", self.prefix),
            &dump(&java_dir.0),
            &expected,
            self.live_impacts_differ,
        );
        let rust_dir = TempDir::new(&format!("{}-rust-{case}", self.prefix));
        write(&rust_dir.0);
        assert_same_lines_but_live(
            &format!("{}: {case}: Rucene's files", self.prefix),
            &dump(&rust_dir.0),
            &expected,
            self.live_impacts_differ,
        );
    }

    /// The terms-enum calls the dump does not make
    /// (`PostingsFormatOracle.enumTrace`) see on Lucene's files and on
    /// Rucene's what Lucene saw on its own.
    pub fn assert_enum_trace_matches(&self) {
        for case in ["nocfs", "cfs", "edge"] {
            self.assert_case_enum_trace_matches(case, |path: &Path| self.write_case(case, path));
        }
    }

    /// One case of
    /// [`assert_enum_trace_matches`](Self::assert_enum_trace_matches), whose
    /// Rucene index `write` writes.
    pub fn assert_case_enum_trace_matches(&self, case: &str, write: impl Fn(&Path)) {
        let expected: Vec<String> = rows(&format!("{}_enum.tsv", self.prefix))
            .into_iter()
            .filter(|r| r[0] == case)
            .map(|r| r[1].clone())
            .collect();
        assert!(
            !expected.is_empty(),
            "{}: {case}: no enum trace",
            self.prefix
        );
        let java_dir = TempDir::new(&format!("{}-enum-java-{case}", self.prefix));
        write_files(&java_dir.0, &java_files(self.prefix, case));
        assert_same_lines(
            &format!("{}: {case}: enum trace of Lucene's files", self.prefix),
            &enum_trace(&java_dir.0),
            &expected,
        );
        let rust_dir = TempDir::new(&format!("{}-enum-rust-{case}", self.prefix));
        write(&rust_dir.0);
        assert_same_lines(
            &format!("{}: {case}: enum trace of Rucene's files", self.prefix),
            &enum_trace(&rust_dir.0),
            &expected,
        );
    }

    /// [`assert_enum_trace_matches`](Self::assert_enum_trace_matches) for a
    /// format whose intersections run a core enum that panics with
    /// `known_panic` where Java raises `AssertionError` (see
    /// [`enum_trace_with`]); the panic must still happen on both sides'
    /// files, so the exclusion fails once the core is fixed.
    pub fn assert_enum_trace_matches_but_core_panic(&self, known_panic: &str) {
        let mut panics = [0usize; 2];
        for case in ["nocfs", "cfs", "edge"] {
            let expected: Vec<String> = rows(&format!("{}_enum.tsv", self.prefix))
                .into_iter()
                .filter(|r| r[0] == case)
                .map(|r| r[1].clone())
                .collect();
            let java_dir = TempDir::new(&format!("{}-enum-java-{case}", self.prefix));
            write_files(&java_dir.0, &java_files(self.prefix, case));
            let rust_dir = TempDir::new(&format!("{}-enum-rust-{case}", self.prefix));
            self.write_case(case, &rust_dir.0);
            for (i, (side, dir)) in [("Lucene's", &java_dir.0), ("Rucene's", &rust_dir.0)]
                .into_iter()
                .enumerate()
            {
                let trace = enum_trace_with(dir, Some(known_panic), &mut panics[i]);
                assert_same_lines(
                    &format!("{}: {case}: enum trace of {side} files", self.prefix),
                    &trace,
                    &expected,
                );
            }
        }
        assert!(
            panics.iter().all(|&p| p > 0),
            "{}: no `{known_panic}` panic on one side ({panics:?}): the core is fixed, remove \
             the exclusion",
            self.prefix
        );
    }

    /// Lucene reads the files Rucene writes and sees what it sees in its own
    /// (`java -cp <jars> SimpleTextReadOracle.java <dir>`).
    pub fn assert_lucene_reads_what_rucene_writes(&self) {
        for case in ["nocfs", "cfs", "edge"] {
            self.assert_case_lucene_reads_what_rucene_writes(case, |path: &Path| {
                self.write_case(case, path)
            });
        }
    }

    /// One case of
    /// [`assert_lucene_reads_what_rucene_writes`](Self::assert_lucene_reads_what_rucene_writes),
    /// whose Rucene index `write` writes.
    pub fn assert_case_lucene_reads_what_rucene_writes(&self, case: &str, write: impl Fn(&Path)) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/codecs/simpletext/\
             SimpleTextReadOracle.java",
        );
        let mut jars = vec![lucene_jar("lucene-core"), lucene_jar("lucene-codecs")];
        jars.extend(self.extra_jars.iter().map(|a| m2_jar(a)));
        let classpath = jars
            .iter()
            .map(|j| j.display().to_string())
            .collect::<Vec<_>>()
            .join(":");
        let rust_dir = TempDir::new(&format!("{}-forjava-{case}", self.prefix));
        write(&rust_dir.0);
        let output = std::process::Command::new("java")
            .arg("-ea")
            .arg("-cp")
            .arg(&classpath)
            .arg(&source)
            .arg(&rust_dir.0)
            .output()
            .expect("a JDK is required to run the Lucene oracle");
        assert!(
            output.status.success(),
            "{}: SimpleTextReadOracle failed on Rucene's {case} index:\n{}",
            self.prefix,
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_same_lines(
            &format!("{}: {case}: Lucene on Rucene's files", self.prefix),
            &lines,
            &java_reads(self.prefix, case),
        );
    }

    /// The corruptions of `PostingsFormatOracle.corrupt` of Lucene's `nocfs`
    /// raise, on open, the exception Lucene raises, with Lucene's message.
    pub fn assert_corrupted_files_raise_lucenes_errors(&self) {
        self.assert_corrupted_files_raise_lucenes_errors_but(&[])
    }

    /// [`assert_corrupted_files_raise_lucenes_errors`](Self::assert_corrupted_files_raise_lucenes_errors)
    /// for every case but the `known` ones, `(label, class, message)`: a
    /// core defect outside the format makes Rucene raise that error instead
    /// of Lucene's, and the comparison requires it to still differ from
    /// Lucene's, so the exclusion fails once the core is fixed.
    pub fn assert_corrupted_files_raise_lucenes_errors_but(&self, known: &[(&str, &str, &str)]) {
        install_codecs();
        let expected: BTreeMap<String, (String, String)> =
            rows(&format!("{}_errors.tsv", self.prefix))
                .into_iter()
                .map(|r| (r[0].clone(), (r[1].clone(), r[2].clone())))
                .collect();
        let java = java_files(self.prefix, "nocfs");
        let mut cases = 0;
        for ext in self.extensions {
            for kind in ["truncate", "id", "checksum"] {
                let label = format!("{ext}-{kind}");
                let tmp = TempDir::new(&format!("{}-corrupt-{label}", self.prefix));
                write_files(&tmp.0, &java);
                let infos =
                    rucene::index::SegmentInfos::read_latest_commit(open_dir(&tmp.0)).unwrap();
                let segment = infos.info(0).info.name.clone();
                let id = rucene::util::string_helper::StringHelper::id_to_string(Some(
                    &infos.info(0).info.id(),
                ));
                let prefix = format!("{segment}_{}_", self.format_name);
                let mut names: Vec<&String> = java
                    .keys()
                    .filter(|n| n.starts_with(&prefix) && n.ends_with(&format!(".{ext}")))
                    .collect();
                names.sort();
                let file = tmp.0.join(names[0]);
                let mut bytes = std::fs::read(&file).unwrap();
                let mut bad_id = None;
                match kind {
                    "truncate" => {
                        bytes.pop();
                    }
                    "id" => {
                        let at = id_offset(&bytes);
                        bytes[at + 15] ^= 0xff;
                        bad_id = Some(rucene::util::string_helper::StringHelper::id_to_string(
                            Some(&bytes[at..at + 16]),
                        ));
                    }
                    _ => {
                        let last = bytes.len() - 1;
                        bytes[last] ^= 1;
                    }
                }
                std::fs::write(&file, &bytes).unwrap();
                let actual = match directory_reader::open(open_dir(&tmp.0)) {
                    Ok(_) => ("none".to_string(), "-".to_string()),
                    Err(e) => {
                        let (class, message) = java_error_of(&e);
                        let mut message = message
                            .replace(&tmp.0.to_string_lossy().into_owned(), "$D")
                            .replace('\n', " ");
                        if let Some(bad) = &bad_id {
                            message = message.replace(bad, "$BADID");
                        }
                        let message = mask_crc(&message).replace(&id, "$ID");
                        (class, message)
                    }
                };
                match known.iter().find(|(l, _, _)| *l == label) {
                    Some((_, class, message)) => {
                        assert_eq!(
                            (actual.0.as_str(), actual.1.as_str()),
                            (*class, *message),
                            "{}: {label}: the known core defect changed",
                            self.prefix
                        );
                        assert_ne!(
                            &actual, &expected[&label],
                            "{}: {label}: the core defect is fixed, remove the exclusion",
                            self.prefix
                        );
                    }
                    None => assert_eq!(&actual, &expected[&label], "{}: {label}", self.prefix),
                }
                cases += 1;
            }
        }
        assert_eq!(cases, expected.len());
    }

    /// The rows of `<prefix>_spi.tsv`.
    pub fn spi(&self) -> BTreeMap<String, String> {
        rows(&format!("{}_spi.tsv", self.prefix))
            .into_iter()
            .map(|r| (r[0].clone(), r[1].clone()))
            .collect()
    }
}
