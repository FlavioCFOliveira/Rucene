//! The `lucene/codecs` test class that extends `BaseDocValuesFormatTestCase`
//! (rmp #360, the #353 slice), with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextDocValuesFormat`
//! (`private final Codec codec = new SimpleTextCodec()`). Its own test method
//! (`testFileIsUTF8`) belongs to the codecs-test port (sprint 7) and is not
//! declared here.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_doc_values_format_test_case::{
    self, BaseDocValuesFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::legacy_base_doc_values_format_test_case::LegacyBaseDocValuesFormatTestCase;

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextDocValuesFormat`.
struct TestSimpleTextDocValuesFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextDocValuesFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.doc_values_add_random_fields(doc);
    }

    fn as_doc_values(&self) -> Option<&dyn LegacyBaseDocValuesFormatTestCase> {
        Some(self)
    }
}

impl LegacyBaseDocValuesFormatTestCase for TestSimpleTextDocValuesFormat {
    fn as_base_doc_values(&self) -> Option<&dyn BaseDocValuesFormatTestCase> {
        Some(self)
    }
}

impl BaseDocValuesFormatTestCase for TestSimpleTextDocValuesFormat {}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextDocValuesFormat")
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextDocValuesFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .build()
}

#[test]
fn test_simple_text_doc_values_format() {
    run_suite(simple_text_class);
}
