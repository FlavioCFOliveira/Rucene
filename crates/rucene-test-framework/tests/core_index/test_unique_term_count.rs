//! Port of `lucene/core/src/test/org/apache/lucene/index/TestUniqueTermCount.java`.
//!
//! Tests the uniqueTermCount statistic in FieldInvertState

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{FieldInvertState, IndexReader};
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, random};
use crate::support::{field_mut, npe};

/// `TestUniqueTermCount`: its fields `dir`, `reader` and `expected`, set by
/// `setUp()`.
#[derive(Default)]
struct TestUniqueTermCountInstance {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn IndexReader>>,
    /// expected uniqueTermCount values for our documents
    expected: Vec<i32>,
}

/// `TestUniqueTermCount`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestUniqueTermCount")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestUniqueTermCountInstance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut TestUniqueTermCountInstance| {
                set_up(inst).unwrap_or_else(|e| rethrow_java(e))
            },
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut TestUniqueTermCountInstance| {
                    tear_down(inst).unwrap_or_else(|e| rethrow_java(e))
                }),
        )
        .method(Method::new("test").throws(&["java.lang.Exception"]).body(
            |inst: &mut TestUniqueTermCountInstance| test(inst).unwrap_or_else(|e| rethrow_java(e)),
        ))
        .build()
}

/// `setUp()`.
fn set_up(inst: &mut TestUniqueTermCountInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    let dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::with_automaton(&*random(), Arc::clone(&SIMPLE), true);
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    config.set_similarity(Some(Arc::new(TestSimilarity) as Arc<dyn Similarity>))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "foo",
        "",
        Store::NO,
    )));
    for _ in 0..100 {
        let value = add_value(inst);
        field_mut(&mut doc, "foo").set_string_value(value)?;
        writer.add_document(&doc)?;
    }
    inst.reader = Some(writer.get_reader()?);
    writer.close()?;
    inst.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
fn tear_down(inst: &mut TestUniqueTermCountInstance) -> Result<()> {
    inst.reader
        .take()
        .unwrap_or_else(|| npe("reader"))
        .close()?;
    inst.dir.take().unwrap_or_else(|| npe("dir")).close()?;
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Port of `TestUniqueTermCount.test`.
fn test(inst: &mut TestUniqueTermCountInstance) -> Result<()> {
    let reader = inst.reader.as_ref().unwrap_or_else(|| npe("reader"));
    let foo_norms = MultiDocValues::get_norm_values(reader, "foo")?;
    assert_not_null(&foo_norms);
    let mut foo_norms = foo_norms.expect("checked");
    for i in 0..reader.max_doc() {
        assert_equals(&i, &foo_norms.next_doc()?);
        assert_equals(
            &i64::from(inst.expected[i as usize]),
            &foo_norms.long_value()?,
        );
    }
    Ok(())
}

/// Makes a bunch of single-char tokens (the max # unique terms will at most be 26). puts the #
/// unique terms into expected, to be checked against the norm.
fn add_value(inst: &mut TestUniqueTermCountInstance) -> String {
    let mut sb = String::new();
    let mut terms = HashSet::new();
    let num = TestUtil::next_int(&*random(), 0, 255);
    for _ in 0..num {
        sb.push(' ');
        let term = char::from_u32(TestUtil::next_int(&*random(), 'a' as i32, 'z' as i32) as u32)
            .expect("ASCII letter");
        sb.push(term);
        terms.insert(term.to_string());
    }
    inst.expected.push(terms.len() as i32);
    sb
}

/// Simple similarity that encodes maxTermFrequency directly
#[derive(Debug)]
struct TestSimilarity;

impl std::fmt::Display for TestSimilarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestSimilarity")
    }
}

impl Similarity for TestSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestUniqueTermCount$TestSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        Ok(i64::from(state.unique_term_count()))
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }
}
