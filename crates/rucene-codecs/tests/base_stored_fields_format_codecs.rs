//! The `lucene/codecs` test class that extends `BaseStoredFieldsFormatTestCase`
//! (rmp #360, the #351 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextStoredFieldsFormat`,
//! whose only member is `getCodec()` (`new SimpleTextCodec()`).
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_stored_fields_format_test_case::{
    self, BaseStoredFieldsFormatTestCase,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextStoredFieldsFormat`.
struct TestSimpleTextStoredFieldsFormat;

impl BaseIndexFileFormatTestCase for TestSimpleTextStoredFieldsFormat {
    /// `getCodec()`: `new SimpleTextCodec()`.
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::new(SimpleTextCodec::new())
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.stored_fields_add_random_fields(doc);
    }

    fn as_stored_fields(&self) -> Option<&dyn BaseStoredFieldsFormatTestCase> {
        Some(self)
    }
}

impl BaseStoredFieldsFormatTestCase for TestSimpleTextStoredFieldsFormat {}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextStoredFieldsFormat")
        .extends(&base_stored_fields_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextStoredFieldsFormat))
        }))
        .build()
}

#[test]
fn test_simple_text_stored_fields_format() {
    run_suite(simple_text_class);
}
