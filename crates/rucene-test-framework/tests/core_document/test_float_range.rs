//! Port of `lucene/core/src/test/org/apache/lucene/document/TestFloatRange.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::range_fields::FloatRange;
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestFloatRange`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestFloatRange",
        vec![test("testToString", test_to_string)],
    )
}

/// Port of `TestFloatRange.testToString`.
fn test_to_string() -> Result<()> {
    let range = FloatRange::new("foo", &[0.1f32, 1.1, 2.1, 3.1], &[0.2f32, 1.2, 2.2, 3.2])?;
    assert_equals(
        "FloatRange <foo: [0.1 : 0.2] [1.1 : 1.2] [2.1 : 2.2] [3.1 : 3.2]>",
        range.to_string().as_str(),
    );
    Ok(())
}
