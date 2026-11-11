//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLatLonShape.java`:
//! test case for indexing polygons and querying by bounding box.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::junit::Ignore;
use rucene::document::shape_field::{QueryRelation, Triangle};
use rucene::document::spatial_query::LatLonGeometryValue;
use rucene::document::{Document, LatLonShape};
use rucene::error::Result;
use rucene::geo::component2d::Component2D;
use rucene::geo::encoding::{GeoEncodingUtils, GeoUtils};
use rucene::geo::geometry::{Circle, Line, Point, Polygon, Rectangle};
use rucene::geo::tessellator::Tessellator;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexReader, IndexableField};
use rucene::search::Query;
use rucene_test_framework::geo::geo_test_util;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test, unchecked};

const FIELDNAME: &str = "field";

/// `TestLatLonShape`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLatLonShape",
        vec![
            test("testRandomPolygons", test_random_polygons).annotate(Ignore {
                value: String::new(),
            }),
            test("testBasicIntersects", test_basic_intersects),
            test("testBasicContains", test_basic_contains),
            test("testPolygonWithHole", test_polygon_with_hole),
            test("testLargeVertexPolygon", test_large_vertex_polygon),
            test("testWithinDateLine", test_within_date_line),
            test("testContainsDateLine", test_contains_date_line),
            test("testLUCENE8454", test_lucene8454),
            test("testPointIndexAndQuery", test_point_index_and_query),
            test("testLUCENE8669", test_lucene8669),
            test("testLUCENE8679", test_lucene8679),
            test("testTriangleTouchingEdges", test_triangle_touching_edges),
            test("testLUCENE8736", test_lucene8736),
            test(
                "testTriangleCrossingPolygonVertices",
                test_triangle_crossing_polygon_vertices,
            ),
            test(
                "testLineCrossingPolygonVertices",
                test_line_crossing_polygon_vertices,
            ),
            test("testLineSharedLine", test_line_shared_line),
            test("testLUCENE9055", test_lucene9055),
            test(
                "testIndexAndQuerySamePolygon",
                test_index_and_query_same_polygon,
            ),
            test(
                "testPointIndexAndDistanceQuery",
                test_point_index_and_distance_query,
            ),
            test("testLucene9239", test_lucene9239),
            test(
                "testContainsWrappingBooleanQuery",
                test_contains_wrapping_boolean_query,
            ),
            test(
                "testContainsGeometryCollectionIntersectsPoint",
                test_contains_geometry_collection_intersects_point,
            ),
            test(
                "testContainsGeometryCollectionIntersectsLine",
                test_contains_geometry_collection_intersects_line,
            ),
            test(
                "testContainsGeometryCollectionIntersectsPolygon",
                test_contains_geometry_collection_intersects_polygon,
            ),
            test(
                "testFlatPolygonDoesNotContainIntersectingLine",
                test_flat_polygon_does_not_contain_intersecting_line,
            ),
        ],
    )
}

/// `quantizeLat(rawLat)`: quantizes a latitude value to be consistent with
/// index encoding.
fn quantize_lat(raw_lat: f64) -> f64 {
    GeoEncodingUtils::decode_latitude(unchecked(GeoEncodingUtils::encode_latitude(raw_lat)))
}

/// `quantizeLon(rawLon)`: quantizes a longitude value to be consistent with
/// index encoding.
fn quantize_lon(raw_lon: f64) -> f64 {
    GeoEncodingUtils::decode_longitude(unchecked(GeoEncodingUtils::encode_longitude(raw_lon)))
}

/// `new Polygon(lats, lons)`.
fn poly(lats: &[f64], lons: &[f64]) -> Result<Polygon> {
    Polygon::new(lats.to_vec(), lons.to_vec(), vec![])
}

/// `addPolygonsToDoc(field, doc, polygon)`.
fn add_polygons_to_doc(field: &str, doc: &mut Document, polygon: &Polygon) -> Result<()> {
    let fields = LatLonShape::create_indexable_fields_polygon_checked(
        field,
        polygon,
        random().next_boolean(),
    )?;
    for f in fields {
        doc.add(Box::new(f));
    }
    Ok(())
}

/// `addLineToDoc(field, doc, line)`.
fn add_line_to_doc(field: &str, doc: &mut Document, line: &Line) -> Result<()> {
    for f in LatLonShape::create_indexable_fields_line(field, line)? {
        doc.add(Box::new(f));
    }
    Ok(())
}

/// `newRectQuery(field, minLat, maxLat, minLon, maxLon)`.
fn new_rect_query(
    field: &str,
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
) -> Result<Arc<dyn Query>> {
    LatLonShape::new_box_query(
        field,
        QueryRelation::Intersects,
        min_lat,
        max_lat,
        min_lon,
        max_lon,
    )?
    .into_query()
}

/// `LatLonShape.newPolygonQuery(field, relation, polygons...)`.
fn polygon_query(
    field: &str,
    relation: QueryRelation,
    polygons: &[Polygon],
) -> Result<Arc<dyn Query>> {
    LatLonShape::new_polygon_query(field, relation, polygons.to_vec())?.into_query()
}

/// `LatLonShape.newBoxQuery(field, relation, ...)`.
fn box_query(
    field: &str,
    relation: QueryRelation,
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
) -> Result<Arc<dyn Query>> {
    LatLonShape::new_box_query(field, relation, min_lat, max_lat, min_lon, max_lon)?.into_query()
}

/// `LatLonShape.newGeometryQuery(field, relation, geometries...)`.
fn geometry_query(
    field: &str,
    relation: QueryRelation,
    geometries: Vec<LatLonGeometryValue>,
) -> Result<Arc<dyn Query>> {
    LatLonShape::new_geometry_query(field, relation, geometries)?.into_query()
}

/// `LatLonGeometry.create(geometry)`.
fn component(geometry: LatLonGeometryValue) -> Result<Arc<dyn Component2D>> {
    LatLonGeometryValue::create(&[geometry])
}

/// `reader.close()`.
fn close_reader(reader: Arc<dyn DirectoryReader>) -> Result<()> {
    (reader as Arc<dyn IndexReader>).close()
}

/// Port of `testRandomPolygons` (`@Ignore`).
fn test_random_polygons() -> Result<()> {
    let mut num_vertices;
    let num_polys = RandomNumbers::random_int_between(&*random(), 10, 20);

    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let mut polygon;
    let mut document;
    for _ in 0..num_polys {
        document = Document::new();
        num_vertices = TestUtil::next_int(&*random(), 100000, 200000);
        polygon = geo_test_util::create_regular_polygon(
            0.0,
            0.0,
            f64::from(LuceneTestCase::at_least(1000000)),
            num_vertices,
        )?;
        add_polygons_to_doc(FIELDNAME, &mut document, &polygon)?;
        writer.add_document(&document)?;
    }

    // search and verify we found our doc
    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    assert_equals(
        &0,
        &searcher.count(new_rect_query("field", -89.9, -89.8, -179.9, -179.8)?)?,
    );

    close_reader(reader)?;
    writer.close()?;
    dir.close()
}

/// Port of `testBasicIntersects`: test we can search for a point with a
/// standard number of vertices.
fn test_basic_intersects() -> Result<()> {
    let num_vertices = TestUtil::next_int(&*random(), 50, 100);
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add a random polygon document
    let p = geo_test_util::create_regular_polygon(
        0.0,
        90.0,
        f64::from(LuceneTestCase::at_least(1000000)),
        num_vertices,
    )?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &p)?;
    writer.add_document(&document)?;

    // add a line document
    document = Document::new();
    // add a line string
    let mut lats = vec![0f64; p.num_points() - 1];
    let mut lons = vec![0f64; p.num_points() - 1];
    for i in 0..lats.len() {
        lats[i] = p.get_polylat(i);
        lons[i] = p.get_polylon(i);
    }
    let l = Line::new(lats.clone(), lons.clone())?;
    add_line_to_doc(FIELDNAME, &mut document, &l)?;
    writer.add_document(&document)?;

    ////// search /////
    // search an intersecting bbox
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let min_lat = lats[0].min(lats[1]);
    let min_lon = lons[0].min(lons[1]);
    let max_lat = lats[0].max(lats[1]);
    let max_lon = lons[0].max(lons[1]);
    let q = new_rect_query(FIELDNAME, min_lat, max_lat, min_lon, max_lon)?;
    assert_equals(&2, &searcher.count(q)?);

    // search a disjoint bbox
    let q = new_rect_query(
        FIELDNAME,
        p.min_lat() - 1.0,
        p.min_lat() + 1.0,
        p.min_lon() - 1.0,
        p.min_lon() + 1.0,
    )?;
    assert_equals(&0, &searcher.count(q)?);

    close_reader(reader)?;
    dir.close()
}

/// Port of `testBasicContains`.
fn test_basic_contains() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add a random polygon document
    let poly_lats = [-10.0, -10.0, 10.0, 10.0, -10.0];
    let poly_lons = [-10.0, 10.0, 10.0, -10.0, -10.0];
    let p = poly(&poly_lats, &poly_lons)?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &p)?;
    writer.add_document(&document)?;

    // add a line document
    document = Document::new();
    // add a line string
    let mut lats = vec![0f64; p.num_points() - 1];
    let mut lons = vec![0f64; p.num_points() - 1];
    for i in 0..lats.len() {
        lats[i] = p.get_polylat(i);
        lons[i] = p.get_polylon(i);
    }
    let l = Line::new(lats, lons)?;
    add_line_to_doc(FIELDNAME, &mut document, &l)?;
    writer.add_document(&document)?;

    ////// search /////
    // search a Polygon
    let reader = writer.get_reader()?;
    writer.close()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let poly_lats = [-5.0, -5.0, 5.0, 5.0, -5.0];
    let poly_lons = [-5.0, 5.0, 5.0, -5.0, -5.0];
    let query = poly(&poly_lats, &poly_lons)?;
    let q = polygon_query(FIELDNAME, QueryRelation::Contains, &[query])?;
    assert_equals(&1, &searcher.count(q)?);

    // search a bounding box
    searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let q = box_query(FIELDNAME, QueryRelation::Contains, 0.0, 0.0, 0.0, 0.0)?;
    assert_equals(&1, &searcher.count(q)?);
    close_reader(reader)?;
    dir.close()
}

/// `new Polygon(new double[] {-1, -1, 1, 1, -1}, new double[] {-91, -89,
/// -89, -91, -91})`: the hole of the hole tests.
fn inner_hole() -> Result<Polygon> {
    poly(
        &[-1.0, -1.0, 1.0, 1.0, -1.0],
        &[-91.0, -89.0, -89.0, -91.0, -91.0],
    )
}

/// Port of `testPolygonWithHole`: test random polygons with a single hole.
fn test_polygon_with_hole() -> Result<()> {
    let num_vertices = TestUtil::next_int(&*random(), 50, 100);
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add a random polygon with a hole
    let inner = inner_hole()?;
    let outer = geo_test_util::create_regular_polygon(
        0.0,
        -90.0,
        f64::from(LuceneTestCase::at_least(1000000)),
        num_vertices,
    )?;

    let mut document = Document::new();
    add_polygons_to_doc(
        FIELDNAME,
        &mut document,
        &Polygon::new(
            outer.get_polylats().to_vec(),
            outer.get_polylons().to_vec(),
            vec![inner.clone()],
        )?,
    )?;
    writer.add_document(&document)?;

    ///// search //////
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // search a bbox in the hole
    let q = new_rect_query(
        FIELDNAME,
        inner.min_lat() + 1e-6,
        inner.max_lat() - 1e-6,
        inner.min_lon() + 1e-6,
        inner.max_lon() - 1e-6,
    )?;
    assert_equals(&0, &searcher.count(q)?);

    close_reader(reader)?;
    dir.close()
}

/// Port of `testLargeVertexPolygon`: test we can search for a point with a
/// large number of vertices.
fn test_large_vertex_polygon() -> Result<()> {
    let num_vertices = if *TEST_NIGHTLY {
        TestUtil::next_int(&*random(), 200000, 500000)
    } else {
        TestUtil::next_int(&*random(), 20000, 50000)
    };
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let mbd = iwc.max_buffered_docs();
    if mbd != -1 && mbd < num_vertices / 100 {
        iwc.set_max_buffered_docs(num_vertices / 100)?;
    }
    let simple_name = LuceneTestCase::get_test_class().simple_name().to_string();
    let dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(&simple_name));
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    // add a random polygon without a hole
    let p = geo_test_util::create_regular_polygon(
        0.0,
        90.0,
        f64::from(LuceneTestCase::at_least(1000000)),
        num_vertices,
    )?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &p)?;
    writer.add_document(&document)?;

    // add a random polygon with a hole
    let inner = inner_hole()?;
    let outer = geo_test_util::create_regular_polygon(
        0.0,
        -90.0,
        f64::from(LuceneTestCase::at_least(1000000)),
        num_vertices,
    )?;

    document = Document::new();
    add_polygons_to_doc(
        FIELDNAME,
        &mut document,
        &Polygon::new(
            outer.get_polylats().to_vec(),
            outer.get_polylons().to_vec(),
            vec![inner.clone()],
        )?,
    )?;
    writer.add_document(&document)?;

    ////// search /////
    // search an intersecting bbox
    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let q = new_rect_query(FIELDNAME, -1.0, 1.0, p.min_lon(), p.max_lon())?;
    assert_equals(&1, &searcher.count(q)?);

    // search a disjoint bbox
    let q = new_rect_query(
        FIELDNAME,
        p.min_lat() - 1.0,
        p.min_lat() + 1.0,
        p.min_lon() - 1.0,
        p.min_lon() + 1.0,
    )?;
    assert_equals(&0, &searcher.count(q)?);

    // search a bbox in the hole
    let q = new_rect_query(
        FIELDNAME,
        inner.min_lat() + 1e-6,
        inner.max_lat() - 1e-6,
        inner.min_lon() + 1e-6,
        inner.max_lon() - 1e-6,
    )?;
    assert_equals(&0, &searcher.count(q)?);

    close_reader(reader)?;
    dir.close()
}

/// The two indexed polygons of `testWithinDateLine` and `testLUCENE8669`.
fn date_line_index_polygons() -> Result<(Polygon, Polygon)> {
    Ok((
        poly(
            &[-7.5, 15.0, 15.0, 0.0, -7.5],
            &[-180.0, -180.0, -176.0, -176.0, -180.0],
        )?,
        poly(
            &[15.0, -7.5, -15.0, -10.0, 15.0, 15.0],
            &[180.0, 180.0, 176.0, 174.0, 176.0, 180.0],
        )?,
    ))
}

/// The search polygons of `testWithinDateLine` and `testLUCENE8669`.
fn date_line_search_polygons() -> Result<Vec<Polygon>> {
    Ok(vec![
        poly(
            &[-20.0, 20.0, 20.0, -20.0, -20.0],
            &[-180.0, -180.0, -170.0, -170.0, -180.0],
        )?,
        poly(
            &[20.0, -20.0, -20.0, 20.0, 20.0],
            &[180.0, 180.0, 170.0, 170.0, 180.0],
        )?,
    ])
}

/// Port of `testWithinDateLine`.
fn test_within_date_line() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();

    let (index_poly1, index_poly2) = date_line_index_polygons()?;

    add_polygons_to_doc("test", &mut doc, &index_poly1)?;
    add_polygons_to_doc("test", &mut doc, &index_poly2)?;

    w.add_document(&doc)?;
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let search_poly = date_line_search_polygons()?;

    let q = polygon_query("test", QueryRelation::Within, &search_poly)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = polygon_query("test", QueryRelation::Intersects, &search_poly)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = polygon_query("test", QueryRelation::Disjoint, &search_poly)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = polygon_query("test", QueryRelation::Contains, &search_poly)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = box_query("test", QueryRelation::Within, -20.0, 20.0, 170.0, -170.0)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = box_query(
        "test",
        QueryRelation::Intersects,
        -20.0,
        20.0,
        170.0,
        -170.0,
    )?;
    assert_equals(&1, &searcher.count(q)?);

    let q = box_query("test", QueryRelation::Disjoint, -20.0, 20.0, 170.0, -170.0)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = box_query("test", QueryRelation::Contains, -20.0, 20.0, 170.0, -170.0)?;
    assert_equals(&0, &searcher.count(q)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// Port of `testContainsDateLine`.
fn test_contains_date_line() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();

    let index_poly1 = poly(
        &[-2.0, -2.0, 2.0, 2.0, -2.0],
        &[178.0, 180.0, 180.0, 178.0, 178.0],
    )?;

    let index_poly2 = poly(
        &[-2.0, -2.0, 2.0, 2.0, -2.0],
        &[-180.0, -178.0, -178.0, -180.0, -180.0],
    )?;

    add_polygons_to_doc("test", &mut doc, &index_poly1)?;
    add_polygons_to_doc("test", &mut doc, &index_poly2)?;

    w.add_document(&doc)?;
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let search_poly = vec![
        poly(
            &[-1.0, -1.0, 1.0, 1.0, -1.0],
            &[179.0, 180.0, 180.0, 179.0, 179.0],
        )?,
        poly(
            &[-1.0, -1.0, 1.0, 1.0, -1.0],
            &[-180.0, -179.0, -179.0, -180.0, -180.0],
        )?,
    ];
    // Not supported due to encoding
    // Query q = LatLonShape.newPolygonQuery("test", QueryRelation.CONTAINS, searchPoly);
    // assertEquals(1, searcher.count(q));

    let q = polygon_query("test", QueryRelation::Intersects, &search_poly)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = polygon_query("test", QueryRelation::Disjoint, &search_poly)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = polygon_query("test", QueryRelation::Within, &search_poly)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = box_query("test", QueryRelation::Intersects, -1.0, 1.0, 179.0, -179.0)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = box_query("test", QueryRelation::Within, -1.0, 1.0, 179.0, -179.0)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = box_query("test", QueryRelation::Disjoint, -1.0, 1.0, 179.0, -179.0)?;
    assert_equals(&0, &searcher.count(q)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// Port of `testLUCENE8454`.
fn test_lucene8454() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let p = poly(
        &[-1.490648725633769E-132, 90.0, 90.0, -1.490648725633769E-132],
        &[0.0, 0.0, 180.0, 0.0],
    )?;

    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &p)?;
    writer.add_document(&document)?;

    ///// search //////
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // search a bbox in the hole
    let q = box_query(
        FIELDNAME,
        QueryRelation::Disjoint,
        -29.46555603761226,
        0.0,
        8.381903171539307E-8,
        0.9999999403953552,
    )?;
    assert_equals(&1, &searcher.count(q)?);

    close_reader(reader)?;
    dir.close()
}

/// Port of `testPointIndexAndQuery`.
fn test_point_index_and_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    let mut p = geo_test_util::next_point();
    let q_lat = GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude(p.lat())?);
    let q_lon = GeoEncodingUtils::decode_longitude(GeoEncodingUtils::encode_longitude(p.lon())?);
    p = Point::new(q_lat, q_lon)?;
    let fields = LatLonShape::create_indexable_fields_point(FIELDNAME, p.lat(), p.lon())?;
    for f in fields {
        document.add(Box::new(f));
    }
    writer.add_document(&document)?;

    //// search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    // search by same point
    let q =
        LatLonShape::new_point_query(FIELDNAME, QueryRelation::Intersects, &[[p.lat(), p.lon()]])?
            .into_query()?;
    assert_equals(&1, &s.count(q)?);
    close_reader(r)?;
    dir.close()
}

/// Port of `testLUCENE8669`.
fn test_lucene8669() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();

    let (index_poly1, index_poly2) = date_line_index_polygons()?;

    add_polygons_to_doc(FIELDNAME, &mut doc, &index_poly1)?;
    add_polygons_to_doc(FIELDNAME, &mut doc, &index_poly2)?;
    w.add_document(&doc)?;
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let search_poly = date_line_search_polygons()?;

    let q = polygon_query(FIELDNAME, QueryRelation::Within, &search_poly)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = polygon_query(FIELDNAME, QueryRelation::Intersects, &search_poly)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = polygon_query(FIELDNAME, QueryRelation::Disjoint, &search_poly)?;
    assert_equals(&0, &searcher.count(q)?);

    let q = box_query(FIELDNAME, QueryRelation::Within, -20.0, 20.0, 170.0, -170.0)?;
    assert_equals(&1, &searcher.count(q)?);

    let q = box_query(
        FIELDNAME,
        QueryRelation::Intersects,
        -20.0,
        20.0,
        170.0,
        -170.0,
    )?;
    assert_equals(&1, &searcher.count(q)?);

    let q = box_query(
        FIELDNAME,
        QueryRelation::Disjoint,
        -20.0,
        20.0,
        170.0,
        -170.0,
    )?;
    assert_equals(&0, &searcher.count(q)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// Port of `testLUCENE8679`.
fn test_lucene8679() -> Result<()> {
    let alat = 1.401298464324817E-45;
    let alon = 24.76789767911785;
    let blat = 34.26468306870807;
    let blon = -52.67048754768767;
    let polygon = poly(
        &[
            -14.448264200949083,
            0.0,
            0.0,
            -14.448264200949083,
            -14.448264200949083,
        ],
        &[
            0.9999999403953552,
            0.9999999403953552,
            124.50086371762484,
            124.50086371762484,
            0.9999999403953552,
        ],
    )?;
    let polygon_2d = component(LatLonGeometryValue::Polygon(polygon))?;
    let mut intersects = polygon_2d.intersects_triangle_bbox(
        quantize_lon(alon),
        quantize_lat(blat),
        quantize_lon(blon),
        quantize_lat(blat),
        quantize_lon(alon),
        quantize_lat(alat),
    );

    Assert::assert_true(intersects);

    intersects = polygon_2d.intersects_triangle_bbox(
        quantize_lon(alon),
        quantize_lat(blat),
        quantize_lon(alon),
        quantize_lat(alat),
        quantize_lon(blon),
        quantize_lat(blat),
    );

    Assert::assert_true(intersects);
    Ok(())
}

/// `polygon2D.containsTriangle(...)` and `polygon2D.intersectsTriangle(...)`
/// over quantized `(lon, lat)` vertices.
fn tri(c: &Arc<dyn Component2D>, contains: bool, v: [(f64, f64); 3]) -> bool {
    let [(a_lon, a_lat), (b_lon, b_lat), (c_lon, c_lat)] = v;
    let args = (
        quantize_lon(a_lon),
        quantize_lat(a_lat),
        quantize_lon(b_lon),
        quantize_lat(b_lat),
        quantize_lon(c_lon),
        quantize_lat(c_lat),
    );
    if contains {
        c.contains_triangle_bbox(args.0, args.1, args.2, args.3, args.4, args.5)
    } else {
        c.intersects_triangle_bbox(args.0, args.1, args.2, args.3, args.4, args.5)
    }
}

/// Port of `testTriangleTouchingEdges`.
fn test_triangle_touching_edges() -> Result<()> {
    let p = poly(&[0.0, 0.0, 1.0, 1.0, 0.0], &[0.0, 1.0, 1.0, 0.0, 0.0])?;
    let polygon_2d = component(LatLonGeometryValue::Polygon(p))?;
    // 3 shared points
    let mut contains_triangle = tri(&polygon_2d, true, [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0)]);
    let mut intersects_triangle = tri(&polygon_2d, false, [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_true(contains_triangle);
    // 2 shared points
    contains_triangle = tri(&polygon_2d, true, [(0.5, 0.0), (1.0, 0.5), (0.5, 0.75)]);
    intersects_triangle = tri(&polygon_2d, false, [(0.5, 0.0), (1.0, 0.5), (0.5, 0.75)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_true(contains_triangle);
    // 1 shared point
    contains_triangle = tri(&polygon_2d, true, [(0.5, 0.5), (0.5, 0.0), (0.75, 0.75)]);
    intersects_triangle = tri(&polygon_2d, false, [(0.5, 0.0), (1.0, 0.5), (0.5, 0.75)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_true(contains_triangle);
    // 1 shared point but out
    contains_triangle = tri(&polygon_2d, true, [(1.0, 0.5), (2.0, 0.0), (2.0, 2.0)]);
    intersects_triangle = tri(&polygon_2d, false, [(1.0, 0.5), (2.0, 0.0), (2.0, 2.0)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_false(contains_triangle);
    // 1 shared point but crossing
    contains_triangle = tri(&polygon_2d, true, [(0.5, 0.0), (2.0, 0.5), (0.5, 1.0)]);
    intersects_triangle = tri(&polygon_2d, false, [(0.5, 0.0), (2.0, 0.5), (0.5, 1.0)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_false(contains_triangle);
    // share one edge
    contains_triangle = tri(&polygon_2d, true, [(0.0, 0.0), (0.0, 1.0), (0.5, 0.5)]);
    intersects_triangle = tri(&polygon_2d, false, [(0.0, 0.0), (0.0, 1.0), (0.5, 0.5)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_true(contains_triangle);
    // share one edge outside
    contains_triangle = tri(&polygon_2d, true, [(0.0, 1.0), (1.5, 1.5), (1.0, 1.0)]);
    intersects_triangle = tri(&polygon_2d, false, [(0.0, 1.0), (1.5, 1.5), (1.0, 1.0)]);
    Assert::assert_true(intersects_triangle);
    Assert::assert_false(contains_triangle);
    Ok(())
}

/// Port of `testLUCENE8736`.
fn test_lucene8736() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // test polygons:
    let index_poly1 = poly(&[4.0, 4.0, 3.0, 3.0, 4.0], &[3.0, 4.0, 4.0, 3.0, 3.0])?;

    let index_poly2 = poly(&[2.0, 2.0, 1.0, 1.0, 2.0], &[6.0, 7.0, 7.0, 6.0, 6.0])?;

    let index_poly3 = poly(&[1.0, 1.0, 0.0, 0.0, 1.0], &[3.0, 4.0, 4.0, 3.0, 3.0])?;

    let index_poly4 = poly(&[2.0, 2.0, 1.0, 1.0, 2.0], &[0.0, 1.0, 1.0, 0.0, 0.0])?;

    // index polygons:
    for p in [&index_poly1, &index_poly2, &index_poly3, &index_poly4] {
        let mut doc = Document::new();
        add_polygons_to_doc(FIELDNAME, &mut doc, p)?;
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let search_poly = vec![poly(
        &[4.0, 4.0, 0.0, 0.0, 4.0],
        &[0.0, 7.0, 7.0, 0.0, 0.0],
    )?];

    let q = polygon_query(FIELDNAME, QueryRelation::Within, &search_poly)?;
    assert_equals(&4, &searcher.count(q)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// Port of `testTriangleCrossingPolygonVertices`.
fn test_triangle_crossing_polygon_vertices() -> Result<()> {
    let p = poly(
        &[0.0, 0.0, -5.0, -10.0, -5.0, 0.0],
        &[-1.0, 1.0, 5.0, 0.0, -5.0, -1.0],
    )?;
    let polygon_2d = component(LatLonGeometryValue::Polygon(p))?;
    let intersects_triangle = tri(
        &polygon_2d,
        false,
        [(-5.0, 0.0), (10.0, 0.0), (-5.0, -15.0)],
    );
    Assert::assert_true(intersects_triangle);
    Ok(())
}

/// Port of `testLineCrossingPolygonVertices`.
fn test_line_crossing_polygon_vertices() -> Result<()> {
    let p = poly(&[0.0, -1.0, 0.0, 1.0, 0.0], &[-1.0, 0.0, 1.0, 0.0, -1.0])?;
    let polygon_2d = component(LatLonGeometryValue::Polygon(p))?;
    let intersects_triangle = tri(&polygon_2d, false, [(-1.5, 0.0), (1.5, 0.0), (-1.5, 0.0)]);
    Assert::assert_true(intersects_triangle);
    Ok(())
}

/// Port of `testLineSharedLine`.
fn test_line_shared_line() -> Result<()> {
    let l = Line::new(vec![0.0, 0.0, 0.0, 0.0], vec![-2.0, -1.0, 0.0, 1.0])?;
    let l2d = component(LatLonGeometryValue::Line(l))?;
    let intersects_line = l2d.intersects_line_bbox(
        quantize_lon(-5.0),
        quantize_lat(0.0),
        quantize_lon(5.0),
        quantize_lat(0.0),
    );
    Assert::assert_true(intersects_line);
    Ok(())
}

/// Port of `testLUCENE9055`.
fn test_lucene9055() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // test polygons:
    // [5, 5], [10, 6], [10, 10], [5, 10], [5, 5] ]
    let index_poly1 = poly(&[5.0, 6.0, 10.0, 10.0, 5.0], &[5.0, 10.0, 10.0, 5.0, 5.0])?;

    // [ [6, 6], [9, 6], [9, 9], [6, 9], [6, 6] ]
    let index_poly2 = poly(&[6.0, 6.0, 9.0, 9.0, 6.0], &[6.0, 9.0, 9.0, 6.0, 6.0])?;

    // index polygons:
    for p in [&index_poly1, &index_poly2] {
        let mut doc = Document::new();
        add_polygons_to_doc(FIELDNAME, &mut doc, p)?;
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // [ [0, 0], [5, 5], [7, 7] ]
    let search_line = Line::new(vec![0.0, 5.0, 7.0], vec![0.0, 5.0, 7.0])?;

    let q = LatLonShape::new_line_query(FIELDNAME, QueryRelation::Intersects, vec![search_line])?
        .into_query()?;
    assert_equals(&2, &searcher.count(q)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// Port of `testIndexAndQuerySamePolygon`.
fn test_index_and_query_same_polygon() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let polygon = loop {
        // `catch (Exception e)`: any failure is an invalid polygon; try a new one.
        let attempt = (|| -> Result<Polygon> {
            let polygon = geo_test_util::next_polygon();
            // quantize the polygon
            let mut lats = vec![0f64; polygon.num_points()];
            let mut lons = vec![0f64; polygon.num_points()];
            for i in 0..polygon.num_points() {
                lats[i] = GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude(
                    polygon.get_polylat(i),
                )?);
                lons[i] = GeoEncodingUtils::decode_longitude(GeoEncodingUtils::encode_longitude(
                    polygon.get_polylon(i),
                )?);
            }
            let polygon = Polygon::new(lats, lons, vec![])?;
            Tessellator::tessellate(&polygon, random().next_boolean())?;
            Ok(polygon)
        })();
        if let Ok(polygon) = attempt {
            break polygon;
        }
        // invalid polygon, try a new one
    };
    add_polygons_to_doc(FIELDNAME, &mut doc, &polygon)?;
    w.add_document(&doc)?;
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let q = polygon_query(
        FIELDNAME,
        QueryRelation::Within,
        std::slice::from_ref(&polygon),
    )?;
    assert_equals(&1, &searcher.count(q)?);
    let q = polygon_query(
        FIELDNAME,
        QueryRelation::Intersects,
        std::slice::from_ref(&polygon),
    )?;
    assert_equals(&1, &searcher.count(q)?);
    let q = polygon_query(
        FIELDNAME,
        QueryRelation::Disjoint,
        std::slice::from_ref(&polygon),
    )?;
    assert_equals(&0, &searcher.count(q)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// Port of `testPointIndexAndDistanceQuery`.
fn test_point_index_and_distance_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    let p = geo_test_util::next_point();
    let fields = LatLonShape::create_indexable_fields_point(FIELDNAME, p.lat(), p.lon())?;
    for f in fields {
        document.add(Box::new(f));
    }
    writer.add_document(&document)?;

    //// search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let lat = geo_test_util::next_latitude();
    let lon = geo_test_util::next_longitude();
    let radius_meters =
        random().next_double() * GeoUtils::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI / 2.0
            + 1.0;
    let circle = Circle::new(lat, lon, radius_meters)?;
    let circle_2d = component(LatLonGeometryValue::Circle(circle))?;
    let (expected, expected_disjoint) = if circle_2d.contains(p.lon(), p.lat()) {
        (1, 0)
    } else {
        (0, 1)
    };

    let distance =
        |relation| LatLonShape::new_distance_query(FIELDNAME, relation, vec![circle])?.into_query();
    let q = distance(QueryRelation::Intersects)?;
    assert_equals(&expected, &s.count(q)?);

    let q = distance(QueryRelation::Within)?;
    assert_equals(&expected, &s.count(q)?);

    let q = distance(QueryRelation::Disjoint)?;
    assert_equals(&expected_disjoint, &s.count(q)?);

    close_reader(r)?;
    dir.close()
}

/// Port of `testLucene9239`.
fn test_lucene9239() -> Result<()> {
    let lats = [
        -22.350172194105966,
        90.0,
        90.0,
        -22.350172194105966,
        -22.350172194105966,
    ];
    let lons = [
        49.931598911327825,
        49.931598911327825,
        51.40819689137876,
        51.408196891378765,
        49.931598911327825,
    ];
    let polygon = poly(&lats, &lons)?;

    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &polygon)?;
    writer.add_document(&document)?;

    //// search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let circle = Circle::new(78.01086555431775, 0.9513280497489234, 1097753.4254892308)?;
    // Circle is not within the polygon
    let q = LatLonShape::new_distance_query(FIELDNAME, QueryRelation::Contains, vec![circle])?
        .into_query()?;
    assert_equals(&0, &s.count(q)?);

    close_reader(r)?;
    dir.close()
}

/// Port of `testContainsWrappingBooleanQuery`.
fn test_contains_wrapping_boolean_query() -> Result<()> {
    let lats = [-30.0, -30.0, 30.0, 30.0, -30.0];
    let lons = [-30.0, 30.0, 30.0, -30.0, -30.0];
    let polygon = poly(&lats, &lons)?;

    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &polygon)?;
    writer.add_document(&document)?;

    //// search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let geometries = vec![
        LatLonGeometryValue::Rectangle(Rectangle::new(0.0, 1.0, 0.0, 1.0)?),
        LatLonGeometryValue::Point(Point::new(4.0, 4.0)?),
    ];
    // geometries within the polygon
    let q = geometry_query(FIELDNAME, QueryRelation::Contains, geometries)?;
    let top_docs = s.search(q, 1)?;
    assert_equals(&1, &top_docs.score_docs.len());
    assert_equals_delta(1.0, f64::from(top_docs.score_docs[0].score), 0.0);
    close_reader(r)?;
    dir.close()
}

/// `new Polygon(new double[] {-64, -64, 64, 64, -64}, new double[] {-132,
/// 132, 132, -132, -132})`.
fn collection_polygon() -> Result<Polygon> {
    poly(
        &[-64.0, -64.0, 64.0, 64.0, -64.0],
        &[-132.0, 132.0, 132.0, -132.0, -132.0],
    )
}

/// Port of `testContainsGeometryCollectionIntersectsPoint`.
fn test_contains_geometry_collection_intersects_point() -> Result<()> {
    let polygon = collection_polygon()?;
    do_test_contains_geometry_collection_intersects(
        LatLonShape::create_indexable_fields_polygon(FIELDNAME, &polygon)?,
        // point inside the indexed polygon
        LatLonShape::create_indexable_fields_point(FIELDNAME, 5.0, 5.0)?,
    )
}

/// Port of `testContainsGeometryCollectionIntersectsLine`.
fn test_contains_geometry_collection_intersects_line() -> Result<()> {
    let polygon = collection_polygon()?;
    let line = Line::new(vec![5.0, 5.1], vec![5.0, 5.1])?;
    do_test_contains_geometry_collection_intersects(
        LatLonShape::create_indexable_fields_polygon(FIELDNAME, &polygon)?,
        // Line inside the polygon
        LatLonShape::create_indexable_fields_line(FIELDNAME, &line)?,
    )
}

/// Port of `testContainsGeometryCollectionIntersectsPolygon`.
fn test_contains_geometry_collection_intersects_polygon() -> Result<()> {
    let polygon = collection_polygon()?;
    let polygon_inside = poly(&[5.0, 5.0, 5.1, 5.1, 5.0], &[5.0, 5.1, 5.1, 5.0, 5.0])?;
    do_test_contains_geometry_collection_intersects(
        LatLonShape::create_indexable_fields_polygon(FIELDNAME, &polygon)?,
        // this polygon is inside the other polygon
        LatLonShape::create_indexable_fields_polygon(FIELDNAME, &polygon_inside)?,
    )
}

/// Port of `testFlatPolygonDoesNotContainIntersectingLine`.
fn test_flat_polygon_does_not_contain_intersecting_line() -> Result<()> {
    // Create line intersecting very flat polygon (but not contained)
    let lons = [-0.001, -0.001, 0.001, 0.001, -0.001];
    let lats = [1e-10, 0.0, -1e-10, 0.0, 1e-10];
    let polygon = poly(&lats, &lons)?;
    let line = Line::new(vec![0.0, 0.001], vec![0.0, 0.0])?;

    // Index the polygon
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &polygon)?;
    writer.add_document(&document)?;

    // search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    // search for line within the polygon
    let q = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![LatLonGeometryValue::Line(line)],
    )?;
    let top_docs = s.search(q, 1)?;
    crate::core_support::assert_equals_msg(
        Some("Polygon should not contain the line,"),
        &0,
        &top_docs.score_docs.len(),
    );

    // cleanup
    close_reader(r)?;
    dir.close()
}

/// The private `doTestContainsGeometryCollectionIntersects(containsFields,
/// intersectsField)`.
fn do_test_contains_geometry_collection_intersects(
    contains_fields: Vec<Triangle>,
    intersects_field: Vec<Triangle>,
) -> Result<()> {
    let contains_fields = contains_fields
        .into_iter()
        .map(|f| Arc::new(f) as Arc<dyn IndexableField>)
        .collect::<Vec<_>>();
    let intersects_field = intersects_field
        .into_iter()
        .map(|f| Arc::new(f) as Arc<dyn IndexableField>)
        .collect::<Vec<_>>();
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = random().next_int_bound(1000);
    // index the same multi geometry many times
    for _ in 0..num_docs {
        let mut doc = Document::new();
        for f in &contains_fields {
            doc.add_shared(Arc::clone(f));
        }
        for _ in 0..10 {
            for f in &intersects_field {
                doc.add_shared(Arc::clone(f));
            }
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    // Contains is only true if the query geometry is inside a geometry and does not intersect with
    // any other geometry
    // belonging to the same document. In this case the query geometry contains the indexed fields
    // but the intersectsFields are
    // inside the query as well, hence the result is 0.
    let polygon_query = poly(&[4.0, 4.0, 6.0, 6.0, 4.0], &[4.0, 6.0, 6.0, 4.0, 4.0])?;
    let query = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![LatLonGeometryValue::Polygon(polygon_query)],
    )?;
    assert_equals(&0, &searcher.count(query)?);

    let rectangle = Rectangle::new(4.0, 6.0, 4.0, 6.0)?;
    let query = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![LatLonGeometryValue::Rectangle(rectangle)],
    )?;
    assert_equals(&0, &searcher.count(query)?);

    let circle = Circle::new(5.0, 5.0, 10000.0)?;
    let query = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![LatLonGeometryValue::Circle(circle)],
    )?;
    assert_equals(&0, &searcher.count(query)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}
