//! A stored field and a doc-values field under one name, as Lucene indexes
//! them.
//!
//! `IndexingChain.processDocument` folds every instance of a field name in a
//! document into one `FieldSchema` before it creates or checks the field's
//! `FieldInfo` (`IndexingChain.java:589-702, 2136-2287`), so a `StoredField`
//! and a `SortedDocValuesField` named `id` describe one field that is stored
//! and has sorted doc values. `StoredDocValuesFixture`
//! (`tests/fixtures/java-codec-harness`, run with `java -cp
//! lucene-core-10.5.0.jar StoredDocValuesFixture.java`) is the oracle:
//!
//! * every doc-values type combined with a stored field, with and without an
//!   index sort, is written by Rucene and by Lucene, and Lucene reads both
//!   back — stored values, doc values, the leaf's sort and `CheckIndex` —
//!   with the same report;
//! * the schema cases — the frozen-type shortcut, a type changing across
//!   documents within a segment and across segments, contradicting instances
//!   in one document, an index sort over the wrong type, and which field
//!   classes share one frozen type (`IndexingChain.java:1803-1825`) — are
//!   accepted or refused with the same messages;
//! * a flush under an index sort over a field without doc values, or without
//!   a `FieldInfo`, fails and closes the writer as Lucene's does.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::feature_field::FeatureField;
use rucene::document::geo_fields::{
    InetAddressPoint, InetAddressRange, LatLonDocValuesField, LatLonPoint, XYDocValuesField,
    XYPointField,
};
use rucene::document::{
    BinaryDocValuesField, Document, DoublePoint, Field, FieldType, FloatPoint, IntPoint,
    LatLonShape, LongPoint, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField, StringField,
    TextField, XYShape,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexableField;
use rucene::search::{SortField, SortFieldType, SortedNumericSortField, SortedSetSortField};
use rucene::store::{ByteBuffersDirectory, Directory, FSDirectory};
use rucene::util::BytesRef;
use rucene::{LuceneError, LuceneErrorKind};

/// `StoredDocValuesFixture.KINDS`.
const KINDS: [&str; 5] = [
    "NUMERIC",
    "BINARY",
    "SORTED",
    "SORTED_NUMERIC",
    "SORTED_SET",
];

/// `StoredDocValuesFixture.DOCS`.
const DOCS: i32 = 3;

fn bytes(text: &str) -> BytesRef {
    BytesRef::new(text.as_bytes().to_vec())
}

/// `StoredDocValuesFixture.docValues(kind, i)`: value `2 - i`.
fn doc_values(kind: &str, i: i32) -> Vec<Box<dyn IndexableField>> {
    let v = i64::from(2 - i);
    match kind {
        "NUMERIC" => vec![Box::new(NumericDocValuesField::new("f", v))],
        "BINARY" => vec![Box::new(BinaryDocValuesField::new(
            "f",
            bytes(&format!("b{v}")),
        ))],
        "SORTED" => vec![Box::new(SortedDocValuesField::new(
            "f",
            bytes(&format!("s{v}")),
        ))],
        "SORTED_NUMERIC" => vec![
            Box::new(SortedNumericDocValuesField::new("f", v)),
            Box::new(SortedNumericDocValuesField::new("f", 10 + v)),
        ],
        "SORTED_SET" => vec![
            Box::new(SortedSetDocValuesField::new("f", bytes(&format!("t{v}")))),
            Box::new(SortedSetDocValuesField::new("f", bytes(&format!("u{v}")))),
        ],
        other => panic!("unknown kind {other}"),
    }
}

/// `StoredDocValuesFixture.sort(kind)`.
fn sort(kind: &str) -> rucene::search::Sort {
    let field = match kind {
        "NUMERIC" => SortField::new(Some("f".to_string()), SortFieldType::Long).unwrap(),
        "BINARY" => SortField::new(Some("key".to_string()), SortFieldType::Long).unwrap(),
        "SORTED" => SortField::new(Some("f".to_string()), SortFieldType::String).unwrap(),
        "SORTED_NUMERIC" => SortedNumericSortField::new("f", SortFieldType::Long)
            .unwrap()
            .into_sort_field(),
        "SORTED_SET" => SortedSetSortField::new("f", false)
            .unwrap()
            .into_sort_field(),
        other => panic!("unknown kind {other}"),
    };
    rucene::search::Sort::new_fields(vec![field]).unwrap()
}

/// `StoredDocValuesFixture.doc(kind, i)`.
fn doc(kind: &str, i: i32) -> Document {
    let mut d = Document::new();
    let stored = StoredField::new_string("f", format!("v{i}")).unwrap();
    let mut stored = Some(Box::new(stored) as Box<dyn IndexableField>);
    if i % 2 == 0 {
        d.add(stored.take().unwrap());
    }
    for field in doc_values(kind, i) {
        d.add(field);
    }
    if let Some(stored) = stored {
        d.add(stored);
    }
    d.add(Box::new(NumericDocValuesField::new(
        "key",
        i64::from(2 - i),
    )));
    d
}

/// `StoredDocValuesFixture.write(root)`.
fn write(root: &Path) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    for kind in KINDS {
        for sorted in [false, true] {
            let path = root.join(format!(
                "{kind}{}",
                if sorted { "-sorted" } else { "-unsorted" }
            ));
            std::fs::create_dir_all(&path).unwrap();
            let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&path).unwrap());
            let config = IndexWriterConfig::new();
            if sorted {
                config.set_index_sort(sort(kind)).unwrap();
            }
            let writer = IndexWriter::new(dir, config).unwrap();
            for i in 0..DOCS {
                writer
                    .add_document(&doc(kind, i))
                    .unwrap_or_else(|e| panic!("{kind} sorted={sorted} doc {i}: {e}"));
            }
            writer.close().unwrap();
        }
    }
}

fn lucene_jar() -> std::path::PathBuf {
    let jar = Path::new(&std::env::var("HOME").unwrap())
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Runs `StoredDocValuesFixture <args>` and returns its standard output.
fn run_java(args: &[&str]) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         StoredDocValuesFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .args(args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "StoredDocValuesFixture {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Lucene reads the segments Rucene writes for every doc-values type combined
/// with a stored field, sorted and unsorted, exactly as it reads its own.
#[test]
fn lucene_reads_rucene_stored_fields_combined_with_every_doc_values_type() {
    let root = tempfile::tempdir().unwrap();
    let rucene_root = root.path().join("rucene");
    let lucene_root = root.path().join("lucene");
    write(&rucene_root);
    std::fs::create_dir_all(&lucene_root).unwrap();
    run_java(&["write", lucene_root.to_str().unwrap()]);

    let on_rucene = run_java(&["read", rucene_root.to_str().unwrap()]);
    let on_lucene = run_java(&["read", lucene_root.to_str().unwrap()]);
    assert_eq!(
        on_rucene.matches("case ").count(),
        KINDS.len() * 2,
        "{on_rucene}"
    );
    assert_eq!(
        on_rucene.matches("clean=true").count(),
        KINDS.len() * 2,
        "{on_rucene}"
    );
    assert_eq!(on_rucene, on_lucene);
}

// ---------------------------------------------------------------------------
// Schema cases: `StoredDocValuesFixture.schema()`, line for line
// ---------------------------------------------------------------------------

/// `getClass().getSimpleName() + ": " + getMessage()` of the Java exception
/// `error` stands for.
fn java_exception(error: &LuceneError) -> String {
    let message = match error {
        LuceneError {
            kind: LuceneErrorKind::IllegalArgument { message, .. },
            ..
        } => message.clone(),
        LuceneError {
            kind: LuceneErrorKind::IllegalState { message, .. },
            ..
        } => message.clone(),
        LuceneError {
            kind: LuceneErrorKind::Unchecked { message, .. },
            ..
        } => message.clone().unwrap_or_else(|| "null".to_string()),
        other => other.to_string(),
    };
    format!("{}: {message}", error.java_simple_name())
}

fn add(out: &mut String, w: &IndexWriter, label: &str, fields: Vec<Box<dyn IndexableField>>) {
    let mut d = Document::new();
    for field in fields {
        d.add(field);
    }
    match w.add_document(&d) {
        Ok(_) => out.push_str(&format!("{label}: ok\n")),
        Err(error) => out.push_str(&format!("{label}: {}\n", java_exception(&error))),
    }
}

fn memory_writer(sort: Option<rucene::search::Sort>) -> Arc<IndexWriter> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let config = IndexWriterConfig::new();
    if let Some(sort) = sort {
        config.set_index_sort(sort).unwrap();
    }
    IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap()
}

fn stored(name: &str, value: &str) -> Box<dyn IndexableField> {
    Box::new(StoredField::new_string(name, value.to_string()).unwrap())
}

fn sdv(name: &str, value: &str) -> Box<dyn IndexableField> {
    Box::new(SortedDocValuesField::new(name, bytes(value)))
}

fn ndv(name: &str, value: i64) -> Box<dyn IndexableField> {
    Box::new(NumericDocValuesField::new(name, value))
}

fn typed(name: &str, value: &str, field_type: &FieldType) -> Box<dyn IndexableField> {
    Box::new(Field::new(name, value.to_string(), field_type.clone()).unwrap())
}

fn stored_type(frozen: bool) -> FieldType {
    let field_type = FieldType::new();
    field_type.set_stored(true).unwrap();
    if frozen {
        field_type.freeze();
    }
    field_type
}

fn rucene_schema() -> String {
    let mut out = String::new();
    {
        let w = memory_writer(None);
        add(
            &mut out,
            &w,
            "Q1 doc0 [stored, sdv]",
            vec![stored("id", "a"), sdv("id", "a")],
        );
        add(&mut out, &w, "Q1 doc1 [stored]", vec![stored("id", "b")]);
        add(&mut out, &w, "Q1 doc2 [sdv]", vec![sdv("id", "c")]);
        add(&mut out, &w, "Q1 doc3 [stored]", vec![stored("id", "d")]);
        w.close().unwrap();
    }
    {
        let w = memory_writer(None);
        add(
            &mut out,
            &w,
            "Q2 doc0 [sdv, stored]",
            vec![sdv("id", "a"), stored("id", "a")],
        );
        add(&mut out, &w, "Q2 doc1 [stored]", vec![stored("id", "b")]);
        add(&mut out, &w, "Q2 doc2 [sdv]", vec![sdv("id", "c")]);
        w.close().unwrap();
    }
    {
        // A mutable type is never taken as validated, however often it is
        // reused: Java tests `isFrozen()` (`IndexingChain.java:1819-1825`).
        let ft = stored_type(false);
        let w = memory_writer(None);
        add(
            &mut out,
            &w,
            "Q3 doc0 [stored(unfrozen), sdv]",
            vec![typed("id", "a", &ft), sdv("id", "a")],
        );
        add(
            &mut out,
            &w,
            "Q3 doc1 [stored(unfrozen)]",
            vec![typed("id", "b", &ft)],
        );
        w.close().unwrap();
    }
    {
        // Two types frozen separately are two Java objects, even when equal.
        let t1 = stored_type(true);
        let t2 = stored_type(true);
        let w = memory_writer(None);
        add(
            &mut out,
            &w,
            "Q4 doc0 [t1, sdv]",
            vec![typed("id", "a", &t1), sdv("id", "a")],
        );
        add(&mut out, &w, "Q4 doc1 [t2]", vec![typed("id", "b", &t2)]);
        add(&mut out, &w, "Q4 doc2 [t1]", vec![typed("id", "c", &t1)]);
        w.close().unwrap();
    }
    {
        let w = memory_writer(None);
        add(&mut out, &w, "Q5 doc0 [stored]", vec![stored("id", "a")]);
        add(
            &mut out,
            &w,
            "Q5 doc1 [stored, sdv]",
            vec![stored("id", "b"), sdv("id", "b")],
        );
        add(&mut out, &w, "Q5 doc2 [stored]", vec![stored("id", "c")]);
        w.close().unwrap();
    }
    {
        let w = memory_writer(None);
        add(&mut out, &w, "X1 doc0 [sdv]", vec![sdv("id", "a")]);
        add(&mut out, &w, "X1 doc1 [ndv]", vec![ndv("id", 1)]);
        add(
            &mut out,
            &w,
            "X1 doc2 [stored, ndv]",
            vec![stored("id", "x"), ndv("id", 1)],
        );
        w.commit().unwrap();
        add(
            &mut out,
            &w,
            "X1 doc3 next segment [ndv]",
            vec![ndv("id", 1)],
        );
        add(
            &mut out,
            &w,
            "X1 doc4 next segment [stored, ndv]",
            vec![stored("id", "x"), ndv("id", 1)],
        );
        w.close().unwrap();
    }
    {
        let w = memory_writer(None);
        add(
            &mut out,
            &w,
            "X2 doc0 [sdv, ndv]",
            vec![sdv("id", "a"), ndv("id", 1)],
        );
        add(
            &mut out,
            &w,
            "X2 doc1 [string, sdv]",
            vec![
                Box::new(StringField::new("id", "a".to_string(), Store::YES).unwrap()),
                sdv("id", "a"),
            ],
        );
        add(
            &mut out,
            &w,
            "X2 doc2 [text]",
            vec![Box::new(
                TextField::new("id", "a b".to_string(), Store::NO).unwrap(),
            )],
        );
        w.close().unwrap();
    }
    {
        let w = memory_writer(Some(
            rucene::search::Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        ));
        add(
            &mut out,
            &w,
            "X3 sort-by-id [stored, ndv]",
            vec![stored("id", "a"), ndv("id", 1)],
        );
        add(
            &mut out,
            &w,
            "X3 sort-by-id [stored, sdv]",
            vec![stored("id", "a"), sdv("id", "a")],
        );
        w.close().unwrap();
    }
    shared_type_cases(&mut out);
    out
}

/// Boxes a field.
fn boxed(field: impl IndexableField + 'static) -> Box<dyn IndexableField> {
    Box::new(field)
}

/// `StoredDocValuesFixture.address(last)`: `10.0.0.<last>`.
fn address(last: u8) -> [u8; 4] {
    [10, 0, 0, last]
}

/// Adds the documents of one schema case, each labelled `<case> doc<i> <what>`,
/// to a new in-memory writer.
fn schema_case(out: &mut String, case: &str, docs: Vec<(&str, Vec<Box<dyn IndexableField>>)>) {
    let w = memory_writer(None);
    for (i, (what, fields)) in docs.into_iter().enumerate() {
        add(out, &w, &format!("{case} doc{i} [{what}]"), fields);
    }
    w.close().unwrap();
}

/// The cases of `StoredDocValuesFixture.schema()` that tell which field
/// classes share one frozen type: a point builds a new type per instance
/// (`IntPoint.java:48-53` and its siblings), so a later point is validated;
/// the geo, address and shape fields share one static type per class, so a
/// later field of the same class is not; `FeatureField`'s two types are never
/// frozen.
fn shared_type_cases(out: &mut String) {
    schema_case(
        out,
        "P1",
        vec![
            (
                "intpoint, ndv",
                vec![boxed(IntPoint::new("p", &[1]).unwrap()), ndv("p", 1)],
            ),
            ("intpoint", vec![boxed(IntPoint::new("p", &[2]).unwrap())]),
        ],
    );
    schema_case(
        out,
        "P2",
        vec![
            (
                "longpoint, ndv",
                vec![boxed(LongPoint::new("p", &[1]).unwrap()), ndv("p", 1)],
            ),
            ("longpoint", vec![boxed(LongPoint::new("p", &[2]).unwrap())]),
        ],
    );
    schema_case(
        out,
        "P3",
        vec![
            (
                "floatpoint, ndv",
                vec![boxed(FloatPoint::new("p", &[1.0]).unwrap()), ndv("p", 1)],
            ),
            (
                "floatpoint",
                vec![boxed(FloatPoint::new("p", &[2.0]).unwrap())],
            ),
        ],
    );
    schema_case(
        out,
        "P4",
        vec![
            (
                "doublepoint, ndv",
                vec![boxed(DoublePoint::new("p", &[1.0]).unwrap()), ndv("p", 1)],
            ),
            (
                "doublepoint",
                vec![boxed(DoublePoint::new("p", &[2.0]).unwrap())],
            ),
        ],
    );
    let lat_lon_point = |lat: f64, lon: f64| boxed(LatLonPoint::new("g", lat, lon).unwrap());
    let lat_lon_dv = |lat: f64, lon: f64| boxed(LatLonDocValuesField::new("g", lat, lon).unwrap());
    let xy_point = |x: f32, y: f32| boxed(XYPointField::new("g", x, y).unwrap());
    let xy_dv = |x: f32, y: f32| boxed(XYDocValuesField::new("g", x, y).unwrap());
    schema_case(
        out,
        "G1",
        vec![
            (
                "latlonpoint, latlondv",
                vec![lat_lon_point(1.0, 2.0), lat_lon_dv(1.0, 2.0)],
            ),
            ("latlonpoint", vec![lat_lon_point(3.0, 4.0)]),
            ("xypoint", vec![xy_point(3.0, 4.0)]),
        ],
    );
    schema_case(
        out,
        "G2",
        vec![
            ("xypoint, xydv", vec![xy_point(1.0, 2.0), xy_dv(1.0, 2.0)]),
            ("xypoint", vec![xy_point(3.0, 4.0)]),
            ("latlonpoint", vec![lat_lon_point(3.0, 4.0)]),
        ],
    );
    schema_case(
        out,
        "D1",
        vec![
            (
                "latlondv, latlonpoint",
                vec![lat_lon_dv(1.0, 2.0), lat_lon_point(1.0, 2.0)],
            ),
            ("latlondv", vec![lat_lon_dv(3.0, 4.0)]),
            ("xydv", vec![xy_dv(3.0, 4.0)]),
        ],
    );
    schema_case(
        out,
        "D2",
        vec![
            ("xydv, xypoint", vec![xy_dv(1.0, 2.0), xy_point(1.0, 2.0)]),
            ("xydv", vec![xy_dv(3.0, 4.0)]),
            ("latlondv", vec![lat_lon_dv(3.0, 4.0)]),
        ],
    );
    schema_case(
        out,
        "I1",
        vec![
            (
                "inetpoint, ndv",
                vec![
                    boxed(InetAddressPoint::new("a", &address(1)).unwrap()),
                    ndv("a", 1),
                ],
            ),
            (
                "inetpoint",
                vec![boxed(InetAddressPoint::new("a", &address(2)).unwrap())],
            ),
        ],
    );
    schema_case(
        out,
        "R1",
        vec![
            (
                "inetrange, ndv",
                vec![
                    boxed(InetAddressRange::new("r", &address(1), &address(2)).unwrap()),
                    ndv("r", 1),
                ],
            ),
            (
                "inetrange",
                vec![boxed(
                    InetAddressRange::new("r", &address(3), &address(4)).unwrap(),
                )],
            ),
        ],
    );
    let mut lat_lon_shape: Vec<Box<dyn IndexableField>> =
        LatLonShape::create_indexable_fields_point("s", 1.0, 2.0)
            .unwrap()
            .into_iter()
            .map(boxed)
            .collect();
    lat_lon_shape.push(ndv("s", 1));
    let xy_shape: Vec<Box<dyn IndexableField>> =
        XYShape::create_indexable_fields_point("s", 3.0, 4.0)
            .unwrap()
            .into_iter()
            .map(boxed)
            .collect();
    schema_case(
        out,
        "T1",
        vec![("latlonshape, ndv", lat_lon_shape), ("xyshape", xy_shape)],
    );
    schema_case(
        out,
        "S1",
        vec![
            (
                "latlonshapedv, intpoint",
                vec![
                    boxed(LatLonShape::create_doc_value_field_point("s", 1.0, 2.0).unwrap()),
                    boxed(IntPoint::new("s", &[1]).unwrap()),
                ],
            ),
            (
                "xyshapedv",
                vec![boxed(
                    XYShape::create_doc_value_field_point("s", 3.0, 4.0).unwrap(),
                )],
            ),
        ],
    );
    schema_case(
        out,
        "F1",
        vec![
            (
                "feature, ndv",
                vec![
                    boxed(FeatureField::new("f", "a", 1.0).unwrap()),
                    ndv("f", 1),
                ],
            ),
            (
                "feature",
                vec![boxed(FeatureField::new("f", "b", 2.0).unwrap())],
            ),
        ],
    );
    out.push_str(&format!(
        "F0 frozen={} with-term-vectors={}\n",
        FeatureField::new("f", "a", 1.0)
            .unwrap()
            .field_type()
            .is_frozen(),
        FeatureField::with_term_vectors("f", "a", 1.0, true)
            .unwrap()
            .field_type()
            .is_frozen()
    ));
}

/// Every schema case is accepted or refused as Lucene does, with Lucene's
/// message: within a segment the `FieldSchema` check of
/// `IndexingChain.java:2272-2286`, across segments the global field numbers'
/// `FieldInfo.verifySameSchema`.
#[test]
fn schema_cases_are_accepted_and_refused_as_lucene_does() {
    assert_eq!(rucene_schema(), run_java(&["schema"]));
}

// ---------------------------------------------------------------------------
// Flush cases: `StoredDocValuesFixture.flush()`
// ---------------------------------------------------------------------------

/// `StoredDocValuesFixture.commit(w, label)`.
fn commit(out: &mut String, w: &IndexWriter, label: &str) {
    match w.commit() {
        Ok(_) => out.push_str(&format!("{label} commit: ok\n")),
        Err(error) => out.push_str(&format!("{label} commit: {}\n", java_exception(&error))),
    }
    out.push_str(&format!("{label} open={}\n", w.is_open()));
}

fn rucene_flush() -> String {
    let mut out = String::new();
    let sorts = vec![
        SortField::new(Some("f".to_string()), SortFieldType::Long).unwrap(),
        SortedSetSortField::new("f", false)
            .unwrap()
            .into_sort_field(),
        SortedNumericSortField::new("f", SortFieldType::Long)
            .unwrap()
            .into_sort_field(),
        SortField::new(Some("f".to_string()), SortFieldType::String).unwrap(),
    ];
    for sort_field in sorts {
        let sort = rucene::search::Sort::new_fields(vec![sort_field]).unwrap();
        let w = memory_writer(Some(sort.clone()));
        for (i, value) in ["x", "y"].into_iter().enumerate() {
            add(
                &mut out,
                &w,
                &format!("N {sort} doc{i}"),
                vec![Box::new(
                    StringField::new("f", value.to_string(), Store::NO).unwrap(),
                )],
            );
        }
        commit(&mut out, &w, &format!("N {sort}"));
    }
    let w = memory_writer(Some(
        rucene::search::Sort::new_fields(vec![SortField::new(
            Some("f".to_string()),
            SortFieldType::Long,
        )
        .unwrap()])
        .unwrap(),
    ));
    add(&mut out, &w, "M doc0 [sdv]", vec![sdv("f", "x")]);
    add(&mut out, &w, "M doc1 [ndv key]", vec![ndv("key", 1)]);
    commit(&mut out, &w, "M");
    out
}

/// A flush under an index sort fails as Lucene's does when the sort field
/// carries no doc values — the buffered reader answers no values and
/// `getFieldInfos()` the segment's (`IndexingChain.java:243`), so
/// `DocValues.checkField` throws (`DocValues.java:206-222`) — and when the
/// field has no `FieldInfo` because its only document failed validation,
/// where Java's reader dereferences the missing `FieldInfo`
/// (`IndexingChain.java:187`), a `NullPointerException` with the JVM's
/// message. Either failure aborts the writer.
#[test]
fn a_sort_over_a_field_without_doc_values_fails_the_flush_as_lucene_does() {
    assert_eq!(rucene_flush(), run_java(&["flush"]));
}
