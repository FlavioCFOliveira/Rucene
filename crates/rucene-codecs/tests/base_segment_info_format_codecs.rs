//! The `lucene/codecs` test class that extends `BaseSegmentInfoFormatTestCase`
//! (rmp #360, the #355 slice), with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextSegmentInfoFormat`,
//! whose `getVersions()` returns `{Version.LATEST}` and whose codec is
//! `private final Codec codec = new SimpleTextCodec()`. Its own test method
//! (`testFileIsUTF8`) belongs to the codecs-test port (sprint 7) and is not
//! declared here.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::util::Version;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_segment_info_format_test_case::{
    self, BaseSegmentInfoFormatTestCase,
};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestSimpleTextSegmentInfoFormat`.
struct TestSimpleTextSegmentInfoFormat {
    codec: Arc<dyn Codec>,
}

impl BaseIndexFileFormatTestCase for TestSimpleTextSegmentInfoFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.segment_infos_add_random_fields(doc);
    }

    fn as_segment_infos(&self) -> Option<&dyn BaseSegmentInfoFormatTestCase> {
        Some(self)
    }
}

impl BaseSegmentInfoFormatTestCase for TestSimpleTextSegmentInfoFormat {
    fn get_versions(&self) -> Vec<Version> {
        vec![Version::LATEST]
    }
}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextSegmentInfoFormat")
        .extends(&base_segment_info_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextSegmentInfoFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .build()
}

#[test]
fn test_simple_text_segment_info_format() {
    run_suite(simple_text_class);
}
