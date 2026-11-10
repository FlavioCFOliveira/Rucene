//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSimilarityProvider.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::{FieldInvertState, IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::per_field_similarity_wrapper::PerFieldSimilarityWrapper;
use rucene::search::similarities::{
    per_field_compute_norm, per_field_scorer, CollectionStatistics, SimScorer, Similarity,
    TermStatistics,
};
use rucene::search::term_query::TermQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{field_mut, fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, random};

/// The instance of `TestSimilarityProvider`.
#[derive(Default)]
pub struct SimilarityProvider {
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
}

/// `TestSimilarityProvider`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SimilarityProvider>(
        "org.apache.lucene.search.TestSimilarityProvider",
        vec![
            set_up(provider_set_up),
            tear_down(provider_tear_down),
            ftest("testBasics", test_basics),
        ],
    )
}

fn provider_set_up(f: &mut SimilarityProvider) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let sim: Arc<dyn Similarity> = Arc::new(ExampleSimilarityProvider::default());
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_similarity(Some(Arc::clone(&sim)))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwc)?;
    let mut doc = Document::new();
    let field = LuceneTestCase::new_text_field("foo", "", Store::NO);
    doc.add(Box::new(field));
    let field2 = LuceneTestCase::new_text_field("bar", "", Store::NO);
    doc.add(Box::new(field2));

    field_mut(&mut doc, "foo").set_string_value("quick brown fox".to_string())?;
    field_mut(&mut doc, "bar").set_string_value("quick brown fox".to_string())?;
    iw.add_document(&doc)?;
    field_mut(&mut doc, "foo").set_string_value("jumps over lazy brown dog".to_string())?;
    field_mut(&mut doc, "bar").set_string_value("jumps over lazy brown dog".to_string())?;
    iw.add_document(&doc)?;
    let reader = iw.get_reader()?;
    iw.close()?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(sim);
    f.directory = Some(directory);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

fn provider_tear_down(f: &mut SimilarityProvider) -> Result<()> {
    f.searcher = None;
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(mut directory) = f.directory.take() {
        directory.close()?;
    }
    Ok(())
}

/// Port of `TestSimilarityProvider.testBasics`.
fn test_basics(f: &mut SimilarityProvider) -> Result<()> {
    let reader: Arc<dyn IndexReader> =
        Arc::clone(f.reader.as_ref().expect("setUp ran")) as Arc<dyn IndexReader>;
    let searcher = f.searcher.as_ref().expect("setUp ran");
    // sanity check of norms writer
    // TODO: generalize
    let npe = || -> ! {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    };
    let mut foo_norms = MultiDocValues::get_norm_values(&reader, "foo")?.unwrap_or_else(|| npe());
    let mut bar_norms = MultiDocValues::get_norm_values(&reader, "bar")?.unwrap_or_else(|| npe());
    for i in 0..reader.max_doc() {
        assert_equals(&i, &foo_norms.next_doc()?);
        assert_equals(&i, &bar_norms.next_doc()?);
        Assert::assert_false(foo_norms.long_value()? == bar_norms.long_value()?);
    }

    // sanity check of searching
    let foodocs = searcher.search(
        Arc::new(TermQuery::new(Term::from_text("foo", "brown"))),
        10,
    )?;
    Assert::assert_true(foodocs.total_hits.value() > 0);
    let bardocs = searcher.search(
        Arc::new(TermQuery::new(Term::from_text("bar", "brown"))),
        10,
    )?;
    Assert::assert_true(bardocs.total_hits.value() > 0);
    Assert::assert_true(foodocs.score_docs[0].score < bardocs.score_docs[0].score);
    Ok(())
}

/// `private static class ExampleSimilarityProvider extends PerFieldSimilarityWrapper`.
#[derive(Debug, Default)]
struct ExampleSimilarityProvider {
    sim1: Sim1,
    sim2: Sim2,
}

impl PerFieldSimilarityWrapper for ExampleSimilarityProvider {
    fn get(&self, field: &str) -> &dyn Similarity {
        if field == "foo" {
            &self.sim1
        } else {
            &self.sim2
        }
    }
}

impl std::fmt::Display for ExampleSimilarityProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PerFieldSimilarityWrapper")
    }
}

impl Similarity for ExampleSimilarityProvider {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestSimilarityProvider$ExampleSimilarityProvider"
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

/// The anonymous `SimScorer` of `Sim1` and `Sim2`, scoring `value`.
struct ConstantSimScorer(f32);

impl SimScorer for ConstantSimScorer {
    fn score(&self, _freq: f32, _norm: i64) -> f32 {
        self.0
    }
}

/// `private static class Sim1 extends Similarity`.
#[derive(Debug, Default)]
struct Sim1;

impl std::fmt::Display for Sim1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sim1")
    }
}

impl Similarity for Sim1 {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestSimilarityProvider$Sim1"
    }

    fn compute_norm(&self, _state: &FieldInvertState) -> Result<i64> {
        Ok(1)
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        Box::new(ConstantSimScorer(1.0))
    }
}

/// `private static class Sim2 extends Similarity`.
#[derive(Debug, Default)]
struct Sim2;

impl std::fmt::Display for Sim2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sim2")
    }
}

impl Similarity for Sim2 {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestSimilarityProvider$Sim2"
    }

    fn compute_norm(&self, _state: &FieldInvertState) -> Result<i64> {
        Ok(10)
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        Box::new(ConstantSimScorer(10.0))
    }
}
