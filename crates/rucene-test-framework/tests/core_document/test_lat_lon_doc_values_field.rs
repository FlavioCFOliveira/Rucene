//! Port of `lucene/core/src/test/org/apache/lucene/document/TestLatLonDocValuesField.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::geo_fields::LatLonDocValuesField;
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestLatLonDocValuesField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestLatLonDocValuesField",
        vec![test("testToString", test_to_string)],
    )
}

/// Port of `TestLatLonDocValuesField.testToString`.
fn test_to_string() -> Result<()> {
    // looks crazy due to lossiness
    assert_equals(
        "LatLonDocValuesField <field:18.313693958334625,-65.22744401358068>",
        LatLonDocValuesField::new("field", 18.313694, -65.227444)?
            .to_string()
            .as_str(),
    );

    // sort field
    assert_equals(
        "<distance:\"field\" latitude=18.0 longitude=19.0>",
        LatLonDocValuesField::new_distance_sort("field", 18.0, 19.0)?
            .to_string()
            .as_str(),
    );
    Ok(())
}
