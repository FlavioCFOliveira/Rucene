//! The `lucene/codecs` test class that extends `BaseSegmentInfoFormatTestCase`
//! (rmp #360, the #355 slice), with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextSegmentInfoFormat`,
//! whose `getVersions()` returns `{Version.LATEST}` and whose codec is
//! `private final Codec codec = new SimpleTextCodec()`, with its own test
//! method `testFileIsUTF8`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use std::collections::HashMap;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_file_names::segment_file_name;
use rucene::index::segment_info::SegmentInfo;
use rucene::index::Term;
use rucene::store::{IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{BytesRef, Version};
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_codecs::simpletext::segment_info_format::SI_EXTENSION;
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_segment_info_format_test_case::{
    self, BaseSegmentInfoFormatTestCase,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

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

impl TestSimpleTextSegmentInfoFormat {
    /// `testFileIsUTF8()`. Java's `null` index sort is `Sort::default()`,
    /// as in `BaseSegmentInfoFormatTestCase`.
    fn test_file_is_utf8(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        let codec = self.get_codec();
        let id = StringHelper::random_id();
        let info = SegmentInfo::new(
            dir.as_directory(),
            self.get_versions()[0],
            Some(self.get_versions()[0]),
            "_123".to_string(),
            1,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            id,
            HashMap::new(),
            None,
        )?;
        info.set_files(std::iter::empty::<String>())?;
        codec
            .segment_info_format()
            .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;
        let seg_file_name = segment_file_name("_123", "", SI_EXTENSION);
        {
            let mut input = dir.as_directory().open_checksum_input(&seg_file_name)?;
            let length = input.length();
            if length > 5_000 {
                // Avoid allocating a huge array if the length is wrong
                Assert::fail("SegmentInfos should not be this large");
            }
            let bytes = vec![0u8; length as usize];
            let bytes_ref = BytesRef::new(bytes);
            // If the following are equal, it means the bytes were not well-formed UTF8.
            Assert::assert_true(bytes_ref.to_string() != Term::to_string_bytes_ref(&bytes_ref));
            input.close()?;
        }
        dir.close()
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
        .method(
            Method::new("testFileIsUTF8")
                .throws(&["java.io.IOException"])
                .body(|inst: &mut FormatTestInstance| {
                    inst.subclass::<TestSimpleTextSegmentInfoFormat>()
                        .test_file_is_utf8()
                        .unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .build()
}

#[test]
fn test_simple_text_segment_info_format() {
    run_suite(simple_text_class);
}
