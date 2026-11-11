//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLatLonPoint.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::geo_fields::LatLonPoint;
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestLatLonPoint`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLatLonPoint",
        vec![test("testToString", test_to_string)],
    )
}

/// Port of `TestLatLonPoint.testToString`.
fn test_to_string() -> Result<()> {
    // looks crazy due to lossiness
    assert_equals(
        "LatLonPoint <field:18.313693958334625,-65.22744401358068>",
        LatLonPoint::new("field", 18.313694, -65.227444)?
            .to_string()
            .as_str(),
    );

    // looks crazy due to lossiness
    assert_equals(
        "field:[18.000000016763806 TO 18.999999999068677],[-65.9999999217689 TO -65.00000006519258]",
        LatLonPoint::new_box_query(Some("field"), 18.0, 19.0, -66.0, -65.0)?
            .to_query_string("")
            .as_str(),
    );

    // distance query does not quantize inputs
    assert_equals(
        "field:18.0,19.0 +/- 25.0 meters",
        LatLonPoint::new_distance_query(Some("field"), 18.0, 19.0, 25.0)?
            .to_query_string("")
            .as_str(),
    );
    Ok(())
}
