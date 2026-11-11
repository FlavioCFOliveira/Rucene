//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMaxTermFrequency.java`.
//!
//! The document's field `foo` is changed through the document
//! (`field_mut(...).set_string_value`), as Java changes the instance it
//! added.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::util::shuffle;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{FieldInvertState, IndexReader};
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene::store::Directory;
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, random};
use crate::support::{field_mut, npe};

/// The instance fields of `TestMaxTermFrequency`.
#[derive(Default)]
struct Instance {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn IndexReader>>,
    /// expected maxTermFrequency values for our documents
    expected: Vec<i32>,
}

fn ti(name: &str, body: fn(&Instance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// `TestMaxTermFrequency`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestMaxTermFrequency")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut Instance| {
                LuceneTestCase::super_set_up();
                set_up(inst).unwrap_or_else(|e| rethrow_java(e));
            },
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut Instance| {
                    tear_down(inst).unwrap_or_else(|e| rethrow_java(e));
                    LuceneTestCase::super_tear_down();
                }),
        )
        .method(ti("test", test))
        .build()
}

/// `setUp()`.
fn set_up(this: &mut Instance) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let directory: Arc<dyn Directory> = dir.as_directory();
    this.dir = Some(dir);
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::with_automaton(&*random(), Arc::clone(&SIMPLE), true),
    )));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    config.set_similarity(Some(Arc::new(TestSimilarity) as Arc<dyn Similarity>))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory, config)?;
    let mut doc = Document::new();
    #[allow(clippy::disallowed_names)] // the Java test's variable name
    let foo = LuceneTestCase::new_text_field("foo", "", Store::NO);
    doc.add(Box::new(foo));
    for _ in 0..100 {
        let value = add_value(this);
        field_mut(&mut doc, "foo").set_string_value(value)?;
        writer.add_document(&doc)?;
    }
    this.reader = Some(writer.get_reader()?);
    writer.close()
}

/// `tearDown()`.
fn tear_down(this: &mut Instance) -> Result<()> {
    this.reader
        .take()
        .unwrap_or_else(|| npe("reader"))
        .close()?;
    this.dir.take().unwrap_or_else(|| npe("dir")).close()
}

/// Port of `TestMaxTermFrequency.test`.
fn test(this: &Instance) -> Result<()> {
    let reader = this.reader.as_ref().unwrap_or_else(|| npe("reader"));
    let mut foo_norms =
        MultiDocValues::get_norm_values(reader, "foo")?.unwrap_or_else(|| npe("fooNorms"));
    for i in 0..reader.max_doc() {
        assert_equals(&i, &foo_norms.next_doc()?);
        assert_equals(
            &i64::from(this.expected[i as usize]),
            &(foo_norms.long_value()? & 0xff),
        );
    }
    Ok(())
}

/// Makes a bunch of single-char tokens (the max freq will at most be 255). shuffles them around,
/// and returns the whole list with Arrays.toString(). This works fine because we use
/// lettertokenizer. puts the max-frequency term into expected, to be checked against the norm.
///
/// Port of `TestMaxTermFrequency.addValue` (`private`).
fn add_value(this: &mut Instance) -> String {
    let mut terms: Vec<String> = Vec::new();
    let max_ceiling = TestUtil::next_int(&*random(), 0, 255);
    let mut max = 0;
    for ch in 'a'..='z' {
        let num = TestUtil::next_int(&*random(), 0, max_ceiling);
        for _ in 0..num {
            terms.push(ch.to_string());
        }
        max = max.max(num);
    }
    this.expected.push(max);
    shuffle(&mut terms, &*random());
    format!("[{}]", terms.join(", "))
}

/// Simple similarity that encodes maxTermFrequency directly as a byte
///
/// `static class TestSimilarity extends Similarity`.
#[derive(Debug)]
struct TestSimilarity;

/// The anonymous `SimScorer` of `TestSimilarity.scorer`.
struct ZeroSimScorer;

impl SimScorer for ZeroSimScorer {
    fn score(&self, _freq: f32, _norm: i64) -> f32 {
        0.0
    }
}

impl Similarity for TestSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestMaxTermFrequency$TestSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        Ok(i64::from(state.max_term_frequency()))
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        Box::new(ZeroSimScorer)
    }
}
