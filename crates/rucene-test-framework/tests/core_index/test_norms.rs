//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNorms.java`.
//!
//! Java's `MySimProvider.get` returns a new `ByteEncodingBoostSimilarity`
//! per call; a Rucene scorer borrows its similarity (see
//! `PerFieldSimilarityWrapper`), so the provider owns the one instance it
//! hands out.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{FieldInvertState, IndexReader};
use rucene::search::similarities::per_field_similarity_wrapper::PerFieldSimilarityWrapper;
use rucene::search::similarities::{
    per_field_compute_norm, per_field_scorer, ClassicSimilarity, CollectionStatistics, SimScorer,
    Similarity, TermStatistics,
};
use rucene::store::Directory;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, class, random, test};
use crate::support::npe;

/// `TestNorms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestNorms",
        vec![
            test("testMaxByteNorms", test_max_byte_norms),
            test("testEmptyValueVsNoValue", test_empty_value_vs_no_value),
        ],
    )
}

const BYTE_TEST_FIELD: &str = "normsTestByte";

/// Port of `TestNorms.testMaxByteNorms`.
fn test_max_byte_norms() -> Result<()> {
    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "TestNorms.testMaxByteNorms",
    ));
    build_index(dir.as_directory())?;
    let open: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let norm_values = MultiDocValues::get_norm_values(&open, BYTE_TEST_FIELD)?;
    assert_not_null(&norm_values);
    let mut norm_values = norm_values.unwrap_or_else(|| npe("normValues"));
    let stored_fields = open.stored_fields()?;
    for i in 0..open.max_doc() {
        let document = stored_fields.document(i)?;
        let stored = document
            .get(BYTE_TEST_FIELD)
            .unwrap_or_else(|| npe("document.get(BYTE_TEST_FIELD)"));
        let first = stored.split(' ').next().unwrap_or_default();
        let expected: i64 = first.parse().unwrap_or_else(|_| {
            Throwable::new(
                "java.lang.NumberFormatException",
                format!("For input string: \"{first}\""),
            )
            .throw()
        });
        assert_equals(&i, &norm_values.next_doc()?);
        assert_equals(&expected, &norm_values.long_value()?);
    }
    open.close()?;
    dir.close()
}

// TODO: create a testNormsNotPresent ourselves by adding/deleting/merging docs

/// Port of `TestNorms.buildIndex`.
fn build_index(dir: Arc<dyn Directory>) -> Result<()> {
    let random = random();
    let analyzer = MockAnalyzer::new(&*crate::core_support::random());
    // we need at least 3 for maxTokenLength otherwise norms are messed up
    analyzer.set_max_token_length(TestUtil::next_int(
        &*crate::core_support::random(),
        3,
        MAX_TERM_LENGTH as i32,
    ));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    let provider: Arc<dyn Similarity> = Arc::new(MySimProvider::new());
    config.set_similarity(Some(provider))?;
    let writer = RandomIndexWriter::new_with_config(&*random, dir, config)?;
    let num = LuceneTestCase::at_least(100);
    for _ in 0..num {
        let mut doc = Document::new();
        let boost = TestUtil::next_int(&*random, 1, 255);
        let value = (0..boost)
            .map(|_| boost.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let f = TextField::new(BYTE_TEST_FIELD, value, Store::YES)?;
        doc.add(Box::new(f));
        writer.add_document(&doc)?;
        doc.remove_field(BYTE_TEST_FIELD);
    }
    writer.commit()?;
    writer.close()
}

/// `public class MySimProvider extends PerFieldSimilarityWrapper`.
#[derive(Debug)]
struct MySimProvider {
    delegate: ClassicSimilarity,
    byte_encoding: ByteEncodingBoostSimilarity,
}

impl MySimProvider {
    fn new() -> Self {
        MySimProvider {
            delegate: ClassicSimilarity::new(),
            byte_encoding: ByteEncodingBoostSimilarity,
        }
    }
}

impl PerFieldSimilarityWrapper for MySimProvider {
    fn get(&self, field: &str) -> &dyn Similarity {
        if BYTE_TEST_FIELD == field {
            &self.byte_encoding
        } else {
            &self.delegate
        }
    }
}

impl std::fmt::Display for MySimProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PerFieldSimilarityWrapper")
    }
}

impl Similarity for MySimProvider {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestNorms$MySimProvider"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        per_field_compute_norm(self, state)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        per_field_scorer(self, boost, collection_stats, term_stats)
    }
}

/// `public static class ByteEncodingBoostSimilarity extends Similarity`.
#[derive(Debug)]
struct ByteEncodingBoostSimilarity;

impl std::fmt::Display for ByteEncodingBoostSimilarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ByteEncodingBoostSimilarity")
    }
}

impl Similarity for ByteEncodingBoostSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestNorms$ByteEncodingBoostSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        Ok(i64::from(state.length()))
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

/// Port of `TestNorms.testEmptyValueVsNoValue`.
fn test_empty_value_vs_no_value() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let cfg = LuceneTestCase::new_index_writer_config();
    cfg.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(dir.as_directory(), cfg)?;
    let mut doc = Document::new();
    w.add_document(&doc)?;
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "foo",
        "",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.force_merge(1)?;
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    drop(w);
    let leaf_reader = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader))?;
    let norm_values = leaf_reader.get_norm_values("foo")?;
    assert_not_null(&norm_values);
    let mut norm_values = norm_values.unwrap_or_else(|| npe("normValues"));
    assert_equals(&1, &norm_values.next_doc()?); // doc 0 does not have norms
    assert_equals(&0, &norm_values.long_value()?);
    reader.close()?;
    dir.close()
}
