//! Portability tests for the `Object` and `Iterable` contract methods of the
//! `document`, `search` and `geo` packages, against Apache Lucene Core 10.5.0.
//!
//! Every expected string in this file was **measured**, not written from the
//! Java source: `tests/fixtures/java-codec-harness/src/main/java/org/apache/
//! lucene/document/RuceneObjectContractFixture.java` constructs the same inputs
//! against `lucene-core-10.5.0.jar` and prints what `toString()` returns, and
//! the literals below are that output. Reproduce it with
//!
//! ```text
//! cd tests/fixtures/java-codec-harness
//! mvn -q -o compile exec:java \
//!   -Dexec.mainClass=org.apache.lucene.document.RuceneObjectContractFixture
//! ```
//!
//! These strings are observable in Lucene — they reach score explanations,
//! `IndexWriter` error messages and Luke — so reproducing them exactly is part
//! of functional parity, not cosmetics.

#![deny(unsafe_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rucene::document::doc_values_queries::DocValuesLongHashSet;
use rucene::document::geo_fields::{
    InetAddressPoint, InetAddressRange, LatLonDocValuesField, LatLonPoint, XYDocValuesField,
    XYPointField,
};
use rucene::document::range_fields::{DoubleRange, FloatRange, IntRange, LongRange};
use rucene::document::{
    DoubleField, DoublePoint, Field, FieldType, FloatField, FloatPoint, IntField, IntPoint,
    LongField, LongPoint, NumericValue, Store,
};
use rucene::geo::geometry::{
    Circle, Line, Point, Polygon, Rectangle, XYCircle, XYLine, XYPoint, XYPolygon, XYRectangle,
};
use rucene::util::BytesRef;
use std::sync::Arc;

use rucene::document::shape_field::QueryRelation;
use rucene::document::LatLonShapeBoundingBoxQuery;
use rucene::index::{DocValuesType, IndexOptions, LeafReaderContext, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::{
    CollectionResult, Collector, FilterCollector, FilterLeafCollector, LeafCollector,
};
use rucene::search::{
    DisjunctionMaxQuery, LogOddsFusionQuery, Query, Scorable, ScoreMode, TermQuery,
};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

// -----------------------------------------------------------------------------
// geo
// -----------------------------------------------------------------------------

#[test]
fn geo_point_to_string_matches_java() {
    // Java prints the longitude first, and prints `35.0` where Rust's own
    // `Display` would print `35`.
    assert_eq!(
        Point::new(35.0, -70.5).unwrap().to_string(),
        "Point(-70.5,35.0)"
    );
    // Outside [10^-3, 10^7) Java switches to scientific notation.
    assert_eq!(
        Point::new(1.0e-4, -1.0e-5).unwrap().to_string(),
        "Point(-1.0E-5,1.0E-4)"
    );
}

#[test]
fn geo_rectangle_to_string_matches_java() {
    assert_eq!(
        Rectangle::new(-10.0, 10.0, -20.0, 20.0)
            .unwrap()
            .to_string(),
        "Rectangle(lat=-10.0 TO 10.0 lon=-20.0 TO 20.0)"
    );
    assert_eq!(
        Rectangle::new(-10.0, 10.0, 20.0, -20.0)
            .unwrap()
            .to_string(),
        "Rectangle(lat=-10.0 TO 10.0 lon=20.0 TO -20.0 [crosses dateline!])"
    );
}

#[test]
fn geo_circle_and_line_to_string_match_java() {
    assert_eq!(
        Circle::new(35.0, -70.5, 1000.0).unwrap().to_string(),
        "Circle([35.0,-70.5] radius = 1000.0 meters)"
    );
    // A line prints `[lon, lat]` pairs with no separator between them.
    assert_eq!(
        Line::new(vec![0.0, 1.5], vec![2.5, 3.0])
            .unwrap()
            .to_string(),
        "Line([2.5, 0.0][3.0, 1.5])"
    );
}

#[test]
fn geo_polygon_to_string_matches_java() {
    let square = Polygon::new(
        vec![0.0, 1.0, 1.0, 0.0, 0.0],
        vec![0.0, 0.0, 1.0, 1.0, 0.0],
        Vec::new(),
    )
    .unwrap();
    // Note the trailing space after the last vertex: Java appends `"] "`.
    assert_eq!(
        square.to_string(),
        "Polygon[0.0, 0.0] [1.0, 0.0] [1.0, 1.0] [0.0, 1.0] [0.0, 0.0] "
    );

    let hole = Polygon::new(
        vec![2.0, 3.0, 3.0, 2.0, 2.0],
        vec![2.0, 2.0, 3.0, 3.0, 2.0],
        Vec::new(),
    )
    .unwrap();
    let outer = Polygon::new(
        vec![0.0, 10.0, 10.0, 0.0, 0.0],
        vec![0.0, 0.0, 10.0, 10.0, 0.0],
        vec![hole],
    )
    .unwrap();
    assert_eq!(
        outer.to_string(),
        "Polygon[0.0, 0.0] [10.0, 0.0] [10.0, 10.0] [0.0, 10.0] [0.0, 0.0] , \
         holes=[Polygon[2.0, 2.0] [3.0, 2.0] [3.0, 3.0] [2.0, 3.0] [2.0, 2.0] ]"
    );
}

#[test]
fn geo_xy_to_string_matches_java() {
    assert_eq!(
        XYPoint::new(1.5, -2.25).unwrap().to_string(),
        "XYPoint(1.5,-2.25)"
    );
    assert_eq!(
        XYRectangle::new(-1.5, 2.5, -3.5, 4.5).unwrap().to_string(),
        "XYRectangle(x=-1.5 TO 2.5 y=-3.5 TO 4.5)"
    );
    // The cartesian circle prints no unit, unlike the geographic one.
    assert_eq!(
        XYCircle::new(1.5, -2.25, 3.0).unwrap().to_string(),
        "XYCircle([1.5,-2.25] radius = 3.0)"
    );
    assert_eq!(
        XYLine::new(vec![0.0, 1.5], vec![2.5, 3.0])
            .unwrap()
            .to_string(),
        "XYLine([0.0, 2.5][1.5, 3.0])"
    );
    let square = XYPolygon::new(
        vec![0.0, 1.0, 1.0, 0.0, 0.0],
        vec![0.0, 0.0, 1.0, 1.0, 0.0],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        square.to_string(),
        "XYPolygon[0.0, 0.0] [1.0, 0.0] [1.0, 1.0] [0.0, 1.0] [0.0, 0.0] "
    );
    let hole = XYPolygon::new(
        vec![2.0, 3.0, 3.0, 2.0, 2.0],
        vec![2.0, 2.0, 3.0, 3.0, 2.0],
        Vec::new(),
    )
    .unwrap();
    let outer = XYPolygon::new(
        vec![0.0, 10.0, 10.0, 0.0, 0.0],
        vec![0.0, 0.0, 10.0, 10.0, 0.0],
        vec![hole],
    )
    .unwrap();
    assert_eq!(
        outer.to_string(),
        "XYPolygon[0.0, 0.0] [10.0, 0.0] [10.0, 10.0] [0.0, 10.0] [0.0, 0.0] , \
         holes=[XYPolygon[2.0, 2.0] [3.0, 2.0] [3.0, 3.0] [2.0, 3.0] [2.0, 2.0] ]"
    );
}

#[test]
fn rectangle_equality_follows_double_compare() {
    // `Rectangle.equals` uses `Double.compare`, not `==`: it separates `-0.0`
    // from `0.0`. A derived `PartialEq` would call them equal.
    let zero = Rectangle::new(0.0, 10.0, 0.0, 10.0).unwrap();
    let negative_zero = Rectangle::new(-0.0, 10.0, 0.0, 10.0).unwrap();
    assert_ne!(zero, negative_zero);
    assert_eq!(zero, Rectangle::new(0.0, 10.0, 0.0, 10.0).unwrap());
    // `Hash` must agree with that notion of equality.
    assert_eq!(
        hash_of(&zero),
        hash_of(&Rectangle::new(0.0, 10.0, 0.0, 10.0).unwrap())
    );
    assert_ne!(hash_of(&zero), hash_of(&negative_zero));
}

// -----------------------------------------------------------------------------
// document: point and numeric fields
// -----------------------------------------------------------------------------

#[test]
fn point_field_to_string_matches_java() {
    assert_eq!(
        IntPoint::new("count", &[42]).unwrap().to_string(),
        "IntPoint <count:42>"
    );
    assert_eq!(
        IntPoint::new("count", &[-1, 7]).unwrap().to_string(),
        "IntPoint <count:-1,7>"
    );
    assert_eq!(
        LongPoint::new("ts", &[123456789]).unwrap().to_string(),
        "LongPoint <ts:123456789>"
    );
    assert_eq!(
        LongPoint::new("ts", &[-5, 9]).unwrap().to_string(),
        "LongPoint <ts:-5,9>"
    );
    assert_eq!(
        FloatPoint::new("temp", &[2.5]).unwrap().to_string(),
        "FloatPoint <temp:2.5>"
    );
    assert_eq!(
        FloatPoint::new("temp", &[-0.5, 1.0e8]).unwrap().to_string(),
        "FloatPoint <temp:-0.5,1.0E8>"
    );
    assert_eq!(
        DoublePoint::new("d", &[2.5]).unwrap().to_string(),
        "DoublePoint <d:2.5>"
    );
    assert_eq!(
        DoublePoint::new("d", &[-0.5, 1.0e-4]).unwrap().to_string(),
        "DoublePoint <d:-0.5,1.0E-4>"
    );
}

#[test]
fn numeric_field_to_string_matches_java() {
    assert_eq!(
        IntField::new("n", 42, Store::NO).to_string(),
        "IntField <n:42>"
    );
    assert_eq!(
        LongField::new("n", -7, Store::NO).to_string(),
        "LongField <n:-7>"
    );
    assert_eq!(
        FloatField::new("n", 2.5, Store::NO).to_string(),
        "FloatField <n:2.5>"
    );
    assert_eq!(
        FloatField::new("n", 1.0e-8, Store::NO).to_string(),
        "FloatField <n:1.0E-8>"
    );
    assert_eq!(
        DoubleField::new("n", 2.5, Store::NO).to_string(),
        "DoubleField <n:2.5>"
    );
    assert_eq!(
        DoubleField::new("n", 1.0e-8, Store::NO).to_string(),
        "DoubleField <n:1.0E-8>"
    );
}

#[test]
fn geo_field_to_string_matches_java() {
    // The coordinates come back out of the quantised encoding, which is why
    // Java prints 34.99999996740371 for an input of 35.0.
    assert_eq!(
        LatLonPoint::new("loc", 35.0, -70.5).unwrap().to_string(),
        "LatLonPoint <loc:34.99999996740371,-70.50000007264316>"
    );
    assert_eq!(
        LatLonDocValuesField::new("loc", 35.0, -70.5)
            .unwrap()
            .to_string(),
        "LatLonDocValuesField <loc:34.99999996740371,-70.50000007264316>"
    );
    assert_eq!(
        XYPointField::new("xy", 1.5, -2.25).unwrap().to_string(),
        "XYPointField <xy:1.5,-2.25>"
    );
    assert_eq!(
        XYDocValuesField::new("xy", 1.5, -2.25).unwrap().to_string(),
        "XYDocValuesField <xy:1.5,-2.25>"
    );
}

#[test]
fn inet_address_point_to_string_matches_java() {
    // An IPv4 address is printed bare; an IPv6 address is bracketed so that it
    // cannot be read as `field:value`, and is printed uncompressed.
    assert_eq!(
        InetAddressPoint::new("ip", &[1, 2, 3, 4])
            .unwrap()
            .to_string(),
        "InetAddressPoint <ip:1.2.3.4>"
    );
    let mut loopback = [0u8; 16];
    loopback[15] = 1;
    assert_eq!(
        InetAddressPoint::new("ip", &loopback).unwrap().to_string(),
        "InetAddressPoint <ip:[0:0:0:0:0:0:0:1]>"
    );
    let link_local = [
        0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x12, 0x34, 0x56, 0x78,
    ];
    assert_eq!(
        InetAddressPoint::new("ip", &link_local)
            .unwrap()
            .to_string(),
        "InetAddressPoint <ip:[fe80:0:0:0:0:0:1234:5678]>"
    );
}

// -----------------------------------------------------------------------------
// document: ranges
// -----------------------------------------------------------------------------

#[test]
fn range_field_to_string_matches_java() {
    // Java computes the rendering of dimension 0 before the loop and discards
    // it, so the value part opens with the loop's leading space.
    assert_eq!(
        IntRange::new("r", &[10], &[20]).unwrap().to_string(),
        "IntRange <r: [10 : 20]>"
    );
    assert_eq!(
        IntRange::new("r", &[10, -5], &[20, 5]).unwrap().to_string(),
        "IntRange <r: [10 : 20] [-5 : 5]>"
    );
    assert_eq!(
        LongRange::new("r", &[10], &[20]).unwrap().to_string(),
        "LongRange <r: [10 : 20]>"
    );
    assert_eq!(
        LongRange::new("r", &[10, -5], &[20, 5])
            .unwrap()
            .to_string(),
        "LongRange <r: [10 : 20] [-5 : 5]>"
    );
    assert_eq!(
        FloatRange::new("r", &[1.5], &[2.5]).unwrap().to_string(),
        "FloatRange <r: [1.5 : 2.5]>"
    );
    assert_eq!(
        FloatRange::new("r", &[1.5, -1e-8], &[2.5, 1e8])
            .unwrap()
            .to_string(),
        "FloatRange <r: [1.5 : 2.5] [-1.0E-8 : 1.0E8]>"
    );
    assert_eq!(
        DoubleRange::new("r", &[1.5], &[2.5]).unwrap().to_string(),
        "DoubleRange <r: [1.5 : 2.5]>"
    );
    assert_eq!(
        DoubleRange::new("r", &[1.5, -1e-8], &[2.5, 1e8])
            .unwrap()
            .to_string(),
        "DoubleRange <r: [1.5 : 2.5] [-1.0E-8 : 1.0E8]>"
    );
}

#[test]
fn range_field_query_to_string_matches_java() {
    let q = IntRange::new_intersects_query("r", &[10], &[20]).unwrap();
    assert_eq!(Query::to_query_string(&q, "r"), "<ranges:[10 : 20]>");
    assert_eq!(Query::to_query_string(&q, "other"), "r:<ranges:[10 : 20]>");

    let two_dims = IntRange::new_within_query("r", &[10, -5], &[20, 5]).unwrap();
    assert_eq!(
        Query::to_query_string(&two_dims, "r"),
        "<ranges:[10 : 20] [-5 : 5]>"
    );

    let long_query = LongRange::new_contains_query("r", &[10], &[20]).unwrap();
    assert_eq!(
        Query::to_query_string(&long_query, "r"),
        "<ranges:[10 : 20]>"
    );

    let float_query = FloatRange::new_crosses_query("r", &[1.5], &[2.5]).unwrap();
    assert_eq!(
        Query::to_query_string(&float_query, "r"),
        "<ranges:[1.5 : 2.5]>"
    );

    let double_query = DoubleRange::new_intersects_query("r", &[1.5], &[2.5]).unwrap();
    assert_eq!(
        Query::to_query_string(&double_query, "r"),
        "<ranges:[1.5 : 2.5]>"
    );

    // Java renders the two bounds through `InetAddress.toString()`, which is
    // `hostname + "/" + hostAddress` and whose host name is always empty for an
    // address decoded from bytes.
    let inet = InetAddressRange::new_intersects_query("r", &[1, 2, 3, 4], &[1, 2, 3, 10]).unwrap();
    assert_eq!(
        Query::to_query_string(&inet, "r"),
        "<ranges:[/1.2.3.4 : /1.2.3.10]>"
    );
    let mut lo = [0u8; 16];
    lo[15] = 1;
    let mut hi = [0u8; 16];
    hi[15] = 9;
    let inet6 = InetAddressRange::new_intersects_query("r", &lo, &hi).unwrap();
    assert_eq!(
        Query::to_query_string(&inet6, "r"),
        "<ranges:[/0:0:0:0:0:0:0:1 : /0:0:0:0:0:0:0:9]>"
    );
}

#[test]
fn range_field_query_equality_separates_value_types() {
    // Measured from the jar: each `XxxRange` builds one anonymous
    // `RangeFieldQuery` subclass shared by its four relations, so
    // `Query.sameClassAs` makes two relations of `IntRange` comparable and an
    // `IntRange` query never equal to a `FloatRange` one.
    let a = IntRange::new_intersects_query("r", &[10], &[20]).unwrap();
    let b = IntRange::new_intersects_query("r", &[10], &[20]).unwrap();
    let other_relation = IntRange::new_within_query("r", &[10], &[20]).unwrap();
    assert!(a.query_eq(&b));
    assert_eq!(a.query_hash(), b.query_hash());
    assert!(!a.query_eq(&other_relation));

    // Two range queries whose encoded bytes are identical but whose value types
    // differ: `IntRange` [10, 20] and the `FloatRange` over the floats those
    // sortable ints decode to.
    let min = FloatRange::decode_min(a.get_ranges(), 0);
    let max = FloatRange::decode_max(a.get_ranges(), 0);
    let float_twin = FloatRange::new_intersects_query("r", &[min], &[max]).unwrap();
    assert_eq!(
        float_twin.get_ranges(),
        a.get_ranges(),
        "the two queries must carry the same bytes for the test to mean anything"
    );
    assert!(!a.query_eq(&float_twin));
    assert_ne!(a.query_hash(), float_twin.query_hash());
}

// -----------------------------------------------------------------------------
// document: FieldType and Field
// -----------------------------------------------------------------------------

/// Builds the configuration of `StoredField.TYPE`.
///
/// The Java fixture verifies that this setter sequence produces a `FieldType`
/// that `equals` the constant, so comparing against it is comparing against
/// Lucene's own.
fn stored_type() -> FieldType {
    let mut ft = FieldType::new();
    ft.set_stored(true).unwrap();
    ft
}

/// Builds the configuration of `TextField.TYPE_NOT_STORED`.
fn text_type() -> FieldType {
    let mut ft = FieldType::new();
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    ft.set_tokenized(true).unwrap();
    ft
}

/// Builds the configuration of `TextField.TYPE_STORED`.
fn text_stored_type() -> FieldType {
    let mut ft = text_type();
    ft.set_stored(true).unwrap();
    ft
}

/// Builds the configuration of `StringField.TYPE_NOT_STORED`.
fn string_type() -> FieldType {
    let mut ft = FieldType::new();
    ft.set_index_options(IndexOptions::DOCS).unwrap();
    ft.set_tokenized(false).unwrap();
    ft.set_omit_norms(true).unwrap();
    ft
}

#[test]
fn field_type_to_string_matches_java() {
    assert_eq!(FieldType::new().to_string(), "");
    assert_eq!(stored_type().to_string(), "stored");
    assert_eq!(text_type().to_string(), "indexed,tokenized");
    assert_eq!(text_stored_type().to_string(), "stored,indexed,tokenized");
    assert_eq!(
        string_type().to_string(),
        "indexed,omitNorms,indexOptions=DOCS"
    );

    let mut vectors = text_stored_type();
    vectors.set_store_term_vectors(true).unwrap();
    vectors.set_store_term_vector_offsets(true).unwrap();
    vectors.set_store_term_vector_positions(true).unwrap();
    vectors.set_store_term_vector_payloads(true).unwrap();
    vectors.set_omit_norms(true).unwrap();
    assert_eq!(
        vectors.to_string(),
        "stored,indexed,tokenized,termVector,termVectorOffsets,termVectorPosition,\
         termVectorPayloads,omitNorms"
    );

    let mut points = FieldType::new();
    points.set_index_options(IndexOptions::DOCS).unwrap();
    points.set_tokenized(false).unwrap();
    points
        .set_doc_values_type(DocValuesType::SORTED_SET)
        .unwrap();
    points.set_dimensions(2, 4).unwrap();
    assert_eq!(
        points.to_string(),
        "indexed,indexOptions=DOCS,pointDimensionCount=2,pointIndexDimensionCount=2,\
         pointNumBytes=4,docValuesType=SORTED_SET"
    );
}

#[test]
fn field_to_string_matches_java() {
    let string_field = Field::new("f", "hello".to_string(), text_stored_type()).unwrap();
    assert_eq!(
        string_field.to_string(),
        "stored,indexed,tokenized<f:hello>"
    );

    let int_field = Field::new_with_number("f", NumericValue::Int(42), stored_type()).unwrap();
    assert_eq!(int_field.to_string(), "stored<f:42>");

    // `BytesRef.toString` uses `Integer.toHexString`, which does not pad: the
    // bytes 0x0a, 0xff, 0x00 print as `a ff 0`.
    let bytes_field =
        Field::new_with_bytes("f", BytesRef::new(vec![0x0a, 0xff, 0x00]), stored_type()).unwrap();
    assert_eq!(bytes_field.to_string(), "stored<f:[a ff 0]>");
}

#[test]
fn field_type_equality_and_hash_agree_with_java() {
    let a = text_stored_type();
    let b = text_stored_type();
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));

    // Java's `equals` ignores the frozen flag.
    let mut frozen = text_stored_type();
    frozen.freeze();
    assert_eq!(frozen, b);
    assert_eq!(hash_of(&frozen), hash_of(&b));

    // Java's `equals` and `hashCode` ignore the vector attributes.
    let mut vectors = text_stored_type();
    vectors
        .set_vector_attributes(
            4,
            rucene::index::VectorEncoding::FLOAT32,
            rucene::index::VectorSimilarityFunction::COSINE,
        )
        .unwrap();
    assert_eq!(vectors, b);
    assert_eq!(hash_of(&vectors), hash_of(&b));

    // ... and the attribute map.
    let mut attributes = text_stored_type();
    attributes
        .put_attribute("k".to_string(), "v".to_string())
        .unwrap();
    assert_eq!(attributes, b);
    assert_eq!(hash_of(&attributes), hash_of(&b));

    // A property `equals` does compare.
    let mut tokenized = text_stored_type();
    tokenized.set_tokenized(false).unwrap();
    assert_ne!(tokenized, b);
    assert_ne!(hash_of(&tokenized), hash_of(&b));
}

// -----------------------------------------------------------------------------
// document: DocValuesLongHashSet
// -----------------------------------------------------------------------------

#[test]
fn doc_values_long_hash_set_to_string_matches_java() {
    // The order is the hash table's layout, not sorted and not insertion order,
    // so this also proves the port builds the same table as Java.
    assert_eq!(
        DocValuesLongHashSet::new(&[1, 2, 3]).to_string(),
        "[1, 2, 3]"
    );
    assert_eq!(DocValuesLongHashSet::new(&[]).to_string(), "[]");
    assert_eq!(
        DocValuesLongHashSet::new(&[i64::MIN, 5]).to_string(),
        "[-9223372036854775808, 5]"
    );
    assert_eq!(
        DocValuesLongHashSet::new(&[-100, 0, 7, 42, 1000]).to_string(),
        "[0, 1000, 42, -100, 7]"
    );
}

// -----------------------------------------------------------------------------
// search: FilterCollector and FilterLeafCollector
// -----------------------------------------------------------------------------

/// A collector with a stable rendering, so that the wrapper's is decidable.
///
/// Java's fixture uses the same device: `Object.toString()` prints an identity
/// hash code, which no port can reproduce and which changes between JVM runs,
/// so the reference case wraps a collector whose `toString` returns `NOOP`.
#[derive(Debug, Default)]
struct NoopCollector;

impl std::fmt::Display for NoopCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NOOP")
    }
}

impl Collector for NoopCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        _context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        Ok(Box::new(NoopLeafCollector))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

/// A leaf collector with a stable rendering, matching the Java fixture's.
#[derive(Debug, Default, Clone, Copy)]
struct NoopLeafCollector;

impl std::fmt::Display for NoopLeafCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NOOP")
    }
}

impl LeafCollector for NoopLeafCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> rucene::error::Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

#[test]
fn filter_collector_to_string_matches_java() {
    // Java prints `getClass().getSimpleName() + "(" + in + ")"`. Measured
    // through a *named* subclass, since Java's class is abstract and an
    // anonymous subclass has an empty simple name:
    //   NamedFilterCollector(NOOP)
    //   NamedFilterLeafCollector(NOOP)
    // This port's types are concrete, so the runtime simple name is always
    // `FilterCollector` / `FilterLeafCollector`.
    assert_eq!(
        FilterCollector::new(NoopCollector).to_string(),
        "FilterCollector(NOOP)"
    );
    // `FilterLeafCollector` additionally substitutes its own name for an empty
    // one, which the Java fixture confirms: an anonymous subclass prints
    // `FilterLeafCollector(NOOP)` — the same text this port produces.
    assert_eq!(
        FilterLeafCollector::new(NoopLeafCollector).to_string(),
        "FilterLeafCollector(NOOP)"
    );
}

// -----------------------------------------------------------------------------
// document: SpatialQuery hash / equality consistency
// -----------------------------------------------------------------------------

#[test]
fn lat_lon_shape_bounding_box_query_hash_agrees_with_equality() {
    let rect = Rectangle::new(-10.0, 10.0, -20.0, 20.0).unwrap();
    let a =
        LatLonShapeBoundingBoxQuery::new("shape", QueryRelation::Intersects, rect.clone()).unwrap();
    let b =
        LatLonShapeBoundingBoxQuery::new("shape", QueryRelation::Intersects, rect.clone()).unwrap();
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));

    // Java's `equalsTo` chains into `SpatialQuery.equalsTo`, which compares the
    // field and the relation, and its `hashCode` folds the rectangle into the
    // superclass hash — so each of the three has to move the hash.
    let other_field =
        LatLonShapeBoundingBoxQuery::new("other", QueryRelation::Intersects, rect.clone()).unwrap();
    assert_ne!(a, other_field);
    assert_ne!(hash_of(&a), hash_of(&other_field));

    let other_relation =
        LatLonShapeBoundingBoxQuery::new("shape", QueryRelation::Within, rect.clone()).unwrap();
    assert_ne!(a, other_relation);
    assert_ne!(hash_of(&a), hash_of(&other_relation));

    let other_rect = LatLonShapeBoundingBoxQuery::new(
        "shape",
        QueryRelation::Intersects,
        Rectangle::new(-10.0, 10.0, -20.0, 21.0).unwrap(),
    )
    .unwrap();
    assert_ne!(a, other_rect);
    assert_ne!(hash_of(&a), hash_of(&other_rect));
}

// -----------------------------------------------------------------------------
// search: the `Iterable` implementations
// -----------------------------------------------------------------------------

#[test]
fn boolean_query_iterates_its_clauses() {
    // Java: `BooleanQuery implements Iterable<BooleanClause>`, and
    // `iterator()` returns `clauses.iterator()` — insertion order, duplicates
    // kept.
    let mut builder = BooleanQueryBuilder::new();
    builder
        .add(
            Arc::new(TermQuery::new(Term::new("f", BytesRef::new(b"a".to_vec()))))
                as Arc<dyn Query>,
            Occur::MUST,
        )
        .unwrap();
    builder
        .add(
            Arc::new(TermQuery::new(Term::new("f", BytesRef::new(b"b".to_vec()))))
                as Arc<dyn Query>,
            Occur::SHOULD,
        )
        .unwrap();
    let query = builder.build();

    let occurs: Vec<Occur> = (&query).into_iter().map(|c| c.occur()).collect();
    assert_eq!(occurs, vec![Occur::MUST, Occur::SHOULD]);
    assert_eq!((&query).into_iter().count(), query.clauses().len());
}

#[test]
fn disjunction_max_and_log_odds_fusion_iterate_their_queries() {
    let a: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new("f", BytesRef::new(b"a".to_vec()))));
    let b: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new("f", BytesRef::new(b"b".to_vec()))));

    // Java: `DisjunctionMaxQuery.iterator()` returns `getDisjuncts().iterator()`.
    let dis_max = DisjunctionMaxQuery::new(vec![Arc::clone(&a), Arc::clone(&b)], 0.5).unwrap();
    let seen: Vec<String> = (&dis_max)
        .into_iter()
        .map(|q| Query::to_query_string(q.as_ref(), ""))
        .collect();
    assert_eq!(
        seen,
        dis_max
            .get_disjuncts()
            .iter()
            .map(|q| Query::to_query_string(q.as_ref(), ""))
            .collect::<Vec<_>>()
    );
    assert_eq!(seen.len(), 2);

    // Java: `LogOddsFusionQuery.iterator()` returns `getClauses().iterator()`.
    let fusion = LogOddsFusionQuery::with_clauses(vec![Arc::clone(&a), Arc::clone(&b)]).unwrap();
    assert_eq!((&fusion).into_iter().count(), fusion.get_clauses().len());
    assert_eq!((&fusion).into_iter().count(), 2);
}
