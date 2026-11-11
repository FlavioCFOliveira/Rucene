//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestGeoEncodingUtils.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::geo::{GeoEncodingUtils, GeoUtils};
use rucene::util::NumericUtils;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestGeoEncodingUtils`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestGeoEncodingUtils",
        vec![
            test("testLatitudeQuantization", test_latitude_quantization),
            test("testLongitudeQuantization", test_longitude_quantization),
            test("testEncodeEdgeCases", test_encode_edge_cases),
        ],
    )
}

/// The encoders and decoder of one dimension.
struct Dim {
    min_incl: f64,
    decode_step: f64,
    decode: fn(i32) -> f64,
    encode: fn(f64) -> Result<i32>,
    encode_ceil: fn(f64) -> Result<i32>,
}

/// The shared body of `testLatitudeQuantization` and
/// `testLongitudeQuantization`, which Java writes out twice.
fn quantization(d: &Dim) -> Result<()> {
    let random = random();
    for _i in 0..10000 {
        let encoded = random.next_int();
        let min = d.min_incl + (encoded as i64 - i32::MIN as i64) as f64 * d.decode_step;
        let decoded = (d.decode)(encoded);
        // should exactly equal expected value
        assert_equals_delta(min, decoded, 0.0);
        // should round-trip
        assert_equals(&encoded, &(d.encode)(decoded)?);
        assert_equals(&encoded, &(d.encode_ceil)(decoded)?);
        // test within the range
        if encoded != i32::MAX {
            // this is the next representable value
            // all double values between [min .. max) should encode to the current integer
            // all double values between (min .. max] should encodeCeil to the next integer.
            let max = min + d.decode_step;
            assert_equals_delta(max, (d.decode)(encoded + 1), 0.0);
            assert_equals(&(encoded + 1), &(d.encode)(max)?);
            assert_equals(&(encoded + 1), &(d.encode_ceil)(max)?);

            // first and last doubles in range that will be quantized
            let min_edge = min.next_up();
            let max_edge = max.next_down();
            assert_equals(&encoded, &(d.encode)(min_edge)?);
            assert_equals(&(encoded + 1), &(d.encode_ceil)(min_edge)?);
            assert_equals(&encoded, &(d.encode)(max_edge)?);
            assert_equals(&(encoded + 1), &(d.encode_ceil)(max_edge)?);

            // check random values within the double range
            let min_bits = NumericUtils::double_to_sortable_long(min_edge);
            let max_bits = NumericUtils::double_to_sortable_long(max_edge);
            for _j in 0..100 {
                let value = NumericUtils::sortable_long_to_double(TestUtil::next_long(
                    &*random, min_bits, max_bits,
                ));
                // round down
                assert_equals(&encoded, &(d.encode)(value)?);
                // round up
                assert_equals(&(encoded + 1), &(d.encode_ceil)(value)?);
            }
        }
    }
    Ok(())
}

/// step through some integers, ensuring they decode to their expected double
/// values. double values start at -90 and increase by LATITUDE_DECODE for
/// each integer. check edge cases within the double range and random doubles
/// within the range too.
///
/// Port of `TestGeoEncodingUtils.testLatitudeQuantization`.
fn test_latitude_quantization() -> Result<()> {
    quantization(&Dim {
        min_incl: GeoUtils::MIN_LAT_INCL,
        decode_step: 180.0 / (1i64 << 32) as f64,
        decode: GeoEncodingUtils::decode_latitude,
        encode: GeoEncodingUtils::encode_latitude,
        encode_ceil: GeoEncodingUtils::encode_latitude_ceil,
    })
}

/// step through some integers, ensuring they decode to their expected double
/// values. double values start at -180 and increase by LONGITUDE_DECODE for
/// each integer. check edge cases within the double range and a random
/// doubles within the range too.
///
/// Port of `TestGeoEncodingUtils.testLongitudeQuantization`.
fn test_longitude_quantization() -> Result<()> {
    quantization(&Dim {
        min_incl: GeoUtils::MIN_LON_INCL,
        decode_step: 360.0 / (1i64 << 32) as f64,
        decode: GeoEncodingUtils::decode_longitude,
        encode: GeoEncodingUtils::encode_longitude,
        encode_ceil: GeoEncodingUtils::encode_longitude_ceil,
    })
}

/// check edge/interesting cases explicitly
///
/// Port of `TestGeoEncodingUtils.testEncodeEdgeCases`.
fn test_encode_edge_cases() -> Result<()> {
    assert_equals(
        &i32::MIN,
        &GeoEncodingUtils::encode_latitude(GeoUtils::MIN_LAT_INCL)?,
    );
    assert_equals(
        &i32::MIN,
        &GeoEncodingUtils::encode_latitude_ceil(GeoUtils::MIN_LAT_INCL)?,
    );
    assert_equals(
        &i32::MAX,
        &GeoEncodingUtils::encode_latitude(GeoUtils::MAX_LAT_INCL)?,
    );
    assert_equals(
        &i32::MAX,
        &GeoEncodingUtils::encode_latitude_ceil(GeoUtils::MAX_LAT_INCL)?,
    );

    assert_equals(
        &i32::MIN,
        &GeoEncodingUtils::encode_longitude(GeoUtils::MIN_LON_INCL)?,
    );
    assert_equals(
        &i32::MIN,
        &GeoEncodingUtils::encode_longitude_ceil(GeoUtils::MIN_LON_INCL)?,
    );
    assert_equals(
        &i32::MAX,
        &GeoEncodingUtils::encode_longitude(GeoUtils::MAX_LON_INCL)?,
    );
    assert_equals(
        &i32::MAX,
        &GeoEncodingUtils::encode_longitude_ceil(GeoUtils::MAX_LON_INCL)?,
    );
    Ok(())
}
