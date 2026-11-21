//! The `lucene/codecs` test class that extends `BaseFieldInfoFormatTestCase`
//! (rmp #360, the #355 slice), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextFieldInfoFormat`, whose
//! only member is `private final Codec codec = new SimpleTextCodec()`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_field_info_format_test_case::{
    self, BaseFieldInfoFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextFieldInfoFormat`.
struct TestSimpleTextFieldInfoFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextFieldInfoFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.field_infos_add_random_fields(doc);
    }

    fn as_field_infos(&self) -> Option<&dyn BaseFieldInfoFormatTestCase> {
        Some(self)
    }
}

impl BaseFieldInfoFormatTestCase for TestSimpleTextFieldInfoFormat {}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextFieldInfoFormat")
        .extends(&base_field_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextFieldInfoFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .build()
}

#[test]
fn test_simple_text_field_info_format() {
    run_suite(simple_text_class);
}
