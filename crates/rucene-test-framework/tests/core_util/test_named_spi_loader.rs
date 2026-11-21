//! Port of `lucene/core/src/test/org/apache/lucene/util/TestNamedSPILoader.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs;
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, test};

/// `TestNamedSPILoader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestNamedSPILoader",
        vec![
            test("testLookup", test_lookup),
            test("testBogusLookup", test_bogus_lookup),
            test("testAvailableServices", test_available_services),
        ],
    )
}

fn test_lookup() -> Result<()> {
    let current_name = TestUtil::get_default_codec().name().to_string();
    // PENDING(member): `Codec.forName(String)` raising
    // `IllegalArgumentException` is `rucene::codecs::lookup`, which is
    // `pub(crate)` in `src/codecs.rs` (out of this pass's directories); the
    // public `for_name` returns `None` instead, so a missing codec fails here
    // with Java's `NullPointerException` on `codec.getName()`.
    let codec = codecs::for_name(&current_name);
    let name = match &codec {
        Some(c) => c.name().to_string(),
        None => randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NullPointerException",
            "codec is null".to_string(),
        )
        .throw(),
    };
    assert_equals(&current_name, &name);
    Ok(())
}

/// Port of `testBogusLookup`: we want an exception if it's not found
/// (`Codec.forName`, which is `rucene::codecs::lookup`).
fn test_bogus_lookup() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        codecs::lookup("dskfdskfsdfksdfdsf")
    });
    Ok(())
}

fn test_available_services() -> Result<()> {
    let codecs = codecs::available_codecs();
    let default_name = TestUtil::get_default_codec().name().to_string();
    Assert::assert_true(codecs.contains(&default_name));
    Ok(())
}
