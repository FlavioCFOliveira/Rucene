//! The `lucene/codecs` test class that extends `BaseCompoundFormatTestCase`
//! (rmp #360, the #352 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextCompoundFormat`:
//! `private final Codec codec = new SimpleTextCodec()`, and the overrides of
//! `testCorruptFilesAreCaught()`, `testMissingCodecHeadersAreCaught()` and
//! `testCheckIntegrity()` with empty bodies (SimpleText checks no checksums).
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_compound_format_test_case::{
    self, BaseCompoundFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextCompoundFormat`.
struct TestSimpleTextCompoundFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextCompoundFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.compound_add_random_fields(doc);
    }

    fn as_compound(&self) -> Option<&dyn BaseCompoundFormatTestCase> {
        Some(self)
    }
}

impl BaseCompoundFormatTestCase for TestSimpleTextCompoundFormat {}

/// An overriding test whose body is empty: `public void <name>() {}`.
fn empty_test(name: &str) -> Method {
    Method::new(name).body(|_: &mut FormatTestInstance| {})
}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextCompoundFormat")
        .extends(&base_compound_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextCompoundFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .method(empty_test("testCorruptFilesAreCaught"))
        .method(empty_test("testMissingCodecHeadersAreCaught"))
        .method(empty_test("testCheckIntegrity"))
        .build()
}

#[test]
fn test_simple_text_compound_format() {
    run_suite(simple_text_class);
}
