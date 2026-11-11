//! Geo points and shapes against Apache Lucene Core 10.5.0 (rmp task #408).
//!
//! The geo corpus is defined by the same formulas here and in
//! `GeoShapeFixture.java`: per document a `LatLonPoint` and a
//! `LatLonDocValuesField` on `loc` and an `XYPointField` on `xy` (absent from
//! every seventeenth document, so the distance sort meets missing values), a
//! `LatLonShape` on `shape` and an `XYShape` on `xyshape` (a polygon — with a
//! hole on every ninth document —, a line or a point). Rucene writes it under
//! `R`, Lucene under `L`.
//!
//! * The query set (`GeoShapeFixture.query`, [`query_report`]) runs on both
//!   indexes from both sides: `LatLonPoint` distance, box (one crossing the
//!   dateline), polygon-with-hole and geometry queries (every
//!   `QueryRelation`), `LatLonDocValuesField` slow distance and box queries,
//!   `XYPointField` box, distance and polygon queries, `LatLonShape` and
//!   `XYShape` box and geometry queries for `INTERSECTS`, `WITHIN`,
//!   `DISJOINT` and `CONTAINS`, and the `LatLonDocValuesField.newDistanceSort`
//!   over every document. Per hit: doc id, score bits (`floatToIntBits`), sort
//!   value bits (`doubleToLongBits`) and id. The four reports are compared
//!   line by line.
//! * Triangle encodings: the packed bytes of every `ShapeField.Triangle`
//!   `LatLonShape`/`XYShape.createIndexableFields` produce for the corpus
//!   shapes are printed by both sides (`GeoShapeFixture triangles`,
//!   [`triangles_report`]) and compared. They are a pure function of the
//!   shape (tessellation plus `ShapeField.encodeTriangle`), with no header or
//!   random input, so they are byte-deterministic on both sides.
//! * Shape index bytes: both sides write the corpus as a single segment (one
//!   commit, no compound file, a RAM buffer large enough that only the commit
//!   flushes), so the points files `.kdd`, `.kdi` and `.kdm` — which hold
//!   every point and triangle of the corpus — come from the same documents in
//!   the same order through the same BKD writer. The only non-deterministic
//!   bytes are the random 16-byte segment id, which every
//!   `CodecUtil.writeIndexHeader` embeds, and the CRC32 footer that covers it:
//!   [`masked`] zeroes every occurrence of the segment id (read from the
//!   file's own header) and the 8-byte checksum, and the masked files are
//!   compared byte for byte.
//! * Positive control: Rucene writes a perturbed corpus (document 0 moved one
//!   degree north); its report must differ from Lucene's report on `L` with
//!   the same document counts, and its masked points files must differ from
//!   `L`'s.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::geo_fields::{LatLonDocValuesField, LatLonPoint, XYPointField};
use rucene::document::shape_field::{QueryRelation, Triangle};
use rucene::document::{
    Document, LatLonGeometryValue, LatLonShape, Store, StringField, XYGeometryValue, XYShape,
};
use rucene::geo::{
    Circle, Line, Point, Polygon, Rectangle, XYLine, XYPoint, XYPolygon, XYRectangle,
};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexReader, StoredFields};
use rucene::search::{
    FieldDoc, IndexSearcher, MatchAllDocsQuery, Query, ScoreDoc, Sort, SortValue, TotalHits,
    TotalHitsRelation,
};
use rucene::store::{Directory, FSDirectory};

// ---------------------------------------------------------------------------
// Geo corpus: the formulas of GeoShapeFixture
// ---------------------------------------------------------------------------

/// `GeoShapeFixture.DOCS`.
const DOCS: i32 = 200;

/// `GeoShapeFixture.TOP_N`.
const TOP_N: i32 = 20;

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

/// `GeoShapeFixture.lat(int)`; the perturbed corpus moves document 0 one
/// degree north.
fn lat(i: i32, perturbed: bool) -> f64 {
    let base = -80.0 + ((i * 37) % 1600) as f64 / 10.0;
    if perturbed && i == 0 {
        base + 1.0
    } else {
        base
    }
}

fn lon(i: i32) -> f64 {
    -170.0 + ((i * 53) % 3400) as f64 / 10.0
}

fn x(i: i32) -> f32 {
    ((i * 29) % 1000) as f32 / 10.0 - 50.0
}

fn y(i: i32) -> f32 {
    ((i * 43) % 1000) as f32 / 10.0 - 50.0
}

/// `GeoShapeFixture.hasLocation(int)`.
fn has_location(i: i32) -> bool {
    i % 17 != 5
}

fn size(i: i32) -> f64 {
    0.5 + (i % 5) as f64 * 0.25
}

fn xy_size(i: i32) -> f32 {
    0.5 + (i % 5) as f32 * 0.25
}

/// `GeoShapeFixture.latLonShape(int)`.
fn lat_lon_shape(i: i32, perturbed: bool) -> Vec<Triangle> {
    let a = lat(i, perturbed);
    let o = lon(i);
    let d = size(i);
    match i % 3 {
        0 => {
            let mut holes = Vec::new();
            if i % 9 == 0 {
                let h = d / 4.0;
                holes.push(
                    Polygon::new(
                        vec![a - h, a + h, a + h, a - h, a - h],
                        vec![o - h, o - h, o + h, o + h, o - h],
                        Vec::new(),
                    )
                    .unwrap(),
                );
            }
            let polygon = Polygon::new(
                vec![a - d, a - d, a + d, a + d, a - d],
                vec![o - d, o + d, o + d, o - d, o - d],
                holes,
            )
            .unwrap();
            LatLonShape::create_indexable_fields_polygon("shape", &polygon).unwrap()
        }
        1 => {
            let line = Line::new(vec![a, a + d, a + 2.0 * d], vec![o, o + d, o - d]).unwrap();
            LatLonShape::create_indexable_fields_line("shape", &line).unwrap()
        }
        _ => LatLonShape::create_indexable_fields_point("shape", a, o).unwrap(),
    }
}

/// `GeoShapeFixture.xyShape(int)`.
fn xy_shape(i: i32) -> Vec<Triangle> {
    let a = x(i);
    let o = y(i);
    let d = xy_size(i);
    match i % 3 {
        0 => {
            let mut holes = Vec::new();
            if i % 9 == 0 {
                let h = d / 4.0;
                holes.push(
                    XYPolygon::new(
                        vec![a - h, a + h, a + h, a - h, a - h],
                        vec![o - h, o - h, o + h, o + h, o - h],
                        Vec::new(),
                    )
                    .unwrap(),
                );
            }
            let polygon = XYPolygon::new(
                vec![a - d, a - d, a + d, a + d, a - d],
                vec![o - d, o + d, o + d, o - d, o - d],
                holes,
            )
            .unwrap();
            XYShape::create_indexable_fields_polygon("xyshape", &polygon).unwrap()
        }
        1 => {
            let line = XYLine::new(vec![a, a + d, a + 2.0 * d], vec![o, o + d, o - d]).unwrap();
            XYShape::create_indexable_fields_line("xyshape", &line).unwrap()
        }
        _ => XYShape::create_indexable_fields_point("xyshape", a, o).unwrap(),
    }
}

/// `GeoShapeFixture.doc(int)`.
fn doc(i: i32, perturbed: bool) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    if has_location(i) {
        let a = lat(i, perturbed);
        d.add(Box::new(LatLonPoint::new("loc", a, lon(i)).unwrap()));
        d.add(Box::new(
            LatLonDocValuesField::new("loc", a, lon(i)).unwrap(),
        ));
        d.add(Box::new(XYPointField::new("xy", x(i), y(i)).unwrap()));
    }
    for t in lat_lon_shape(i, perturbed) {
        d.add(Box::new(t));
    }
    for t in xy_shape(i) {
        d.add(Box::new(t));
    }
    d
}

/// `GeoShapeFixture.write(Directory)`: one commit, no compound file.
fn write_corpus(path: &Path, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    std::fs::create_dir_all(path).unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    config.set_use_compound_file(false).unwrap();
    config.set_ram_buffer_size_mb(256.0).unwrap();
    let w = IndexWriter::new(dir, config).unwrap();
    for i in 0..DOCS {
        w.add_document(&doc(i, perturbed)).unwrap();
    }
    w.commit().unwrap();
    w.close().unwrap();
}

// ---------------------------------------------------------------------------
// Query set: the output of GeoShapeFixture.query, line for line
// ---------------------------------------------------------------------------

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

/// `GeoShapeFixture.queryPolygon()`.
fn query_polygon() -> Polygon {
    Polygon::new(
        vec![-50.0, -50.0, 50.0, 50.0, -50.0],
        vec![-100.0, 100.0, 100.0, -100.0, -100.0],
        vec![Polygon::new(
            vec![-10.0, 10.0, 10.0, -10.0, -10.0],
            vec![-20.0, -20.0, 20.0, 20.0, -20.0],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap()
}

/// `GeoShapeFixture.queryXYPolygon()`.
fn query_xy_polygon() -> XYPolygon {
    XYPolygon::new(
        vec![-40.0, 40.0, 40.0, -40.0, -40.0],
        vec![-40.0, -40.0, 40.0, 40.0, -40.0],
        vec![XYPolygon::new(
            vec![-5.0, -5.0, 5.0, 5.0, -5.0],
            vec![-5.0, 5.0, 5.0, -5.0, -5.0],
            Vec::new(),
        )
        .unwrap()],
    )
    .unwrap()
}

/// `GeoShapeFixture.RELATIONS`, with the names `QueryRelation.toString()`
/// prints.
const RELATIONS: [(QueryRelation, &str); 4] = [
    (QueryRelation::Intersects, "INTERSECTS"),
    (QueryRelation::Within, "WITHIN"),
    (QueryRelation::Disjoint, "DISJOINT"),
    (QueryRelation::Contains, "CONTAINS"),
];

fn poly(p: Polygon) -> Vec<LatLonGeometryValue> {
    vec![LatLonGeometryValue::Polygon(p)]
}

fn rect(min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> LatLonGeometryValue {
    LatLonGeometryValue::Rectangle(Rectangle::new(min_lat, max_lat, min_lon, max_lon).unwrap())
}

/// `GeoShapeFixture.queries()`, in the same order.
fn queries() -> Vec<(String, Arc<dyn Query>)> {
    let loc = Some("loc");
    let xy = Some("xy");
    let mut q: Vec<(String, Arc<dyn Query>)> = vec![
        (
            "latlon distance (-5,-60) 2500km".into(),
            LatLonPoint::new_distance_query(loc, -5.0, -60.0, 2_500_000.0).unwrap(),
        ),
        (
            "latlon distance (-40,-100) 3500km".into(),
            LatLonPoint::new_distance_query(loc, -40.0, -100.0, 3_500_000.0).unwrap(),
        ),
        (
            "latlon box [-30,30]x[-60,60]".into(),
            LatLonPoint::new_box_query(loc, -30.0, 30.0, -60.0, 60.0).unwrap(),
        ),
        (
            "latlon box dateline [-60,60]x[150,-150]".into(),
            LatLonPoint::new_box_query(loc, -60.0, 60.0, 150.0, -150.0).unwrap(),
        ),
        (
            "latlon polygon with hole".into(),
            LatLonPoint::new_polygon_query(loc, poly(query_polygon())).unwrap(),
        ),
        (
            "latlon geometry INTERSECTS circle+box".into(),
            LatLonPoint::new_geometry_query(
                loc,
                QueryRelation::Intersects,
                vec![
                    LatLonGeometryValue::Circle(Circle::new(-5.0, -60.0, 2_500_000.0).unwrap()),
                    rect(-30.0, 30.0, -60.0, 60.0),
                ],
            )
            .unwrap(),
        ),
        (
            "latlon geometry WITHIN polygon".into(),
            LatLonPoint::new_geometry_query(loc, QueryRelation::Within, poly(query_polygon()))
                .unwrap(),
        ),
        (
            "latlon geometry DISJOINT box".into(),
            LatLonPoint::new_geometry_query(
                loc,
                QueryRelation::Disjoint,
                vec![rect(-30.0, 30.0, -60.0, 60.0)],
            )
            .unwrap(),
        ),
        (
            "latlon geometry CONTAINS point of doc 3".into(),
            LatLonPoint::new_geometry_query(
                loc,
                QueryRelation::Contains,
                vec![LatLonGeometryValue::Point(
                    Point::new(lat(3, false), lon(3)).unwrap(),
                )],
            )
            .unwrap(),
        ),
        (
            "latlon dv slowDistance (-5,-60) 2500km".into(),
            LatLonDocValuesField::new_slow_distance_query(loc, -5.0, -60.0, 2_500_000.0).unwrap(),
        ),
        (
            "latlon dv slowBox [-30,30]x[-60,60]".into(),
            LatLonDocValuesField::new_slow_box_query(loc, -30.0, 30.0, -60.0, 60.0).unwrap(),
        ),
        (
            "latlon dv slowBox dateline [-60,60]x[150,-150]".into(),
            LatLonDocValuesField::new_slow_box_query(loc, -60.0, 60.0, 150.0, -150.0).unwrap(),
        ),
        (
            "xy box [-20,20]x[-20,20]".into(),
            XYPointField::new_box_query(xy, -20.0, 20.0, -20.0, 20.0).unwrap(),
        ),
        (
            "xy distance (0,0) 25".into(),
            XYPointField::new_distance_query(xy, 0.0, 0.0, 25.0).unwrap(),
        ),
        (
            "xy polygon with hole".into(),
            XYPointField::new_polygon_query(xy, vec![query_xy_polygon()]).unwrap(),
        ),
    ];
    // The centre of document 3's polygon: the CONTAINS queries need a query
    // shape a document shape can enclose.
    let c_lat = lat(3, false);
    let c_lon = lon(3);
    let c_x = x(3);
    let c_y = y(3);
    for (r, name) in RELATIONS {
        let contains = r == QueryRelation::Contains;
        let shape_box = if contains {
            LatLonShape::new_box_query(
                "shape",
                r,
                c_lat - 0.01,
                c_lat + 0.01,
                c_lon - 0.01,
                c_lon + 0.01,
            )
        } else {
            LatLonShape::new_box_query("shape", r, -30.0, 30.0, -60.0, 60.0)
        };
        q.push((
            format!("latlonshape box {name}"),
            shape_box.unwrap().into_query().unwrap(),
        ));
        let shape_geometry = if contains {
            LatLonShape::new_geometry_query(
                "shape",
                r,
                vec![LatLonGeometryValue::Point(
                    Point::new(c_lat, c_lon).unwrap(),
                )],
            )
        } else {
            LatLonShape::new_geometry_query("shape", r, poly(query_polygon()))
        };
        q.push((
            format!("latlonshape geometry {name}"),
            shape_geometry.unwrap().into_query().unwrap(),
        ));
        let xy_box = if contains {
            XYShape::new_box_query("xyshape", r, c_x - 0.01, c_x + 0.01, c_y - 0.01, c_y + 0.01)
        } else {
            XYShape::new_box_query("xyshape", r, -20.0, 20.0, -20.0, 20.0)
        };
        q.push((
            format!("xyshape box {name}"),
            xy_box.unwrap().into_query().unwrap(),
        ));
        let xy_geometry = if contains {
            XYShape::new_geometry_query(
                "xyshape",
                r,
                vec![XYGeometryValue::Point(XYPoint::new(c_x, c_y).unwrap())],
            )
        } else {
            XYShape::new_geometry_query(
                "xyshape",
                r,
                vec![XYGeometryValue::Polygon(query_xy_polygon())],
            )
        };
        q.push((
            format!("xyshape geometry {name}"),
            xy_geometry.unwrap().into_query().unwrap(),
        ));
    }
    q.push((
        "latlonshape geometry INTERSECTS circle+line".into(),
        LatLonShape::new_geometry_query(
            "shape",
            QueryRelation::Intersects,
            vec![
                LatLonGeometryValue::Circle(Circle::new(-5.0, -60.0, 2_500_000.0).unwrap()),
                LatLonGeometryValue::Line(
                    Line::new(vec![-20.0, 0.0, 20.0], vec![100.0, 120.0, 100.0]).unwrap(),
                ),
            ],
        )
        .unwrap()
        .into_query()
        .unwrap(),
    ));
    q.push((
        "xyshape geometry INTERSECTS box+line".into(),
        XYShape::new_geometry_query(
            "xyshape",
            QueryRelation::Intersects,
            vec![
                XYGeometryValue::Rectangle(XYRectangle::new(-10.0, 10.0, 20.0, 30.0).unwrap()),
                XYGeometryValue::Line(
                    XYLine::new(vec![-45.0, 0.0, 45.0], vec![-45.0, 10.0, -45.0]).unwrap(),
                ),
            ],
        )
        .unwrap()
        .into_query()
        .unwrap(),
    ));
    q
}

/// `GeoShapeFixture.printHits`: `sort` is set for a field-sorted hit.
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

/// The sort value `GeoShapeFixture.printHits` prints for a field-sorted hit.
fn sort_value(fd: &FieldDoc) -> String {
    match &fd.fields.as_ref().expect("sort values")[0] {
        SortValue::Double(v) => format!("Double:{}", bits64(*v)),
        SortValue::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}

/// Rucene's run of the query set over the index at `path`: the same lines
/// `GeoShapeFixture query` prints.
fn query_report(path: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    let stored = stored.as_ref();
    let mut out = String::new();
    writeln!(
        out,
        "reader maxDoc={} numDocs={}",
        reader.max_doc(),
        reader.num_docs()
    )
    .unwrap();
    for ctx in &Arc::clone(&reader).leaves().unwrap() {
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
        let hits: Vec<(ScoreDoc, Option<String>)> =
            top.score_docs.iter().map(|h| (*h, None)).collect();
        print_hits(
            &mut out,
            &format!("query {name}"),
            &top.total_hits,
            &hits,
            stored,
        );
    }
    let sort = Sort::new_fields(vec![LatLonDocValuesField::new_distance_sort(
        "loc", -5.0, -60.0,
    )
    .unwrap()
    .sort_field()
    .clone()])
    .unwrap();
    let sorted = searcher
        .search_with_sort(Arc::new(MatchAllDocsQuery::instance()), DOCS, &sort)
        .unwrap();
    let hits: Vec<(ScoreDoc, Option<String>)> = sorted
        .score_docs
        .iter()
        .map(|fd| (fd.score_doc, Some(sort_value(fd))))
        .collect();
    print_hits(
        &mut out,
        "sort latlon distance from (-5,-60)",
        &sorted.total_hits,
        &hits,
        stored,
    );
    out
}

// ---------------------------------------------------------------------------
// Triangle encodings: the output of GeoShapeFixture.triangles
// ---------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn print_triangles(out: &mut String, kind: &str, i: i32, triangles: &[Triangle]) {
    writeln!(out, "{kind} doc={i} triangles={}", triangles.len()).unwrap();
    for t in triangles {
        writeln!(
            out,
            "  {}",
            hex(t.binary_value().expect("packed triangle").slice())
        )
        .unwrap();
    }
}

/// Rucene's `GeoShapeFixture triangles`.
fn triangles_report() -> String {
    let mut out = String::new();
    for i in 0..DOCS {
        print_triangles(&mut out, "latlonshape", i, &lat_lon_shape(i, false));
        print_triangles(&mut out, "xyshape", i, &xy_shape(i));
    }
    out
}

// ---------------------------------------------------------------------------
// Points files
// ---------------------------------------------------------------------------

/// `CodecUtil.CODEC_MAGIC`.
const CODEC_MAGIC: [u8; 4] = [0x3f, 0xd7, 0x6c, 0x17];

/// Reads a Lucene `vInt` at the start of `bytes`: `(value, length)`.
fn read_vint(bytes: &[u8]) -> (usize, usize) {
    let mut value = 0usize;
    for (k, b) in bytes.iter().enumerate().take(5) {
        value |= ((b & 0x7f) as usize) << (7 * k);
        if b & 0x80 == 0 {
            return (value, k + 1);
        }
    }
    panic!("malformed vInt in a codec header");
}

/// A codec file with its non-deterministic bytes zeroed: every occurrence of
/// the 16-byte segment id the file's index header carries
/// (`CodecUtil.writeIndexHeader`: magic, codec name as a `vInt`-prefixed
/// string, version, id) and the 8-byte CRC32 of the footer, which covers it.
fn masked(bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    assert!(
        b.len() > 4 + 16 + 16 && b[0..4] == CODEC_MAGIC,
        "not a codec file"
    );
    let (name_len, vint_len) = read_vint(&b[4..]);
    let id_at = 4 + vint_len + name_len + 4;
    let id = b[id_at..id_at + 16].to_vec();
    let mut k = 0;
    while k + 16 <= b.len() {
        if b[k..k + 16] == id[..] {
            b[k..k + 16].fill(0);
            k += 16;
        } else {
            k += 1;
        }
    }
    let n = b.len();
    b[n - 8..].fill(0);
    b
}

/// The masked points files (`.kdd`, `.kdi`, `.kdm`) of the index at `path`,
/// by file name.
fn points_files(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == "kdd" || e == "kdi" || e == "kdm")
        })
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                masked(&std::fs::read(&p).unwrap()),
            )
        })
        .collect();
    files.sort();
    files
}

/// The differences between two sets of masked points files, or `None`.
fn points_diff(
    left_name: &str,
    left: &[(String, Vec<u8>)],
    right_name: &str,
    right: &[(String, Vec<u8>)],
) -> Option<String> {
    let left_names: Vec<&String> = left.iter().map(|(n, _)| n).collect();
    let right_names: Vec<&String> = right.iter().map(|(n, _)| n).collect();
    if left_names != right_names {
        return Some(format!(
            "points files of {left_name} {left_names:?} differ from those of {right_name} {right_names:?}"
        ));
    }
    let mut out = String::new();
    for ((name, a), (_, b)) in left.iter().zip(right) {
        if a != b {
            let first = a
                .iter()
                .zip(b.iter())
                .position(|(x, y)| x != y)
                .unwrap_or(a.len().min(b.len()));
            writeln!(
                out,
                "  {name}: {left_name} {} bytes, {right_name} {} bytes, first difference at offset {first}",
                a.len(),
                b.len()
            )
            .unwrap();
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(format!(
            "masked points files of {left_name} differ from {right_name}:\n{out}"
        ))
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

/// Runs `GeoShapeFixture <mode> [<dir>]`: `(success, stdout, stderr)`.
fn run_java(mode: &str, dir: Option<&Path>) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         GeoShapeFixture.java",
    );
    let mut command = Command::new("java");
    command.arg("-cp").arg(lucene_jar()).arg(fixture).arg(mode);
    if let Some(dir) = dir {
        command.arg(dir);
    }
    let output = command
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

/// The `reader` lines of a report: the document counts of the index.
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
fn geo_points_and_shapes_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-geo-shapes")
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
    let (ok, _, err) = run_java("write", Some(&l));
    assert!(ok, "Lucene failed to write the corpus:\n{err}");

    // The query set, from both sides over both indexes.
    let mut java_query = |dir: &Path, name: &str| -> Option<String> {
        let (ok, out, err) = run_java("query", Some(dir));
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

    // Every query of the oracle must have hits, and the sort must list every
    // document: an empty result would make the comparisons vacuous.
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
        queries().len() + 1,
        "the oracle must report every query and the sort"
    );
    assert_eq!(
        oracle
            .lines()
            .filter(|l| l.contains(" sort=Double:"))
            .count(),
        DOCS as usize,
        "the distance sort must list every document"
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

    // Triangle encodings.
    let (ok, java_triangles, err) = run_java("triangles", None);
    if ok {
        assert!(
            java_triangles
                .lines()
                .filter(|l| l.starts_with("  "))
                .count()
                > DOCS as usize,
            "the oracle must print the corpus triangles"
        );
        let rust_triangles = std::panic::catch_unwind(triangles_report).map_err(panic_message);
        match rust_triangles {
            Ok(rust_triangles) => {
                if let Some(d) = diff(
                    "Lucene triangles",
                    &java_triangles,
                    "Rucene triangles",
                    &rust_triangles,
                ) {
                    failures.push(d);
                } else {
                    writeln!(
                        report,
                        "== triangles: Lucene == Rucene ({} lines)",
                        java_triangles.lines().count()
                    )
                    .unwrap();
                }
            }
            Err(e) => failures.push(format!("Rucene triangles panicked: {e}")),
        }
    } else {
        failures.push(format!("Lucene triangles failed:\n{}", java_errors(&err)));
    }

    // Shape index bytes: the masked points files of the two single-segment
    // indexes.
    let r_points = points_files(&r);
    let l_points = points_files(&l);
    assert_eq!(
        l_points.len(),
        3,
        "Lucene's single segment must have one .kdd, .kdi and .kdm: {:?}",
        l_points.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    match points_diff("R", &r_points, "L", &l_points) {
        Some(d) => failures.push(d),
        None => writeln!(
            report,
            "== masked points files R == L: {:?}",
            l_points
                .iter()
                .map(|(n, b)| format!("{n} ({} bytes)", b.len()))
                .collect::<Vec<_>>()
        )
        .unwrap(),
    }

    // Positive control: document 0 one degree north must change the report
    // (the distance sort lists it) with the same document counts, and the
    // points files.
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
    let points_control = points_diff("L", &l_points, "P", &points_files(&p));
    assert!(
        points_control.is_some(),
        "the masked points comparison must fail on the perturbed corpus"
    );
    writeln!(
        report,
        "== positive control: masked points files L != P\n{}",
        points_control.unwrap().trim_end()
    )
    .unwrap();

    println!("{report}");
    assert!(
        failures.is_empty(),
        "{} geo failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
