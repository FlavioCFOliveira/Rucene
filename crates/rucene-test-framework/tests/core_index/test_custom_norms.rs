//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCustomNorms.java`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Store, TextField};
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{FieldInvertState, IndexReader};
use rucene::search::similarities::per_field_similarity_wrapper::PerFieldSimilarityWrapper;
use rucene::search::similarities::{
    per_field_compute_norm, per_field_scorer, ClassicSimilarity, CollectionStatistics, SimScorer,
    Similarity, TermStatistics,
};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, class, random, test};
use crate::support::open_reader;

/// `TestCustomNorms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestCustomNorms",
        vec![test("testFloatNorms", test_float_norms)],
    )
}

const FLOAT_TEST_FIELD: &str = "normsTestFloat";
#[allow(dead_code)]
const EXCEPTION_TEST_FIELD: &str = "normsTestExcp";

/// Port of `TestCustomNorms.testFloatNorms`.
fn test_float_norms() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 2, MAX_TERM_LENGTH as i32));

    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    let provider: Arc<dyn Similarity> = Arc::new(MySimProvider::new());
    config.set_similarity(Some(provider))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let docs = LineFileDocs::with_default_file(&*random())?;
    let num = LuceneTestCase::at_least(100);
    for _ in 0..num {
        let doc = docs.next_doc()?;
        let mut doc = doc.borrow_mut();
        let boost = TestUtil::next_int(&*random(), 1, 10);
        let value = (0..boost)
            .map(|_| boost.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let f = TextField::new(FLOAT_TEST_FIELD, value, Store::YES)?;

        doc.add(Box::new(f));
        writer.add_document(&*doc)?;
        doc.remove_field(FLOAT_TEST_FIELD);
        if LuceneTestCase::rarely() {
            writer.commit()?;
        }
    }
    writer.commit()?;
    writer.close()?;
    let open: Arc<dyn IndexReader> = open_reader(&dir)?;
    let norms = MultiDocValues::get_norm_values(&open, FLOAT_TEST_FIELD)?;
    assert_not_null(&norms);
    let mut norms = norms.expect("checked by assertNotNull");
    let stored_fields = open.stored_fields()?;
    for i in 0..open.max_doc() {
        let document = stored_fields.document(i)?;
        let stored = document
            .get(FLOAT_TEST_FIELD)
            .unwrap_or_else(|| crate::support::npe("document.get(FLOAT_TEST_FIELD)"));
        let first = stored.split(' ').next().unwrap_or_default();
        let expected: i64 = first.parse().unwrap_or_else(|_| {
            Throwable::new(
                "java.lang.NumberFormatException",
                format!("For input string: \"{first}\""),
            )
            .throw()
        });
        assert_equals(&i, &norms.next_doc()?);
        assert_equals(&expected, &norms.long_value()?);
    }
    open.close()?;
    dir.close()?;
    docs.close();
    Ok(())
}

/// `public class MySimProvider extends PerFieldSimilarityWrapper`.
///
/// Java's `get` returns a new `FloatEncodingBoostSimilarity` per call; a
/// Rucene scorer borrows its similarity (see `PerFieldSimilarityWrapper`),
/// so the provider owns the one instance it hands out.
#[derive(Debug)]
struct MySimProvider {
    delegate: ClassicSimilarity,
    float_encoding: FloatEncodingBoostSimilarity,
}

impl MySimProvider {
    fn new() -> Self {
        MySimProvider {
            delegate: ClassicSimilarity::new(),
            float_encoding: FloatEncodingBoostSimilarity,
        }
    }
}

impl PerFieldSimilarityWrapper for MySimProvider {
    fn get(&self, field: &str) -> &dyn Similarity {
        if FLOAT_TEST_FIELD == field {
            &self.float_encoding
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
        "org.apache.lucene.index.TestCustomNorms$MySimProvider"
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

/// `public static class FloatEncodingBoostSimilarity extends Similarity`.
#[derive(Debug)]
struct FloatEncodingBoostSimilarity;

impl std::fmt::Display for FloatEncodingBoostSimilarity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FloatEncodingBoostSimilarity")
    }
}

impl Similarity for FloatEncodingBoostSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestCustomNorms$FloatEncodingBoostSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        Ok(state.length() as i64)
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
