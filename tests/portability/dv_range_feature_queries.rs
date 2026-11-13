//! Doc-values, range-field and feature queries and selector sorts against
//! Apache Lucene Core 10.5.0 (rmp task #409).
//!
//! The corpora are defined by the same formulas here and in
//! `DvRangeFeatureFixture.java`. Under each root, `main` holds the main corpus
//! (three commits, deletions in the first and the last segment): sorted
//! numeric doc values with one or two values per document as longs, sortable
//! floats and sortable doubles, numeric and sorted-set doc values, int and long
//! points, int/long/float/double range fields, IPv4 and IPv6 inet addresses,
//! two features, a `LongField` timestamp, a text field with norms and a float
//! vector — every field missing from some documents. `sorted` holds a second
//! corpus whose index is sorted on the numeric doc values `ns`. Rucene writes
//! both under `R`, Lucene under `L`.
//!
//! * The query set (`DvRangeFeatureFixture.query`, [`query_report`]) runs on
//!   both roots from both sides: `SortedNumericDocValuesField`
//!   `newSlowRangeQuery`/`newSlowSetQuery`,
//!   `NumericDocValuesField.newSlowRangeQuery`, `SortedSetDocValuesField`
//!   `newSlowRangeQuery`/`newSlowSetQuery`, `IndexOrDocValuesQuery`,
//!   `FieldExistsQuery` over doc values, norms, vectors and points,
//!   `IntPoint`/`LongPoint.newSetQuery`, the four relations of every range
//!   field, `InetAddressPoint` exact/prefix/range/set queries, the
//!   `FeatureField` saturation (explicit and automatic pivot), log, sigmoid
//!   and linear queries, `LongField.newDistanceFeatureQuery` (Lucene 10.5.0
//!   has no `LongPoint.newDistanceFeatureQuery`), the `SortedNumericSortField`
//!   sorts (LONG/INT/FLOAT/DOUBLE, MIN/MAX, both orders, missing first and
//!   last) and `SortedSetSortField` sorts (MIN/MAX/MIDDLE_MIN/MIDDLE_MAX, both
//!   orders, `STRING_FIRST`/`STRING_LAST`), and
//!   `IndexSortSortedNumericDocValuesRangeQuery` on `sorted`. Per hit: doc id,
//!   score bits, sort value (bits for floats and doubles) and id. The four
//!   reports are compared line by line.
//! * Positive control: Rucene writes a perturbed main corpus (document 0
//!   gains the sorted-numeric value 7 and its `num` becomes 100); its report
//!   must differ from Lucene's report on `L` with the same document counts.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::feature_field::FeatureField;
use rucene::document::range_fields::{DoubleRange, FloatRange, IntRange, LongRange};
use rucene::document::{
    Document, Field, FieldType, InetAddressPoint, IntPoint, KnnFloatVectorField, LongField,
    LongPoint, NumericDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store,
    StringField,
};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexOptions, IndexReader, StoredFields, Term, VectorSimilarityFunction};
use rucene::search::{
    FieldDoc, FieldExistsQuery, IndexOrDocValuesQuery, IndexSearcher,
    IndexSortSortedNumericDocValuesRangeQuery, MatchAllDocsQuery, MissingValue, Query, ScoreDoc,
    Sort, SortField, SortFieldType, SortValue, SortedNumericSelectorType, SortedNumericSortField,
    SortedSetSelectorType, SortedSetSortField, TotalHits, TotalHitsRelation,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::{BytesRef, NumericUtils};

// ---------------------------------------------------------------------------
// Corpus: the formulas of DvRangeFeatureFixture
// ---------------------------------------------------------------------------

/// `DvRangeFeatureFixture.BATCHES`.
const BATCHES: [(i32, i32); 3] = [(0, 100), (100, 200), (200, 300)];

/// `DvRangeFeatureFixture.SORTED_BATCHES`.
const SORTED_BATCHES: [(i32, i32); 2] = [(0, 100), (100, 200)];

/// `DvRangeFeatureFixture.WORDS`.
const WORDS: [&str; 20] = [
    "apple", "apply", "ample", "maple", "apricot", "banana", "bandana", "cabana", "cherry",
    "sherry", "berry", "merry", "date", "data", "gate", "grape", "graph", "form", "from", "zebra",
];

/// `DvRangeFeatureFixture.TS_BASE`.
const TS_BASE: i64 = 1_600_000_000_000;

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, bytes(text))
}

/// `DvRangeFeatureFixture.TEXT_TYPE`.
fn text_type() -> FieldType {
    let t = FieldType::new();
    t.set_tokenized(true).unwrap();
    t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    t.freeze();
    t
}

/// `DvRangeFeatureFixture.ipv4(int)`.
fn ipv4(i: i32) -> Vec<u8> {
    vec![10, 0, (i / 100) as u8, ((i * 7) % 256) as u8]
}

/// `DvRangeFeatureFixture.ipv6(int)`: `2001:db8::` plus `i` in the last two
/// bytes.
fn ipv6(i: i32) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    b[0] = 0x20;
    b[1] = 0x01;
    b[2] = 0x0d;
    b[3] = 0xb8;
    b[14] = (i / 256) as u8;
    b[15] = (i % 256) as u8;
    b
}

/// `DvRangeFeatureFixture.inet(byte[])`: `InetAddress.getByAddress`.
fn inet(b: &[u8]) -> std::net::IpAddr {
    InetAddressPoint::decode(b).unwrap()
}

fn sndv(field: &str, value: i64) -> Box<SortedNumericDocValuesField> {
    Box::new(SortedNumericDocValuesField::new(field, value))
}

/// `DvRangeFeatureFixture.doc(int)`; the perturbed corpus gives document 0 a
/// third `sn` value, 7, and the `num` value 100.
fn doc(i: i32, perturbed: bool) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    if i % 7 != 6 {
        d.add(sndv("sn", ((i * 37) % 101 - 50) as i64));
    }
    if i % 3 == 0 {
        d.add(sndv("sn", ((i * 13) % 97) as i64));
    }
    if perturbed && i == 0 {
        d.add(sndv("sn", 7));
    }
    if i % 5 != 4 {
        let f = ((i * 17) % 89) as f32 / 4.0 - 10.0;
        d.add(sndv("snf", NumericUtils::float_to_sortable_int(f) as i64));
    }
    if i % 4 == 0 {
        let f = ((i * 29) % 71) as f32 / 2.0 - 5.0;
        d.add(sndv("snf", NumericUtils::float_to_sortable_int(f) as i64));
    }
    if i % 6 != 5 {
        let v = ((i * 23) % 103) as f64 / 8.0 - 6.0;
        d.add(sndv("snd", NumericUtils::double_to_sortable_long(v)));
    }
    if i % 4 == 1 {
        let v = ((i * 19) % 61) as f64 / 3.0 - 9.0;
        d.add(sndv("snd", NumericUtils::double_to_sortable_long(v)));
    }
    if i % 11 != 10 {
        let num = if perturbed && i == 0 {
            100
        } else {
            ((i * 41) % 211) as i64
        };
        d.add(Box::new(NumericDocValuesField::new("num", num)));
    }
    if i % 9 != 8 {
        d.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            bytes(WORDS[(i % 20) as usize]),
        )));
        d.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            bytes(WORDS[((i * 7 + 3) % 20) as usize]),
        )));
        if i % 5 == 0 {
            d.add(Box::new(SortedSetDocValuesField::new(
                "ss",
                bytes(WORDS[((i * 3 + 1) % 20) as usize]),
            )));
        }
    }
    if i % 10 != 9 {
        d.add(Box::new(IntPoint::new("ip", &[(i * 31) % 500]).unwrap()));
    }
    let lv = ((i * 53) % 1000) as i64;
    d.add(Box::new(LongPoint::new("lv", &[lv]).unwrap()));
    d.add(sndv("lv", lv));
    let ir_min = (i * 7) % 100;
    d.add(Box::new(
        IntRange::new("ir", &[ir_min], &[ir_min + i % 13]).unwrap(),
    ));
    let lr_min = ((i * 11) % 1000 - 500) as i64;
    d.add(Box::new(
        LongRange::new("lr", &[lr_min], &[lr_min + ((i % 17) * 3) as i64]).unwrap(),
    ));
    let fr_min = (i % 50) as f32 / 2.0;
    d.add(Box::new(
        FloatRange::new("fr", &[fr_min], &[fr_min + (i % 7) as f32 / 4.0]).unwrap(),
    ));
    let a = ((i * 3) % 40) as f64;
    let b = ((i * 7) % 30) as f64;
    d.add(Box::new(
        DoubleRange::new(
            "dr",
            &[a, b],
            &[a + (i % 5 + 1) as f64, b + (i % 4) as f64 + 0.5],
        )
        .unwrap(),
    ));
    d.add(Box::new(InetAddressPoint::new("inet", inet(&ipv4(i)))));
    if i % 10 == 0 {
        d.add(Box::new(InetAddressPoint::new("inet", inet(&ipv6(i)))));
    }
    if i % 8 != 7 {
        d.add(Box::new(
            FeatureField::new("features", "pagerank", 1.0 + ((i * 13) % 50) as f32 / 2.0).unwrap(),
        ));
    }
    if i % 2 == 0 {
        d.add(Box::new(
            FeatureField::new("features", "recency", 0.5 + (i % 30) as f32).unwrap(),
        ));
    }
    if i % 12 != 11 {
        let ts = TS_BASE + ((i as i64 * 7919) % 100_000) * 1000;
        d.add(Box::new(LongField::new("ts", ts, Store::NO)));
    }
    if i % 4 != 3 {
        d.add(Box::new(
            Field::new(
                "body",
                format!("w{} w{}", i % 10, (i * 3) % 10),
                text_type(),
            )
            .unwrap(),
        ));
    }
    if i % 3 != 2 {
        d.add(Box::new(
            KnnFloatVectorField::new(
                "vec",
                &[(i % 10) as f32, (i % 7) as f32 + 1.0],
                VectorSimilarityFunction::EUCLIDEAN,
            )
            .unwrap(),
        ));
    }
    d
}

/// `DvRangeFeatureFixture.sortedDoc(int)`.
fn sorted_doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    d.add(Box::new(NumericDocValuesField::new(
        "ns",
        ((i * 41) % 211) as i64,
    )));
    d
}

/// `DvRangeFeatureFixture.config()`.
fn config() -> IndexWriterConfig {
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config
}

fn open_dir(path: &Path) -> Arc<dyn Directory> {
    std::fs::create_dir_all(path).unwrap();
    Arc::new(FSDirectory::open(path).unwrap())
}

/// `DvRangeFeatureFixture.write(String)`: the same operations, in order.
fn write_all(root: &Path, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let w = IndexWriter::new(open_dir(&root.join("main")), config()).unwrap();
    for (k, (from, to)) in BATCHES.iter().enumerate() {
        for i in *from..*to {
            w.add_document(&doc(i, perturbed)).unwrap();
        }
        if k == 0 {
            w.delete_documents_by_terms(vec![term("id", &id(7))])
                .unwrap();
        } else if k == 2 {
            w.delete_documents_by_terms(vec![term("id", &id(250))])
                .unwrap();
        }
        w.commit().unwrap();
    }
    w.close().unwrap();

    let sorted_config = config();
    sorted_config
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("ns".to_string()),
                SortFieldType::Long,
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
    let w = IndexWriter::new(open_dir(&root.join("sorted")), sorted_config).unwrap();
    for (from, to) in SORTED_BATCHES {
        for i in from..to {
            w.add_document(&sorted_doc(i)).unwrap();
        }
        w.commit().unwrap();
    }
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Query set: the output of DvRangeFeatureFixture.query, line for line
// ---------------------------------------------------------------------------

/// `DvRangeFeatureFixture.TOP_N`.
const TOP_N: i32 = 20;

/// `DvRangeFeatureFixture.TOP_SORT`.
const TOP_SORT: i32 = 40;

/// `Float.floatToIntBits`, which collapses every NaN to `0x7fc00000`.
fn bits(f: f32) -> String {
    let raw = if f.is_nan() { 0x7fc0_0000 } else { f.to_bits() };
    format!("{raw:08x}")
}

/// `Double.doubleToLongBits`, which collapses every NaN to
/// `0x7ff8000000000000`.
fn bits64(d: f64) -> String {
    let raw = if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits()
    };
    format!("{raw:016x}")
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

fn arc<Q: Query + 'static>(q: Q) -> Arc<dyn Query> {
    Arc::new(q)
}

/// `DvRangeFeatureFixture.queries()`, in the same order.
fn queries() -> Vec<(String, Arc<dyn Query>)> {
    let mut q: Vec<(String, Arc<dyn Query>)> = vec![
        (
            "sn slowRange [-10,20]".into(),
            SortedNumericDocValuesField::new_slow_range_query("sn", -10, 20),
        ),
        (
            "sn slowSet {0,5,13,40,-3}".into(),
            SortedNumericDocValuesField::new_slow_set_query("sn", &[0, 5, 13, 40, -3]),
        ),
        (
            "num slowRange [50,150]".into(),
            NumericDocValuesField::new_slow_range_query("num", 50, 150),
        ),
        (
            "ss slowRange [banana,date)".into(),
            SortedSetDocValuesField::new_slow_range_query(
                "ss",
                Some(bytes("banana")),
                Some(bytes("date")),
                true,
                false,
            ),
        ),
        (
            "ss slowRange [*,cherry]".into(),
            SortedSetDocValuesField::new_slow_range_query(
                "ss",
                None,
                Some(bytes("cherry")),
                true,
                true,
            ),
        ),
        (
            "ss slowSet {apple,grape,zebra,nothere}".into(),
            SortedSetDocValuesField::new_slow_set_query(
                "ss",
                vec![
                    bytes("apple"),
                    bytes("grape"),
                    bytes("zebra"),
                    bytes("nothere"),
                ],
            )
            .unwrap(),
        ),
        (
            "lv indexOrDocValues [100,300]".into(),
            arc(IndexOrDocValuesQuery::new(
                LongPoint::new_range_query("lv", &[100], &[300]).unwrap(),
                SortedNumericDocValuesField::new_slow_range_query("lv", 100, 300),
            )),
        ),
    ];
    for field in ["num", "sn", "ss", "body", "vec", "ts", "lv"] {
        q.push((format!("exists {field}"), arc(FieldExistsQuery::new(field))));
    }
    q.extend([
        (
            "ip set {3,31,62,93,499,1000}".to_string(),
            IntPoint::new_set_query("ip", &[3, 31, 62, 93, 499, 1000]).unwrap(),
        ),
        (
            "lv set {0,53,106,500,999}".to_string(),
            LongPoint::new_set_query("lv", &[0, 53, 106, 500, 999]).unwrap(),
        ),
        (
            "ir intersects [10,20]".to_string(),
            arc(IntRange::new_intersects_query("ir", &[10], &[20]).unwrap()),
        ),
        (
            "ir contains [40,41]".to_string(),
            arc(IntRange::new_contains_query("ir", &[40], &[41]).unwrap()),
        ),
        (
            "ir within [0,60]".to_string(),
            arc(IntRange::new_within_query("ir", &[0], &[60]).unwrap()),
        ),
        (
            "ir crosses [25,35]".to_string(),
            arc(IntRange::new_crosses_query("ir", &[25], &[35]).unwrap()),
        ),
        (
            "lr intersects [-100,100]".to_string(),
            arc(LongRange::new_intersects_query("lr", &[-100], &[100]).unwrap()),
        ),
        (
            "lr contains [0,1]".to_string(),
            arc(LongRange::new_contains_query("lr", &[0], &[1]).unwrap()),
        ),
        (
            "lr within [-500,0]".to_string(),
            arc(LongRange::new_within_query("lr", &[-500], &[0]).unwrap()),
        ),
        (
            "lr crosses [200,260]".to_string(),
            arc(LongRange::new_crosses_query("lr", &[200], &[260]).unwrap()),
        ),
        (
            "fr intersects [5,6]".to_string(),
            arc(FloatRange::new_intersects_query("fr", &[5.0], &[6.0]).unwrap()),
        ),
        (
            "fr contains [10.1,10.2]".to_string(),
            arc(FloatRange::new_contains_query("fr", &[10.1], &[10.2]).unwrap()),
        ),
        (
            "fr within [0,12]".to_string(),
            arc(FloatRange::new_within_query("fr", &[0.0], &[12.0]).unwrap()),
        ),
        (
            "fr crosses [3.3,3.6]".to_string(),
            arc(FloatRange::new_crosses_query("fr", &[3.3], &[3.6]).unwrap()),
        ),
        (
            "dr intersects [5,5]-[10,10]".to_string(),
            arc(DoubleRange::new_intersects_query("dr", &[5.0, 5.0], &[10.0, 10.0]).unwrap()),
        ),
        (
            "dr contains [20,10]-[20.5,10.5]".to_string(),
            arc(DoubleRange::new_contains_query("dr", &[20.0, 10.0], &[20.5, 10.5]).unwrap()),
        ),
        (
            "dr within [0,0]-[30,25]".to_string(),
            arc(DoubleRange::new_within_query("dr", &[0.0, 0.0], &[30.0, 25.0]).unwrap()),
        ),
        (
            "dr crosses [10,10]-[12,12]".to_string(),
            arc(DoubleRange::new_crosses_query("dr", &[10.0, 10.0], &[12.0, 12.0]).unwrap()),
        ),
        (
            "inet exact 10.0.0.7".to_string(),
            InetAddressPoint::new_exact_query("inet", inet(&[10, 0, 0, 7])).unwrap(),
        ),
        (
            "inet prefix 10.0.1.0/24".to_string(),
            InetAddressPoint::new_prefix_query("inet", inet(&[10, 0, 1, 0]), 24).unwrap(),
        ),
        (
            "inet range [10.0.0.0,10.0.0.127]".to_string(),
            InetAddressPoint::new_range_query("inet", inet(&[10, 0, 0, 0]), inet(&[10, 0, 0, 127]))
                .unwrap(),
        ),
        (
            "inet set {10.0.0.14,10.0.2.0,2001:db8::a}".to_string(),
            InetAddressPoint::new_set_query(
                "inet",
                &[inet(&[10, 0, 0, 14]), inet(&[10, 0, 2, 0]), inet(&ipv6(10))],
            )
            .unwrap(),
        ),
        (
            "feature saturation pagerank w=1 pivot=10".to_string(),
            FeatureField::new_saturation_query("features", "pagerank", 1.0, 10.0).unwrap(),
        ),
        (
            "feature saturation pagerank auto pivot".to_string(),
            FeatureField::new_saturation_query_auto_pivot("features", "pagerank").unwrap(),
        ),
        (
            "feature log pagerank w=2 scaling=1.5".to_string(),
            FeatureField::new_log_query("features", "pagerank", 2.0, 1.5).unwrap(),
        ),
        (
            "feature sigmoid recency w=1 pivot=15 exp=0.6".to_string(),
            FeatureField::new_sigmoid_query("features", "recency", 1.0, 15.0, 0.6).unwrap(),
        ),
        (
            "feature linear pagerank w=0.5".to_string(),
            FeatureField::new_linear_query("features", "pagerank", 0.5).unwrap(),
        ),
        (
            "ts distanceFeature w=2 origin=+50000s pivot=5000s".to_string(),
            LongField::new_distance_feature_query("ts", 2.0, TS_BASE + 50_000_000, 5_000_000)
                .unwrap(),
        ),
    ]);
    q
}

/// `DvRangeFeatureFixture.NUMERIC_SELECTORS`, with their Java names.
const NUMERIC_SELECTORS: [(SortedNumericSelectorType, &str); 2] = [
    (SortedNumericSelectorType::MIN, "MIN"),
    (SortedNumericSelectorType::MAX, "MAX"),
];

/// `DvRangeFeatureFixture.SET_SELECTORS`, with their Java names.
const SET_SELECTORS: [(SortedSetSelectorType, &str); 4] = [
    (SortedSetSelectorType::MIN, "MIN"),
    (SortedSetSelectorType::MAX, "MAX"),
    (SortedSetSelectorType::MIDDLE_MIN, "MIDDLE_MIN"),
    (SortedSetSelectorType::MIDDLE_MAX, "MIDDLE_MAX"),
];

/// `DvRangeFeatureFixture.missing(SortField.Type, boolean)`.
fn missing(field_type: SortFieldType, low: bool) -> MissingValue {
    match field_type {
        SortFieldType::Long => MissingValue::Long(if low { i64::MIN } else { i64::MAX }),
        SortFieldType::Int => MissingValue::Int(if low { i32::MIN } else { i32::MAX }),
        SortFieldType::Float => MissingValue::Float(if low {
            f32::NEG_INFINITY
        } else {
            f32::INFINITY
        }),
        SortFieldType::Double => MissingValue::Double(if low {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }),
        other => panic!("no numeric missing value for {other:?}"),
    }
}

/// `DvRangeFeatureFixture.sorts()`, in the same order.
fn sorts() -> Vec<(String, Sort)> {
    let mut s = Vec::new();
    let numeric = [
        ("sn", SortFieldType::Long, "LONG"),
        ("sn", SortFieldType::Int, "INT"),
        ("snf", SortFieldType::Float, "FLOAT"),
        ("snd", SortFieldType::Double, "DOUBLE"),
    ];
    for (field, field_type, type_name) in numeric {
        for (selector, selector_name) in NUMERIC_SELECTORS {
            for reverse in [false, true] {
                for first in [true, false] {
                    // First when ascending is the lowest value, first when
                    // descending the highest.
                    let sf = SortedNumericSortField::with_missing_value(
                        field,
                        field_type,
                        reverse,
                        selector,
                        Some(missing(field_type, first != reverse)),
                    )
                    .unwrap();
                    s.push((
                        format!(
                            "sortedNumeric {field} {type_name} {selector_name} reverse={reverse} missing={}",
                            if first { "first" } else { "last" }
                        ),
                        Sort::new_fields(vec![sf.into_sort_field()]).unwrap(),
                    ));
                }
            }
        }
    }
    for (selector, selector_name) in SET_SELECTORS {
        for reverse in [false, true] {
            for first in [true, false] {
                let sf = SortedSetSortField::with_missing_value(
                    "ss",
                    reverse,
                    selector,
                    Some(if first {
                        MissingValue::StringFirst
                    } else {
                        MissingValue::StringLast
                    }),
                )
                .unwrap();
                s.push((
                    format!(
                        "sortedSet ss {selector_name} reverse={reverse} missing={}",
                        if first { "STRING_FIRST" } else { "STRING_LAST" }
                    ),
                    Sort::new_fields(vec![sf.into_sort_field()]).unwrap(),
                ));
            }
        }
    }
    s
}

/// `DvRangeFeatureFixture.sortedQueries()`.
fn sorted_queries() -> Vec<(&'static str, Arc<dyn Query>)> {
    vec![
        (
            "indexSort ns [50,120]",
            arc(IndexSortSortedNumericDocValuesRangeQuery::new(
                "ns",
                50,
                120,
                NumericDocValuesField::new_slow_range_query("ns", 50, 120),
            )),
        ),
        (
            "indexSort ns [0,10]",
            arc(IndexSortSortedNumericDocValuesRangeQuery::new(
                "ns",
                0,
                10,
                NumericDocValuesField::new_slow_range_query("ns", 0, 10),
            )),
        ),
    ]
}

/// `DvRangeFeatureFixture.sortValue(Object)`.
fn sort_value(fd: &FieldDoc) -> String {
    match &fd.fields.as_ref().expect("sort values")[0] {
        SortValue::Null => "null".to_string(),
        SortValue::Long(v) => format!("Long:{v}"),
        SortValue::Int(v) => format!("Integer:{v}"),
        SortValue::Float(v) => format!("Float:{}", bits(*v)),
        SortValue::Double(v) => format!("Double:{}", bits64(*v)),
        SortValue::Bytes(b) => format!("bytes:{}", String::from_utf8_lossy(b.slice())),
    }
}

/// `DvRangeFeatureFixture.printHits`: `sort` is set for a field-sorted hit.
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

/// `DvRangeFeatureFixture.printReader`.
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

fn run_queries(
    out: &mut String,
    searcher: &IndexSearcher,
    stored: &dyn StoredFields,
    queries: Vec<(String, Arc<dyn Query>)>,
) {
    for (name, query) in queries {
        let top = searcher.search(query, TOP_N).unwrap();
        let hits: Vec<(ScoreDoc, Option<String>)> =
            top.score_docs.iter().map(|h| (*h, None)).collect();
        print_hits(
            out,
            &format!("query {name}"),
            &top.total_hits,
            &hits,
            stored,
        );
    }
}

/// Rucene's run of the query set over the indexes under `root`: the same
/// lines `DvRangeFeatureFixture query` prints.
fn query_report(root: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let mut out = String::new();

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(root.join("main")).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    writeln!(out, "index main").unwrap();
    print_reader(&mut out, &reader);
    run_queries(&mut out, &searcher, stored.as_ref(), queries());
    for (name, sort) in sorts() {
        let top = searcher
            .search_with_sort(Arc::new(MatchAllDocsQuery::instance()), TOP_SORT, &sort)
            .unwrap();
        let hits: Vec<(ScoreDoc, Option<String>)> = top
            .score_docs
            .iter()
            .map(|fd| (fd.score_doc, Some(sort_value(fd))))
            .collect();
        print_hits(
            &mut out,
            &format!("sort {name}"),
            &top.total_hits,
            &hits,
            stored.as_ref(),
        );
    }

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(root.join("sorted")).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    writeln!(out, "index sorted").unwrap();
    print_reader(&mut out, &reader);
    run_queries(
        &mut out,
        &searcher,
        stored.as_ref(),
        sorted_queries()
            .into_iter()
            .map(|(n, q)| (n.to_string(), q))
            .collect(),
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

/// Runs `DvRangeFeatureFixture <mode> <root>`: `(success, stdout, stderr)`.
fn run_java(mode: &str, root: &Path) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         DvRangeFeatureFixture.java",
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
        if a.starts_with("index ") {
            section = a;
        }
        if a.starts_with("query ") || a.starts_with("sort ") {
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
fn doc_values_range_and_feature_queries_and_selector_sorts_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-dv-range-feature")
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

    // Both sides write both corpora.
    write_all(&r, false);
    let (ok, _, err) = run_java("write", &l);
    assert!(ok, "Lucene failed to write the corpora:\n{err}");

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

    // Every query and sort of the oracle must have hits: an empty result
    // would make the comparisons vacuous.
    let oracle = lucene_on_l
        .as_ref()
        .expect("the Lucene report on L is required");
    let mut lines = oracle.lines().peekable();
    let mut query_count = 0;
    while let Some(line) = lines.next() {
        if line.starts_with("query ") || line.starts_with("sort ") {
            query_count += 1;
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  hit ")),
                "the oracle query has no hits: {line}"
            );
        }
    }
    assert_eq!(
        query_count,
        queries().len() + sorts().len() + sorted_queries().len(),
        "the oracle must report every query and sort"
    );
    assert!(
        oracle.contains(" sort=null ") && oracle.contains(" sort=Long:-9223372036854775808 "),
        "the sorts must exercise missing values"
    );
    writeln!(
        report,
        "== oracle report ({} lines, {query_count} queries and sorts)",
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

    // Positive control: one more sorted-numeric value and a different
    // numeric value in document 0 must change the report, with the same
    // document counts.
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
        "{} doc-values/range/feature failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
