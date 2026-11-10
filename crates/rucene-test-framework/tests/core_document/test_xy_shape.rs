//! Port of `lucene/core/src/test/org/apache/lucene/document/TestXYShape.java`:
//! test case for indexing cartesian shapes and search by bounding box, lines,
//! and polygons.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::shape_field::QueryRelation;
use rucene::document::spatial_query::XYGeometryValue;
use rucene::document::{Document, XYShape};
use rucene::error::Result;
use rucene::geo::geometry::{XYCircle, XYLine, XYPoint, XYPolygon, XYRectangle};
use rucene::geo::tessellator::Tessellator;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::IndexReader;
use rucene::search::Query;
use rucene_test_framework::geo::shape_test_util;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};
use crate::shared_field::SharedField;

const FIELDNAME: &str = "field";

/// `TestXYShape`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestXYShape",
        vec![
            test("testBasicIntersects", test_basic_intersects),
            test("testBoundingBoxQueries", test_bounding_box_queries),
            test(
                "testPointIndexAndDistanceQuery",
                test_point_index_and_distance_query,
            ),
            test(
                "testContainsWrappingBooleanQuery",
                test_contains_wrapping_boolean_query,
            ),
            test(
                "testContainsIndexedGeometryCollection",
                test_contains_indexed_geometry_collection,
            ),
        ],
    )
}

/// `addPolygonsToDoc(field, doc, polygon)`.
fn add_polygons_to_doc(field: &str, doc: &mut Document, polygon: &XYPolygon) -> Result<()> {
    for f in XYShape::create_indexable_fields_polygon(field, polygon)? {
        doc.add(Box::new(f));
    }
    Ok(())
}

/// `addLineToDoc(field, doc, line)`.
fn add_line_to_doc(field: &str, doc: &mut Document, line: &XYLine) -> Result<()> {
    for f in XYShape::create_indexable_fields_line(field, line)? {
        doc.add(Box::new(f));
    }
    Ok(())
}

/// `newRectQuery(field, minX, maxX, minY, maxY)`.
fn new_rect_query(
    field: &str,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
) -> Result<Arc<dyn Query>> {
    XYShape::new_box_query(field, QueryRelation::Intersects, min_x, max_x, min_y, max_y)?
        .into_query()
}

/// `XYShape.newGeometryQuery(field, relation, geometries...)`.
fn geometry_query(
    field: &str,
    relation: QueryRelation,
    geometries: Vec<XYGeometryValue>,
) -> Result<Arc<dyn Query>> {
    XYShape::new_geometry_query(field, relation, geometries)?.into_query()
}

/// `reader.close()`.
fn close_reader(reader: Arc<dyn DirectoryReader>) -> Result<()> {
    (reader as Arc<dyn IndexReader>).close()
}

/// Port of `testBasicIntersects`: test we can search for a point with a
/// standard number of vertices.
fn test_basic_intersects() -> Result<()> {
    let num_vertices = TestUtil::next_int(&*random(), 50, 100);
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    // add a random polygon document
    let p = shape_test_util::create_regular_polygon(
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
    let mut x = vec![0f32; p.num_points() - 1];
    let mut y = vec![0f32; p.num_points() - 1];
    for i in 0..x.len() {
        x[i] = p.get_polyx_at(i);
        y[i] = p.get_polyy_at(i);
    }
    let l = XYLine::new(x.clone(), y.clone())?;
    add_line_to_doc(FIELDNAME, &mut document, &l)?;
    writer.add_document(&document)?;

    ////// search /////
    // search an intersecting bbox
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let min_x = x[0].min(x[1]);
    let min_y = y[0].min(y[1]);
    let max_x = x[0].max(x[1]);
    let max_y = y[0].max(y[1]);
    let q = new_rect_query(FIELDNAME, min_x, max_x, min_y, max_y)?;
    assert_equals(&2, &searcher.count(q)?);

    // search a disjoint bbox
    let q = new_rect_query(
        FIELDNAME,
        p.min_x() - 1.0,
        p.min_x() + 1.0,
        p.min_y() - 1.0,
        p.min_y() + 1.0,
    )?;
    assert_equals(&0, &searcher.count(q)?);

    // search w/ an intersecting polygon
    let q = XYShape::new_polygon_query(
        FIELDNAME,
        QueryRelation::Intersects,
        vec![XYPolygon::new(
            vec![min_x, min_x, max_x, max_x, min_x],
            vec![min_y, max_y, max_y, min_y, min_y],
            vec![],
        )?],
    )?
    .into_query()?;
    assert_equals(&2, &searcher.count(q)?);

    // search w/ an intersecting line
    let q = XYShape::new_line_query(
        FIELDNAME,
        QueryRelation::Intersects,
        vec![XYLine::new(
            vec![min_x, min_x, max_x, max_x],
            vec![min_y, max_y, max_y, min_y],
        )?],
    )?
    .into_query()?;
    assert_equals(&2, &searcher.count(q)?);

    close_reader(reader)?;
    dir.close()
}

/// Port of `testBoundingBoxQueries`.
fn test_bounding_box_queries() -> Result<()> {
    let random = random();
    let mut r1 = shape_test_util::next_box(&*random);
    let mut r2 = shape_test_util::next_box(&*random);
    let p;
    // find two boxes so that r1 contains r2
    loop {
        // TODO: Should XYRectangle hold values as float?
        if are_box_disjoint(&r1, &r2) {
            let candidate = to_polygon(&r2)?;
            // `catch (Exception e)`: ignore, try other combination
            if Tessellator::tessellate_xy(&candidate, random.next_boolean()).is_ok() {
                p = candidate;
                break;
            }
        }
        r1 = shape_test_util::next_box(&*random);
        r2 = shape_test_util::next_box(&*random);
    }

    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random, dir.as_directory())?;

    // add the polygon to the index
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &p)?;
    writer.add_document(&document)?;

    ////// search /////
    let reader = writer.get_reader()?;
    writer.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    // Query by itself should match
    let q = new_rect_query(FIELDNAME, r2.min_x(), r2.max_x(), r2.min_y(), r2.max_y())?;
    assert_equals(&1, &searcher.count(q)?);
    // r1 contains r2, intersects should match
    let q = new_rect_query(FIELDNAME, r1.min_x(), r1.max_x(), r1.min_y(), r1.max_y())?;
    assert_equals(&1, &searcher.count(q)?);
    // r1 contains r2, WITHIN should match
    let q = XYShape::new_box_query(
        FIELDNAME,
        QueryRelation::Within,
        r1.min_x(),
        r1.max_x(),
        r1.min_y(),
        r1.max_y(),
    )?
    .into_query()?;
    assert_equals(&1, &searcher.count(q)?);

    close_reader(reader)?;
    dir.close()
}

/// Port of `testPointIndexAndDistanceQuery`.
fn test_point_index_and_distance_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    let p_x = shape_test_util::next_float(&*random());
    let py = shape_test_util::next_float(&*random());
    for f in XYShape::create_indexable_fields_point(FIELDNAME, p_x, py)? {
        document.add(Box::new(f));
    }
    writer.add_document(&document)?;

    //// search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let circle = shape_test_util::next_circle();
    let circle_2d = XYGeometryValue::create(&[XYGeometryValue::Circle(circle.clone())])?;
    let (expected, expected_disjoint) = if circle_2d.contains(f64::from(p_x), f64::from(py)) {
        (1, 0)
    } else {
        (0, 1)
    };

    let distance = |relation| {
        XYShape::new_distance_query(FIELDNAME, relation, vec![circle.clone()])?.into_query()
    };
    let q = distance(QueryRelation::Intersects)?;
    assert_equals(&expected, &s.count(q)?);

    let q = distance(QueryRelation::Within)?;
    assert_equals(&expected, &s.count(q)?);

    let q = distance(QueryRelation::Disjoint)?;
    assert_equals(&expected_disjoint, &s.count(q)?);

    close_reader(r)?;
    dir.close()
}

/// Port of `testContainsWrappingBooleanQuery`.
fn test_contains_wrapping_boolean_query() -> Result<()> {
    let ys = vec![-30.0, -30.0, 30.0, 30.0, -30.0];
    let xs = vec![-30.0, 30.0, 30.0, -30.0, -30.0];
    let polygon = XYPolygon::new(xs, ys, vec![])?;

    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut document = Document::new();
    add_polygons_to_doc(FIELDNAME, &mut document, &polygon)?;
    writer.add_document(&document)?;

    //// search
    let r = writer.get_reader()?;
    writer.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let geometries = vec![
        XYGeometryValue::Rectangle(XYRectangle::new(0.0, 1.0, 0.0, 1.0)?),
        XYGeometryValue::Point(XYPoint::new(4.0, 4.0)?),
    ];
    // geometries within the polygon
    let q = geometry_query(FIELDNAME, QueryRelation::Contains, geometries)?;
    let top_docs = s.search(q, 1)?;
    assert_equals(&1, &top_docs.score_docs.len());
    assert_equals_delta(1.0, f64::from(top_docs.score_docs[0].score), 0.0);
    close_reader(r)?;
    dir.close()
}

/// Port of `testContainsIndexedGeometryCollection`.
fn test_contains_indexed_geometry_collection() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let polygon = XYPolygon::new(
        vec![-132.0, 132.0, 132.0, -132.0, -132.0],
        vec![-64.0, -64.0, 64.0, 64.0, -64.0],
        vec![],
    )?;
    let polygon_fields = SharedField::share_all(XYShape::create_indexable_fields_polygon(
        FIELDNAME, &polygon,
    )?);
    // POINT(5, 5) inside the indexed polygon
    let point_fields =
        SharedField::share_all(XYShape::create_indexable_fields_point(FIELDNAME, 5.0, 5.0)?);
    let num_docs = random().next_int_bound(1000);
    // index the same multi geometry many times
    for _ in 0..num_docs {
        let mut doc = Document::new();
        for f in &polygon_fields {
            doc.add(SharedField::boxed(f));
        }
        for _ in 0..10 {
            for f in &point_fields {
                doc.add(SharedField::boxed(f));
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
    // belonging to the same document. In this case the query geometry contains the indexed polygon
    // but the point is
    // inside the query as well, hence the result is 0.
    let polygon_query = XYPolygon::new(
        vec![4.0, 6.0, 6.0, 4.0, 4.0],
        vec![4.0, 4.0, 6.0, 6.0, 4.0],
        vec![],
    )?;
    let query = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![XYGeometryValue::Polygon(polygon_query)],
    )?;
    assert_equals(&0, &searcher.count(query)?);

    let rectangle = XYRectangle::new(4.0, 6.0, 4.0, 6.0)?;
    let query = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![XYGeometryValue::Rectangle(rectangle)],
    )?;
    assert_equals(&0, &searcher.count(query)?);

    let circle = XYCircle::new(5.0, 5.0, 1.0)?;
    let query = geometry_query(
        FIELDNAME,
        QueryRelation::Contains,
        vec![XYGeometryValue::Circle(circle)],
    )?;
    assert_equals(&0, &searcher.count(query)?);

    w.close()?;
    close_reader(reader)?;
    dir.close()
}

/// `areBoxDisjoint(r1, r2)`.
fn are_box_disjoint(r1: &XYRectangle, r2: &XYRectangle) -> bool {
    r1.min_x() <= r2.min_x()
        && r1.min_y() <= r2.min_y()
        && r1.max_x() >= r2.max_x()
        && r1.max_y() >= r2.max_y()
}

/// `toPolygon(r)`.
fn to_polygon(r: &XYRectangle) -> Result<XYPolygon> {
    XYPolygon::new(
        vec![r.min_x(), r.max_x(), r.max_x(), r.min_x(), r.min_x()],
        vec![r.min_y(), r.min_y(), r.max_y(), r.max_y(), r.min_y()],
        vec![],
    )
}
