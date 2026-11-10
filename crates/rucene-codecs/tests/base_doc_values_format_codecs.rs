//! The `lucene/codecs` test class that extends `BaseDocValuesFormatTestCase`
//! (rmp #360, the #353 slice), with the cases it inherits:
//! `org.apache.lucene.codecs.simpletext.TestSimpleTextDocValuesFormat`
//! (`private final Codec codec = new SimpleTextCodec()`), with its own test
//! method `testFileIsUTF8`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::Codec;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedSetDocValuesField,
};
use rucene::error::Result;
use rucene::index::Term;
use rucene::store::{DataInput, IndexInput};
use rucene::util::BytesRef;
use rucene_codecs::simpletext::codec::SimpleTextCodec;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::base_doc_values_format_test_case::{
    self, BaseDocValuesFormatTestCase,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::legacy_base_doc_values_format_test_case::LegacyBaseDocValuesFormatTestCase;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

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

/// `TestSimpleTextDocValuesFormat.testFileIsUTF8()`.
fn test_file_is_utf8() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*LuceneTestCase::random()),
        )));
        {
            let writer = RandomIndexWriter::new_with_config(
                &*LuceneTestCase::random(),
                dir.as_directory(),
                conf,
            )?;
            let added = (|| -> Result<()> {
                for _ in 0..100 {
                    // `List.of(...)`: the four fields in Java's order.
                    let mut doc = Document::new();
                    doc.add(Box::new(SortedDocValuesField::new(
                        "sortedVal",
                        LuceneTestCase::new_bytes_ref_str(&TestUtil::random_simple_string(
                            &*LuceneTestCase::random(),
                        )),
                    )));
                    doc.add(Box::new(SortedSetDocValuesField::new(
                        "sortedSetVal",
                        LuceneTestCase::new_bytes_ref_str(&TestUtil::random_simple_string(
                            &*LuceneTestCase::random(),
                        )),
                    )));
                    doc.add(Box::new(NumericDocValuesField::new(
                        "numberVal",
                        LuceneTestCase::random().next_long(),
                    )));
                    doc.add(Box::new(BinaryDocValuesField::new(
                        "binaryVal",
                        TestUtil::random_binary_term(&*LuceneTestCase::random()),
                    )));
                    writer.add_document(&doc)?;
                }
                Ok(())
            })();
            added.and(writer.close())?;
        }
        for file in dir.as_directory().list_all()? {
            if file.ends_with("dat") {
                let mut input = dir.as_directory().open_checksum_input(&file)?;
                let length = input.length();
                if length > 20_000 {
                    // Avoid allocating a huge array if the length is wrong
                    Assert::fail("Doc values should not be this large");
                }
                let mut bytes = vec![0u8; length as usize];
                let len = bytes.len();
                input.read_bytes(Some(&mut bytes), 0, len)?;
                let bytes_ref = BytesRef::new(bytes);
                Assert::assert_true(bytes_ref.to_string() != Term::to_string_bytes_ref(&bytes_ref));
                input.close()?;
            }
        }
        Ok(())
    })();
    result.and(dir.close())
}

fn simple_text_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.simpletext.TestSimpleTextDocValuesFormat")
        .extends(&base_doc_values_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestSimpleTextDocValuesFormat {
                codec: Arc::new(SimpleTextCodec::new()),
            }))
        }))
        .method(
            Method::new("testFileIsUTF8")
                .throws(&["java.io.IOException"])
                .body(|_: &mut FormatTestInstance| {
                    test_file_is_utf8().unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .build()
}

#[test]
fn test_simple_text_doc_values_format() {
    run_suite(simple_text_class);
}
