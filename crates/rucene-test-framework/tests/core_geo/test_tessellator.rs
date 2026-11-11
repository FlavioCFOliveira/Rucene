//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestTessellator.java`:
//! test case for the Polygon `Tessellator` class.
//!
//! The WKT and GeoJSON literals live in `test_tessellator_data.rs`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::geo::tessellator::Monitor;
use rucene::geo::{
    ParseException, Point, Polygon, SimpleWKTShapeParser, Tessellator, Triangle, WktGeometry,
    XYPolygon,
};
use rucene_test_framework::geo::geo_test_util::{self, next_box_not_crossing_dateline};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{AwaitsFix, Nightly};

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class, iae_message, random, test, IAE,
};
use crate::test_tessellator_data as data;

/// `TestTessellator`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testLinesIntersect", test_lines_intersect).annotate(AwaitsFix {
            bug_url: "please open a PR if you want to fix this".to_string(),
        }),
        test("testSimpleTessellation", test_simple_tessellation),
        test(
            "testSimpleTessellationAtNight",
            test_simple_tessellation_at_night,
        )
        .annotate(Nightly),
        test("testLUCENE8454", test_lucene8454),
        test("testLUCENE8534", test_lucene8534),
        test(
            "testInvalidPolygonIntersects",
            test_invalid_polygon_intersects,
        ),
        test("testInvalidPolygonOverlap", test_invalid_polygon_overlap),
        test("testLUCENE8559", test_lucene8559),
        test("testLUCENE8556", test_lucene8556),
        test(
            "testInvalidPolygonCollinear",
            test_invalid_polygon_collinear,
        ),
        test(
            "testInvalidPolygonHoleDisjoint",
            test_invalid_polygon_hole_disjoint,
        ),
        test("testComplexPolygon01", test_complex_polygon01),
        test("testComplexPolygon02", test_complex_polygon02),
        test("testComplexPolygon03", test_complex_polygon03),
        test("testComplexPolygon04", test_complex_polygon04),
        test("testComplexPolygon05", test_complex_polygon05),
        test("testComplexPolygon06", test_complex_polygon06),
        test("testComplexPolygon07", test_complex_polygon07),
        test("testComplexPolygon08", test_complex_polygon08),
        test("testComplexPolygon09", test_complex_polygon09),
        test("testComplexPolygon10", test_complex_polygon10),
        test("testComplexPolygon11", test_complex_polygon11),
        test("testComplexPolygon12", test_complex_polygon12),
        test("testComplexPolygon13", test_complex_polygon13),
        test("testComplexPolygon14", test_complex_polygon14),
        test("testComplexPolygon15", test_complex_polygon15),
        test("testComplexPolygon16", test_complex_polygon16),
        test("testComplexPolygon17", test_complex_polygon17),
        test("testComplexPolygon18", test_complex_polygon18),
        test("testComplexPolygon19", test_complex_polygon19),
        test("testComplexPolygon20", test_complex_polygon20),
        test("testComplexPolygon21", test_complex_polygon21),
        test("testComplexPolygon22", test_complex_polygon22),
        test("testComplexPolygon23", test_complex_polygon23),
        test("testComplexPolygon24", test_complex_polygon24),
        test("testComplexPolygon25", test_complex_polygon25),
        test("testComplexPolygon26", test_complex_polygon26),
        test("testComplexPolygon27", test_complex_polygon27),
        test("testComplexPolygon28", test_complex_polygon28),
        test("testComplexPolygon29", test_complex_polygon29),
        test("testComplexPolygon30", test_complex_polygon30),
        test("testComplexPolygon31", test_complex_polygon31),
        test("testComplexPolygon32", test_complex_polygon32),
        test("testComplexPolygon33", test_complex_polygon33),
        test("testComplexPolygon34", test_complex_polygon34),
        test("testComplexPolygon35", test_complex_polygon35),
        test("testComplexPolygon36", test_complex_polygon36),
        test("testComplexPolygon37", test_complex_polygon37),
        test("testComplexPolygon38", test_complex_polygon38),
        test("testComplexPolygon39", test_complex_polygon39),
        test("testComplexPolygon40", test_complex_polygon40).annotate(Nightly),
        test("testComplexPolygon41", test_complex_polygon41),
        test("testComplexPolygon42", test_complex_polygon42).annotate(Nightly),
        test("testComplexPolygon43", test_complex_polygon43),
        test("testComplexPolygon44", test_complex_polygon44),
        test("testComplexPolygon45", test_complex_polygon45),
        test("testComplexPolygon46", test_complex_polygon46),
        test("testComplexPolygon47", test_complex_polygon47),
        test("testComplexPolygon48", test_complex_polygon48).annotate(Nightly),
        test("testComplexPolygon49", test_complex_polygon49),
        test("testComplexPolygon50", test_complex_polygon50),
        test(
            "testComplexPolygon50_WithMonitor",
            test_complex_polygon50_with_monitor,
        ),
        test("testComplexPolygon51", test_complex_polygon51),
        test("testComplexPolygon52", test_complex_polygon52),
        test("testComplexPolygon53", test_complex_polygon53),
        test("testComplexPolygon54", test_complex_polygon54),
        test("testComplexPolygon55", test_complex_polygon55),
        test("testComplexPolygon56", test_complex_polygon56),
        test("testComplexPolygon57", test_complex_polygon57),
        test("testComplexPolygon58", test_complex_polygon58).annotate(Nightly),
        test("testComplexPolygon59", test_complex_polygon59).annotate(Nightly),
        test("testComplexPolygon60", test_complex_polygon60),
        test("testComplexPolygon61", test_complex_polygon61),
        test("testComplexPolygon62", test_complex_polygon62),
        test("testComplexPolygon63", test_complex_polygon63),
        test("testComplexPolygon64", test_complex_polygon64),
        test("testComplexPolygon65", test_complex_polygon65),
    ];
    class("org.apache.lucene.geo.TestTessellator", methods)
}

/// `java.text.ParseException`, which the test methods declare.
fn checked(e: ParseException) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: rucene::util::extra::PARSE_EXCEPTION,
        message: Some(e.message().to_string()),
        cause: None,
    })
}

/// `Polygon.fromGeoJSON(geoJson)`.
fn from_geo_json(geo_json: &str) -> Result<Vec<Polygon>> {
    Polygon::from_geojson(geo_json).map_err(checked)
}

/// `(Polygon) SimpleWKTShapeParser.parse(wkt)`: any other shape is Java's
/// `ClassCastException`.
fn parse_polygon(wkt: &str) -> Result<Polygon> {
    match SimpleWKTShapeParser::parse(wkt)? {
        Some(WktGeometry::Polygon(p)) => Ok(p),
        _ => Err(LuceneError::unchecked(
            "java.lang.ClassCastException",
            "shape is not a org.apache.lucene.geo.Polygon",
        )),
    }
}

/// `(Polygon[]) SimpleWKTShapeParser.parse(wkt)`: any other shape is
/// Java's `ClassCastException`.
fn parse_multi_polygon(wkt: &str) -> Result<Vec<Polygon>> {
    match SimpleWKTShapeParser::parse(wkt)? {
        Some(WktGeometry::MultiPolygon(p)) => Ok(p
            .into_iter()
            .map(|p| p.expect("a MULTIPOLYGON member of this test is never EMPTY"))
            .collect()),
        _ => Err(LuceneError::unchecked(
            "java.lang.ClassCastException",
            "shape is not a [Lorg.apache.lucene.geo.Polygon;",
        )),
    }
}

/// Port of `TestTessellator.testLinesIntersect`.
fn test_lines_intersect() -> Result<()> {
    let rect = next_box_not_crossing_dateline();
    // simple case; test intersecting diagonals
    // note: we don't quantize because the tessellator operates on non quantized vertices
    Assert::assert_true(Tessellator::lines_intersect(
        rect.min_lon(),
        rect.min_lat(),
        rect.max_lon(),
        rect.max_lat(),
        rect.max_lon(),
        rect.min_lat(),
        rect.min_lon(),
        rect.max_lat(),
    ));
    // test closest encoded value
    Assert::assert_false(Tessellator::lines_intersect(
        rect.min_lon(),
        rect.max_lat(),
        rect.max_lon(),
        rect.max_lat(),
        rect.min_lon() - 1.0,
        rect.min_lat(),
        rect.min_lon() - 1.0,
        rect.max_lat(),
    ));
    Ok(())
}

/// Port of `TestTessellator.testSimpleTessellation`.
fn test_simple_tessellation() -> Result<()> {
    let poly = geo_test_util::create_regular_polygon(0.0, 0.0, 1000000.0, 100000)?;
    let inner = Polygon::new(
        vec![-1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0],
        vec![1.0, -1.0, -0.5, -1.0, 1.0, 0.5, 1.0],
        vec![],
    )?;
    let inner2 = Polygon::new(
        vec![-1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0],
        vec![-2.0, -4.0, -3.5, -4.0, -2.0, -2.5, -2.0],
        vec![],
    )?;
    let poly = Polygon::new(
        poly.get_polylats().to_vec(),
        poly.get_polylons().to_vec(),
        vec![inner, inner2],
    )?;
    Assert::assert_true(!Tessellator::tessellate(&poly, random().next_boolean())?.is_empty());
    Ok(())
}

/// Port of `TestTessellator.testSimpleTessellationAtNight`.
fn test_simple_tessellation_at_night() -> Result<()> {
    let poly = geo_test_util::create_regular_polygon(0.0, 0.0, 1000000.0, 1000000)?;
    let inner = Polygon::new(
        vec![-1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0],
        vec![1.0, -1.0, -0.5, -1.0, 1.0, 0.5, 1.0],
        vec![],
    )?;
    let inner2 = Polygon::new(
        vec![-1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0],
        vec![-2.0, -4.0, -3.5, -4.0, -2.0, -2.5, -2.0],
        vec![],
    )?;
    let poly = Polygon::new(
        poly.get_polylats().to_vec(),
        poly.get_polylons().to_vec(),
        vec![inner, inner2],
    )?;
    Assert::assert_true(!Tessellator::tessellate(&poly, random().next_boolean())?.is_empty());
    Ok(())
}

/// Port of `TestTessellator.testLUCENE8454`.
fn test_lucene8454() -> Result<()> {
    let geo_json = data::L_LUCENE8454;
    let polygons = from_geo_json(geo_json)?;
    let result = Tessellator::tessellate(&polygons[0], random().next_boolean())?;
    assert_equals(&result.len(), &84);
    Ok(())
}

/// Port of `TestTessellator.testLUCENE8534`.
fn test_lucene8534() -> Result<()> {
    let geo_json = data::L_LUCENE8534;
    let polygons = from_geo_json(geo_json)?;
    let result = Tessellator::tessellate(&polygons[0], random().next_boolean())?;
    assert_equals(&113, &result.len());
    Ok(())
}

/// Port of `TestTessellator.testInvalidPolygonIntersects`.
fn test_invalid_polygon_intersects() -> Result<()> {
    let wkt = data::L_INVALID_POLYGON_INTERSECTS;
    let polygon = parse_polygon(wkt)?;
    {
        let ex = iae_message(|| Tessellator::tessellate(&polygon, true));
        assert_equals("Polygon self-intersection at lat=0.5 lon=0.5", ex.as_str());
    }
    {
        let ex = iae_message(|| Tessellator::tessellate(&polygon, false));
        assert_equals(
            "Unable to Tessellate shape. Possible malformed shape detected.",
            ex.as_str(),
        );
    }
    Ok(())
}

/// Port of `TestTessellator.testInvalidPolygonOverlap`.
fn test_invalid_polygon_overlap() -> Result<()> {
    // holes are equal
    let wkt = data::L_INVALID_POLYGON_OVERLAP;
    let polygon = parse_polygon(wkt)?;
    {
        let ex = iae_message(|| Tessellator::tessellate(&polygon, true));
        assert_equals(
            "Polygon ring self-intersection at lat=52.0924189 lon=6.0377263",
            ex.as_str(),
        );
    }
    {
        let ex = iae_message(|| Tessellator::tessellate(&polygon, false));
        assert_equals(
            "Unable to Tessellate shape. Possible malformed shape detected.",
            ex.as_str(),
        );
    }
    Ok(())
}

/// Port of `TestTessellator.testLUCENE8559`.
fn test_lucene8559() -> Result<()> {
    let wkt = data::L_LUCENE8559;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testLUCENE8556`.
fn test_lucene8556() -> Result<()> {
    let wkt = data::L_LUCENE8556;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testInvalidPolygonCollinear`.
fn test_invalid_polygon_collinear() -> Result<()> {
    let wkt = data::L_INVALID_POLYGON_COLLINEAR;
    let polygon = parse_polygon(wkt)?;
    let ex = iae_message(|| Tessellator::tessellate(&polygon, true));
    assert_equals("at least three non-collinear points required", ex.as_str());
    Ok(())
}

/// Port of `TestTessellator.testInvalidPolygonHoleDisjoint`.
fn test_invalid_polygon_hole_disjoint() -> Result<()> {
    let wkt = data::L_INVALID_POLYGON_HOLE_DISJOINT;
    let polygon = parse_polygon(wkt)?;
    let ex = iae_message(|| Tessellator::tessellate(&polygon, true));
    assert_equals("Illegal hole detected: [[-180.0, 51.3], [-129.99, 51.3], [-129.99, 71.4], [-180.0, 71.4], [-180.0, 51.3]]", ex.as_str());

    let mut xs = vec![0f32; polygon.num_points()];
    let mut ys = vec![0f32; polygon.num_points()];
    for i in 0..polygon.num_points() {
        xs[i] = polygon.get_polylon(i) as f32;
        ys[i] = polygon.get_polylat(i) as f32;
    }
    let mut xs_hole = vec![0f32; polygon.get_hole(0).num_points()];
    let mut ys_hole = vec![0f32; polygon.get_hole(0).num_points()];
    for i in 0..polygon.get_hole(0).num_points() {
        xs_hole[i] = polygon.get_hole(0).get_polylon(i) as f32;
        ys_hole[i] = polygon.get_hole(0).get_polylat(i) as f32;
    }
    let xy_polygon = XYPolygon::new(xs, ys, vec![XYPolygon::new(xs_hole, ys_hole, vec![])?])?;
    let ex = iae_message(|| Tessellator::tessellate_xy(&xy_polygon, true));
    assert_equals("Illegal hole detected: [[-180.0, 51.3], [-129.99, 51.3], [-129.99, 71.4], [-180.0, 71.4], [-180.0, 51.3]]", ex.as_str());
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon01`.
fn test_complex_polygon01() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON01;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon02`.
fn test_complex_polygon02() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON02;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon03`.
fn test_complex_polygon03() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON03;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon04`.
fn test_complex_polygon04() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON04;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon05`.
fn test_complex_polygon05() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON05;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon06`.
fn test_complex_polygon06() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON06;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon07`.
fn test_complex_polygon07() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON07;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon08`.
fn test_complex_polygon08() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON08;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon09`.
fn test_complex_polygon09() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON09;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon10`.
fn test_complex_polygon10() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON10;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon11`.
fn test_complex_polygon11() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON11;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon12`.
fn test_complex_polygon12() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON12;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon13`.
fn test_complex_polygon13() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON13;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon14`.
fn test_complex_polygon14() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON14;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon15`.
fn test_complex_polygon15() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON15;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon16`.
fn test_complex_polygon16() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON16;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon17`.
fn test_complex_polygon17() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON17;
    let polygon = parse_polygon(wkt)?;
    let ex = iae_message(|| Tessellator::tessellate(&polygon, true));
    assert_equals(
        "Polygon ring self-intersection at lat=40.8277665 lon=14.1993975",
        ex.as_str(),
    );
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon18`.
fn test_complex_polygon18() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON18;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon19`.
fn test_complex_polygon19() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON19;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon20`.
fn test_complex_polygon20() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON20;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon21`.
fn test_complex_polygon21() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON21;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon22`.
fn test_complex_polygon22() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON22;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon23`.
fn test_complex_polygon23() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON23;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon24`.
fn test_complex_polygon24() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON24;
    let polygon = parse_polygon(wkt)?;
    expect_throws(IAE, || {
        Tessellator::tessellate(&polygon, random().next_boolean())
    });
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon25`.
fn test_complex_polygon25() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON25;
    let polygon = parse_polygon(wkt)?;
    expect_throws(IAE, || {
        Tessellator::tessellate(&polygon, random().next_boolean())
    });
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon26`.
fn test_complex_polygon26() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON26;
    let polygons = parse_multi_polygon(wkt)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.testComplexPolygon27`.
fn test_complex_polygon27() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON27;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon28`.
fn test_complex_polygon28() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON28;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon29`.
fn test_complex_polygon29() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON29;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon30`.
fn test_complex_polygon30() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON30;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon31`.
fn test_complex_polygon31() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON31;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon32`.
fn test_complex_polygon32() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON32;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon33`.
fn test_complex_polygon33() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON33;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon34`.
fn test_complex_polygon34() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON34;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon35`.
fn test_complex_polygon35() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON35;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon36`.
fn test_complex_polygon36() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON36;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon37`.
fn test_complex_polygon37() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON37;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon38`.
fn test_complex_polygon38() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON38;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon39`.
fn test_complex_polygon39() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON39;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon40`.
fn test_complex_polygon40() -> Result<()> {
    let wkt = geo_test_util::read_shape("lucene-9251.wkt.gz")?;
    let polygon = parse_polygon(&wkt)?;
    check_polygon(&polygon, 1e-12)
}

/// Port of `TestTessellator.testComplexPolygon41`.
fn test_complex_polygon41() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON41;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon42`.
fn test_complex_polygon42() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-9417.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon43`.
fn test_complex_polygon43() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON43;
    let polygon = parse_polygon(wkt)?;
    check_polygon(&polygon, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon44`.
fn test_complex_polygon44() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-9538-invalid.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    for i in 0..polygons.len() {
        if i == 21 {
            let illegal_polygon = &polygons[i];
            let ex = iae_message(|| Tessellator::tessellate(illegal_polygon, true));
            assert_equals(
                "Polygon self-intersection at lat=34.21165542666664 lon=-83.88787058666672",
                ex.as_str(),
            );
        } else {
            check_polygon(&polygons[i], 0.0)?;
        }
    }
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon45`.
fn test_complex_polygon45() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10470.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon46`.
fn test_complex_polygon46() -> Result<()> {
    let wkt = geo_test_util::read_shape("lucene-10470.wkt.gz")?;
    let polygon = parse_polygon(&wkt)?;
    check_polygon(&polygon, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon47`.
fn test_complex_polygon47() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10470-2.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon48`.
fn test_complex_polygon48() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10470-3.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon49`.
fn test_complex_polygon49() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON49;
    let polygon = parse_polygon(wkt)?;
    check_polygon(&polygon, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon50`.
fn test_complex_polygon50() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10563-1.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    assert_equals_msg(Some("Only one polygon"), &1, &polygons.len());
    check_polygon(&polygons[0], 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon50_WithMonitor`.
fn test_complex_polygon50_with_monitor() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10563-1.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    assert_equals_msg(Some("Only one polygon"), &1, &polygons.len());
    let polygon = &polygons[0];
    let mut monitor = TestCountingMonitor::default();
    Tessellator::tessellate_monitored(polygon, true, Some(&mut monitor))?;
    // MatcherAssert.assertThat("Expected many monitor calls", monitor.count, greaterThan(390))
    if monitor.count <= 390 {
        Assert::fail(&format!(
            "Expected many monitor calls\nExpected: a value greater than <390>\n     but: <{}> was less than or equal to <390>",
            monitor.count
        ));
    }
    assert_equals_msg(
        Some("Expected specific number of splits"),
        &3,
        &monitor.splits_started,
    );
    assert_equals_msg(
        Some("Expected splits to start and end"),
        &monitor.splits_ended,
        &monitor.splits_started,
    );
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon51`.
fn test_complex_polygon51() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10563-2.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    assert_equals_msg(Some("Only one polygon"), &1, &polygons.len());
    let polygon = &polygons[0];
    let check_self_intersections = random().next_boolean();
    let ex = iae_message(|| Tessellator::tessellate(polygon, check_self_intersections));
    let error = if check_self_intersections {
        "Polygon self-intersection at lat=2.8440144262027296 lon=177.96701124393607"
    } else {
        "Unable to Tessellate shape. Possible malformed shape detected."
    };
    assert_equals_msg(
        Some(&format!(
            "Expected specific error depending on checkSelfIntersections={check_self_intersections}"
        )),
        error,
        ex.as_str(),
    );
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon52`.
fn test_complex_polygon52() -> Result<()> {
    let geo_json = geo_test_util::read_shape("lucene-10563-3.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    assert_equals_msg(Some("Only one polygon"), &1, &polygons.len());
    let polygon = &polygons[0];
    let check_self_intersections = random().next_boolean();
    let ex = iae_message(|| Tessellator::tessellate(polygon, check_self_intersections));
    let error = if check_self_intersections {
        "Polygon self-intersection at lat=-11.22876335157631 lon=126.94854431224186"
    } else {
        "Unable to Tessellate shape. Possible malformed shape detected."
    };
    assert_equals_msg(
        Some(&format!(
            "Expected specific error depending on checkSelfIntersections={check_self_intersections}"
        )),
        error,
        ex.as_str(),
    );
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon53`.
fn test_complex_polygon53() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-11986-1.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.testComplexPolygon54`.
fn test_complex_polygon54() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-11986-2.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.testComplexPolygon55`.
fn test_complex_polygon55() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-12352-1.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    for polygon in &polygons {
        let tessellation = Tessellator::tessellate(polygon, random().next_boolean())?;
        assert_equals_delta(area(polygon)?, area_of(&tessellation)?, 0.0);
        // don't check edges as it takes several minutes
    }
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon56`.
fn test_complex_polygon56() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-12352-2.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    for polygon in &polygons {
        let tessellation = Tessellator::tessellate(polygon, random().next_boolean())?;
        assert_equals_delta(area(polygon)?, area_of(&tessellation)?, 0.0);
        // don't check edges as it takes several minutes
    }
    Ok(())
}

/// Port of `TestTessellator.testComplexPolygon57`.
fn test_complex_polygon57() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-13841-1.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 3e-11)
}

/// Port of `TestTessellator.testComplexPolygon58`.
fn test_complex_polygon58() -> Result<()> {
    let wkt = geo_test_util::read_shape("github-13841-2.wkt.gz")?;
    check_multi_polygon_wkt(&wkt)
}

/// Port of `TestTessellator.testComplexPolygon59`.
fn test_complex_polygon59() -> Result<()> {
    let wkt = geo_test_util::read_shape("github-13841-3.wkt.gz")?;
    let polygons = parse_multi_polygon(&wkt)?;
    check_multi_polygon(&polygons, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon60`.
fn test_complex_polygon60() -> Result<()> {
    let wkt = data::L_COMPLEX_POLYGON60;
    check_polygon_wkt(wkt)
}

/// Port of `TestTessellator.testComplexPolygon61`.
fn test_complex_polygon61() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-15205.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 1e-11)
}

/// Port of `TestTessellator.testComplexPolygon62`.
fn test_complex_polygon62() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-15657.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.testComplexPolygon63`.
fn test_complex_polygon63() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-15695.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.testComplexPolygon64`.
fn test_complex_polygon64() -> Result<()> {
    let geo_json = geo_test_util::read_shape("github-15731.geojson.gz")?;
    let polygons = from_geo_json(&geo_json)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.testComplexPolygon65`.
fn test_complex_polygon65() -> Result<()> {
    // Simplified extract of the top-left corner of github-15731.geojson.gz:
    // shell replaced with a bounding rectangle from the top-left-most shell vertex
    // to a bottom-right cutoff point, with two surviving holes.
    let geo_json = data::L_COMPLEX_POLYGON65;
    let polygons = from_geo_json(geo_json)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.TestCountingMonitor`.
#[derive(Default)]
struct TestCountingMonitor {
    count: i32,
    splits_started: i32,
    splits_ended: i32,
}

impl Monitor for TestCountingMonitor {
    fn current_state(
        &mut self,
        _status: &str,
        _points: Option<&[Point]>,
        _tessellation: &[Triangle],
    ) {
        self.count += 1;
    }

    fn start_split(&mut self, _status: &str, _left_polygon: &[Point], _right_polygon: &[Point]) {
        self.splits_started += 1;
    }

    fn end_split(&mut self, _status: &str) {
        self.splits_ended += 1;
    }
}

/// Port of `TestTessellator.checkMultiPolygon(String)`.
fn check_multi_polygon_wkt(wkt: &str) -> Result<()> {
    let polygons = parse_multi_polygon(wkt)?;
    check_multi_polygon(&polygons, 0.0)
}

/// Port of `TestTessellator.checkMultiPolygon(Polygon[], double)`.
fn check_multi_polygon(polygons: &[Polygon], delta: f64) -> Result<()> {
    for polygon in polygons {
        check_polygon(polygon, delta)?;
    }
    Ok(())
}

/// Port of `TestTessellator.checkPolygon(String)`.
fn check_polygon_wkt(wkt: &str) -> Result<()> {
    let polygon = parse_polygon(wkt)?;
    check_polygon(&polygon, 0.0)
}

/// Port of `TestTessellator.checkPolygon(Polygon, double)`.
fn check_polygon(polygon: &Polygon, delta: f64) -> Result<()> {
    let tessellation = Tessellator::tessellate(polygon, random().next_boolean())?;
    assert_equals_delta(area(polygon)?, area_of(&tessellation)?, delta);
    for t in &tessellation {
        check_triangle_edges_from_polygon(polygon, t);
    }
    Ok(())
}

/// Port of `TestTessellator.area(Polygon)`.
fn area(p: &Polygon) -> Result<f64> {
    let mut val = 0.0;
    for i in 0..p.num_points() - 1 {
        val += p.get_polylon(i) * p.get_polylat(i + 1) - p.get_polylat(i) * p.get_polylon(i + 1);
    }
    let mut area_v = (val / 2.0).abs();
    for hole in p.get_holes() {
        area_v -= area(hole)?;
    }
    Ok(area_v)
}

/// Port of `TestTessellator.area(List<Tessellator.Triangle>)`.
fn area_of(triangles: &[Triangle]) -> Result<f64> {
    let mut area_v = 0.0;
    for t in triangles {
        let lats = vec![t.get_y(0), t.get_y(1), t.get_y(2), t.get_y(0)];
        let lons = vec![t.get_x(0), t.get_x(1), t.get_x(2), t.get_x(0)];
        area_v += area(&Polygon::new(lats, lons, vec![])?)?;
    }
    Ok(area_v)
}

/// Port of `TestTessellator.checkTriangleEdgesFromPolygon`.
fn check_triangle_edges_from_polygon(p: &Polygon, t: &Triangle) {
    // first edge
    assert_equals(
        &t.is_edge_from_polygon(0),
        &is_edge_from_polygon(p, t.get_x(0), t.get_y(0), t.get_x(1), t.get_y(1)),
    );
    // second edge
    assert_equals(
        &t.is_edge_from_polygon(1),
        &is_edge_from_polygon(p, t.get_x(1), t.get_y(1), t.get_x(2), t.get_y(2)),
    );
    // third edge
    assert_equals(
        &t.is_edge_from_polygon(2),
        &is_edge_from_polygon(p, t.get_x(2), t.get_y(2), t.get_x(0), t.get_y(0)),
    );
}

/// Port of `TestTessellator.isEdgeFromPolygon`.
fn is_edge_from_polygon(p: &Polygon, a_lon: f64, a_lat: f64, b_lon: f64, b_lat: f64) -> bool {
    for i in 0..p.get_polylats().len() - 1 {
        if is_point_in_line(
            p.get_polylon(i),
            p.get_polylat(i),
            p.get_polylon(i + 1),
            p.get_polylat(i + 1),
            a_lon,
            a_lat,
        ) && is_point_in_line(
            p.get_polylon(i),
            p.get_polylat(i),
            p.get_polylon(i + 1),
            p.get_polylat(i + 1),
            b_lon,
            b_lat,
        ) {
            return true;
        }
        if p.get_polylon(i) != p.get_polylon(i + 1) || p.get_polylat(i) != p.get_polylat(i + 1) {
            // Check for co-planar points
            let length = p.get_polylats().len();
            let offset = i + 2;
            let mut j = 0;
            let mut index = get_index(length, j + offset);
            while j < length
                && area_signed(
                    p.get_polylon(i),
                    p.get_polylat(i),
                    p.get_polylon(i + 1),
                    p.get_polylat(i + 1),
                    p.get_polylon(index),
                    p.get_polylat(index),
                ) == 0.0
            {
                if is_point_in_line(
                    p.get_polylon(i),
                    p.get_polylat(i),
                    p.get_polylon(index),
                    p.get_polylat(index),
                    a_lon,
                    a_lat,
                ) && is_point_in_line(
                    p.get_polylon(i),
                    p.get_polylat(i),
                    p.get_polylon(index),
                    p.get_polylat(index),
                    b_lon,
                    b_lat,
                ) {
                    return true;
                }
                j += 1;
                index = get_index(length, j + offset);
            }
        }
    }
    // `p.getHoles() != null` always holds in Rucene.
    for hole in p.get_holes() {
        if is_edge_from_polygon(hole, a_lon, a_lat, b_lon, b_lat) {
            return true;
        }
    }
    false
}

/// Port of `TestTessellator.getIndex`.
fn get_index(size: usize, index: usize) -> usize {
    if index < size {
        return index;
    }
    index - size
}

/// Compute signed area of triangle
///
/// Port of `TestTessellator.area(double, double, double, double, double,
/// double)`.
fn area_signed(a_x: f64, a_y: f64, b_x: f64, b_y: f64, c_x: f64, c_y: f64) -> f64 {
    (b_y - a_y) * (c_x - b_x) - (b_x - a_x) * (c_y - b_y)
}

/// Port of `TestTessellator.isPointInLine`.
fn is_point_in_line(a_x: f64, a_y: f64, b_x: f64, b_y: f64, lon: f64, lat: f64) -> bool {
    let dxc = lon - a_x;
    let dyc = lat - a_y;
    let dxl = b_x - a_x;
    let dyl = b_y - a_y;
    if dxc * dyl - dyc * dxl == 0.0 {
        if dxl.abs() >= dyl.abs() {
            return if dxl > 0.0 {
                a_x <= lon && lon <= b_x
            } else {
                b_x <= lon && lon <= a_x
            };
        } else {
            return if dyl > 0.0 {
                a_y <= lat && lat <= b_y
            } else {
                b_y <= lat && lat <= a_y
            };
        }
    }
    false
}
