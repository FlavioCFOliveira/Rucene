//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestPolygon.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{ParseException, Polygon};

use crate::core_support::{assert_contains, assert_equals, class, iae_message, test};

/// `TestPolygon`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestPolygon",
        vec![
            test("testPolygonNullPolyLats", test_polygon_null_poly_lats),
            test("testPolygonNullPolyLons", test_polygon_null_poly_lons),
            test("testPolygonLine", test_polygon_line),
            test("testPolygonBogus", test_polygon_bogus),
            test("testPolygonNotClosed", test_polygon_not_closed),
            test("testGeoJSONPolygon", test_geo_json_polygon),
            test(
                "testGeoJSONPolygonWithHole",
                test_geo_json_polygon_with_hole,
            ),
            test("testGeoJSONMultiPolygon", test_geo_json_multi_polygon),
            test("testGeoJSONTypeComesLast", test_geo_json_type_comes_last),
            test("testGeoJSONPolygonFeature", test_geo_json_polygon_feature),
            test(
                "testGeoJSONMultiPolygonFeature",
                test_geo_json_multi_polygon_feature,
            ),
            test(
                "testGeoJSONFeatureCollectionWithSinglePolygon",
                test_geo_json_feature_collection_with_single_polygon,
            ),
            test(
                "testIllegalGeoJSONExtraCrapAtEnd",
                test_illegal_geo_json_extra_crap_at_end,
            ),
            test(
                "testIllegalGeoJSONLinkedCRS",
                test_illegal_geo_json_linked_crs,
            ),
            test(
                "testIllegalGeoJSONMultipleFeatures",
                test_illegal_geo_json_multiple_features,
            ),
            test(
                "testPolygonPropertiesCanBeStringArrays",
                test_polygon_properties_can_be_string_arrays,
            ),
        ],
    )
}

/// `Polygon.fromGeoJSON(geojson)`, whose `java.text.ParseException` is a
/// checked exception the test methods declare (`throws Exception`).
fn from_geo_json(geojson: &str) -> Result<Vec<Polygon>> {
    Polygon::from_geojson(geojson).map_err(|e| {
        rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::Unchecked {
            class: rucene::util::extra::PARSE_EXCEPTION,
            message: Some(e.message().to_string()),
            cause: None,
        })
    })
}

/// `expectThrows(ParseException.class, () -> Polygon.fromGeoJSON(geojson))
/// .getMessage()`.
fn expect_parse_exception(geojson: &str) -> String {
    let result: std::result::Result<Vec<Polygon>, ParseException> = Polygon::from_geojson(geojson);
    match result {
        Err(e) => e.message().to_string(),
        Ok(_) => Assert::fail("Expected exception ParseException but no exception was thrown"),
    }
}

/// null polyLats not allowed
///
/// Port of `TestPolygon.testPolygonNullPolyLats`.
fn test_polygon_null_poly_lats() -> Result<()> {
    let expected =
        iae_message(|| Polygon::new(None, vec![-66.0, -65.0, -65.0, -66.0, -66.0], vec![]));
    Assert::assert_true(expected.contains("polyLats must not be null"));
    Ok(())
}

/// null polyLons not allowed
///
/// Port of `TestPolygon.testPolygonNullPolyLons`.
fn test_polygon_null_poly_lons() -> Result<()> {
    let expected = iae_message(|| Polygon::new(vec![18.0, 18.0, 19.0, 19.0, 18.0], None, vec![]));
    Assert::assert_true(expected.contains("polyLons must not be null"));
    Ok(())
}

/// polygon needs at least 3 vertices
///
/// Port of `TestPolygon.testPolygonLine`.
fn test_polygon_line() -> Result<()> {
    let expected =
        iae_message(|| Polygon::new(vec![18.0, 18.0, 18.0], vec![-66.0, -65.0, -66.0], vec![]));
    Assert::assert_true(expected.contains("at least 4 polygon points required"));
    Ok(())
}

/// polygon needs same number of latitudes as longitudes
///
/// Port of `TestPolygon.testPolygonBogus`.
fn test_polygon_bogus() -> Result<()> {
    let expected = iae_message(|| {
        Polygon::new(
            vec![18.0, 18.0, 19.0, 19.0],
            vec![-66.0, -65.0, -65.0, -66.0, -66.0],
            vec![],
        )
    });
    Assert::assert_true(expected.contains("must be equal length"));
    Ok(())
}

/// polygon must be closed
///
/// Port of `TestPolygon.testPolygonNotClosed`.
fn test_polygon_not_closed() -> Result<()> {
    let expected = iae_message(|| {
        Polygon::new(
            vec![18.0, 18.0, 19.0, 19.0, 19.0],
            vec![-66.0, -65.0, -65.0, -66.0, -67.0],
            vec![],
        )
    });
    assert_contains(&expected, "it must close itself");
    Ok(())
}

/// Port of `TestPolygon.testGeoJSONPolygon`.
fn test_geo_json_polygon() -> Result<()> {
    let b = r#"{
  "type": "Polygon",
  "coordinates": [
    [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
      [100.0, 1.0], [100.0, 0.0] ]
  ]
}
"#;
    let polygons = from_geo_json(b)?;
    assert_equals(&1, &polygons.len());
    assert_equals(
        &Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?,
        &polygons[0],
    );
    Ok(())
}

/// Port of `TestPolygon.testGeoJSONPolygonWithHole`.
fn test_geo_json_polygon_with_hole() -> Result<()> {
    let b = r#"{
  "type": "Polygon",
  "coordinates": [
    [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
      [100.0, 1.0], [100.0, 0.0] ],
    [ [100.5, 0.5], [100.5, 0.75], [100.75, 0.75], [100.75, 0.5], [100.5, 0.5]]
  ]
}
"#;
    let hole = Polygon::new(
        vec![0.5, 0.75, 0.75, 0.5, 0.5],
        vec![100.5, 100.5, 100.75, 100.75, 100.5],
        vec![],
    )?;
    let expected = Polygon::new(
        vec![0.0, 0.0, 1.0, 1.0, 0.0],
        vec![100.0, 101.0, 101.0, 100.0, 100.0],
        vec![hole],
    )?;
    let polygons = from_geo_json(b)?;
    assert_equals(&1, &polygons.len());
    assert_equals(&expected, &polygons[0]);
    Ok(())
}

/// a MultiPolygon returns multiple Polygons
///
/// Port of `TestPolygon.testGeoJSONMultiPolygon`.
fn test_geo_json_multi_polygon() -> Result<()> {
    let b = r#"{
  "type": "MultiPolygon",
  "coordinates": [
    [
      [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
        [100.0, 1.0], [100.0, 0.0] ]
    ],
    [
      [ [10.0, 2.0], [11.0, 2.0], [11.0, 3.0],
        [10.0, 3.0], [10.0, 2.0] ]
    ]
  ],
}
"#;
    let polygons = from_geo_json(b)?;
    assert_equals(&2, &polygons.len());
    assert_equals(
        &Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?,
        &polygons[0],
    );
    assert_equals(
        &Polygon::new(
            vec![2.0, 2.0, 3.0, 3.0, 2.0],
            vec![10.0, 11.0, 11.0, 10.0, 10.0],
            vec![],
        )?,
        &polygons[1],
    );
    Ok(())
}

/// make sure type can appear last (JSON allows arbitrary key/value order for
/// objects)
///
/// Port of `TestPolygon.testGeoJSONTypeComesLast`.
fn test_geo_json_type_comes_last() -> Result<()> {
    let b = r#"{
  "coordinates": [
    [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
      [100.0, 1.0], [100.0, 0.0] ]
  ],
  "type": "Polygon",
}
"#;
    let polygons = from_geo_json(b)?;
    assert_equals(&1, &polygons.len());
    assert_equals(
        &Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?,
        &polygons[0],
    );
    Ok(())
}

/// make sure Polygon inside a type: Feature also works
///
/// Port of `TestPolygon.testGeoJSONPolygonFeature`.
fn test_geo_json_polygon_feature() -> Result<()> {
    let b = r#"{ "type": "Feature",
  "geometry": {
    "type": "Polygon",
    "coordinates": [
      [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
        [100.0, 1.0], [100.0, 0.0] ]
      ]
  },
  "properties": {
    "prop0": "value0",
    "prop1": {"this": "that"}
  }
}
"#;
    let polygons = from_geo_json(b)?;
    assert_equals(&1, &polygons.len());
    assert_equals(
        &Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?,
        &polygons[0],
    );
    Ok(())
}

/// make sure MultiPolygon inside a type: Feature also works
///
/// Port of `TestPolygon.testGeoJSONMultiPolygonFeature`.
fn test_geo_json_multi_polygon_feature() -> Result<()> {
    let b = r#"{ "type": "Feature",
  "geometry": {
      "type": "MultiPolygon",
      "coordinates": [
        [
          [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
            [100.0, 1.0], [100.0, 0.0] ]
        ],
        [
          [ [10.0, 2.0], [11.0, 2.0], [11.0, 3.0],
            [10.0, 3.0], [10.0, 2.0] ]
        ]
      ]
  },
  "properties": {
    "prop0": "value0",
    "prop1": {"this": "that"}
  }
}
"#;
    let polygons = from_geo_json(b)?;
    assert_equals(&2, &polygons.len());
    assert_equals(
        &Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?,
        &polygons[0],
    );
    assert_equals(
        &Polygon::new(
            vec![2.0, 2.0, 3.0, 3.0, 2.0],
            vec![10.0, 11.0, 11.0, 10.0, 10.0],
            vec![],
        )?,
        &polygons[1],
    );
    Ok(())
}

/// FeatureCollection with one geometry is allowed:
///
/// Port of `TestPolygon.testGeoJSONFeatureCollectionWithSinglePolygon`.
fn test_geo_json_feature_collection_with_single_polygon() -> Result<()> {
    let b = r#"{ "type": "FeatureCollection",
  "features": [
    { "type": "Feature",
      "geometry": {
        "type": "Polygon",
        "coordinates": [
          [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
            [100.0, 1.0], [100.0, 0.0] ]
          ]
      },
      "properties": {
        "prop0": "value0",
        "prop1": {"this": "that"}
      }
    }
  ]
}    
"#;
    let expected = Polygon::new(
        vec![0.0, 0.0, 1.0, 1.0, 0.0],
        vec![100.0, 101.0, 101.0, 100.0, 100.0],
        vec![],
    )?;
    let actual = from_geo_json(b)?;
    assert_equals(&1, &actual.len());
    assert_equals(&expected, &actual[0]);
    Ok(())
}

/// stuff after the object is not allowed
///
/// Port of `TestPolygon.testIllegalGeoJSONExtraCrapAtEnd`.
fn test_illegal_geo_json_extra_crap_at_end() -> Result<()> {
    let b = r#"{
  "type": "Polygon",
  "coordinates": [
    [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
      [100.0, 1.0], [100.0, 0.0] ]
  ]
}
foo
"#;
    let e = expect_parse_exception(b);
    Assert::assert_true(e.contains("unexpected character 'f' after end of GeoJSON object"));
    Ok(())
}

/// Port of `TestPolygon.testIllegalGeoJSONLinkedCRS`.
fn test_illegal_geo_json_linked_crs() -> Result<()> {
    let b = r#"{
  "type": "Polygon",
  "coordinates": [
    [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
      [100.0, 1.0], [100.0, 0.0] ]
  ],
  "crs": {
    "type": "link",
    "properties": {
      "href": "http://example.com/crs/42",
      "type": "proj4"
    }
  }    
}
"#;
    let e = expect_parse_exception(b);
    Assert::assert_true(e.contains("cannot handle linked crs"));
    Ok(())
}

/// FeatureCollection with more than one geometry is not supported:
///
/// Port of `TestPolygon.testIllegalGeoJSONMultipleFeatures`.
fn test_illegal_geo_json_multiple_features() -> Result<()> {
    let b = r#"{ "type": "FeatureCollection",
  "features": [
    { "type": "Feature",
      "geometry": {"type": "Point", "coordinates": [102.0, 0.5]},
      "properties": {"prop0": "value0"}
    },
    { "type": "Feature",
      "geometry": {
      "type": "LineString",
      "coordinates": [
        [102.0, 0.0], [103.0, 1.0], [104.0, 0.0], [105.0, 1.0]
        ]
      },
      "properties": {
        "prop0": "value0",
        "prop1": 0.0
      }
    },
    { "type": "Feature",
      "geometry": {
        "type": "Polygon",
        "coordinates": [
          [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
            [100.0, 1.0], [100.0, 0.0] ]
          ]
      },
      "properties": {
        "prop0": "value0",
        "prop1": {"this": "that"}
      }
    }
  ]
}    
"#;
    let e = expect_parse_exception(b);
    Assert::assert_true(e.contains(
        "can only handle type FeatureCollection (if it has a single polygon geometry), Feature, Polygon or MultiPolygon, but got Point",
    ));
    Ok(())
}

/// Port of `TestPolygon.testPolygonPropertiesCanBeStringArrays`.
fn test_polygon_properties_can_be_string_arrays() -> Result<()> {
    let b = r#"{
  "type": "Polygon",
  "coordinates": [
    [ [100.0, 0.0], [101.0, 0.0], [101.0, 1.0],
      [100.0, 1.0], [100.0, 0.0] ]
  ],
  "properties": {
    "array": [ "value" ]
  }
}
"#;
    let polygons = from_geo_json(b)?;
    assert_equals(&1, &polygons.len());
    Ok(())
}
