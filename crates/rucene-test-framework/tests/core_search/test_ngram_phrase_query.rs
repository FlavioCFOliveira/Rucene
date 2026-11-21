//! Port of `lucene/core/src/test/org/apache/lucene/search/TestNGramPhraseQuery.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::ngram_phrase_query::NGramPhraseQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::Query;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{close_dir, close_reader, static_hook, term};
use crate::core_support::{assert_array_equals, class, random, test};

/// The static fields `reader`, `directory` and `searcher`.
struct Statics {
    reader: Arc<dyn DirectoryReader>,
    directory: TestDirectory,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// `TestNGramPhraseQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestNGramPhraseQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testRewrite", test_rewrite),
        ],
    )
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;
    writer.close()?;
    let reader = directory_reader::open(directory.as_directory())?;
    let searcher = Arc::new(IndexSearcher::new(
        Arc::clone(&reader) as Arc<dyn IndexReader>
    )?);
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        reader,
        directory,
        searcher,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(s) = statics {
        drop(s.searcher);
        close_reader(Some(s.reader))?;
        close_dir(Some(s.directory))?;
    }
    Ok(())
}

/// `(PhraseQuery) q`.
fn as_phrase_query(q: &Arc<dyn Query>) -> &PhraseQuery {
    q.as_any()
        .downcast_ref::<PhraseQuery>()
        .unwrap_or_else(|| Assert::fail("expected a PhraseQuery"))
}

/// `pq.rewrite(searcher)`: the query it rewrites to, `pq` itself when it
/// rewrites to itself (`None`).
fn rewrite(pq: &NGramPhraseQuery, searcher: &IndexSearcher) -> Result<(Arc<dyn Query>, bool)> {
    Ok(match pq.rewrite(searcher)? {
        Some(q) => (q, false),
        None => (Arc::new(pq.clone()), true),
    })
}

/// Port of `TestNGramPhraseQuery.testRewrite`.
fn test_rewrite() -> Result<()> {
    let searcher = {
        let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
    };

    // bi-gram test ABC => AB/BC => AB/BC
    let pq1 = NGramPhraseQuery::new(2, PhraseQuery::from_strings("f", &["AB", "BC"])?);

    let (q, _) = rewrite(&pq1, &searcher)?;
    // assertSame(q.rewrite(searcher), q): `None` is "rewrites to itself"
    Assert::assert_true(q.rewrite(&searcher)?.is_none());
    let rewritten1 = as_phrase_query(&q);
    assert_array_equals(&[term("f", "AB"), term("f", "BC")], rewritten1.get_terms());
    assert_array_equals(&[0, 1], rewritten1.get_positions());

    // bi-gram test ABCD => AB/BC/CD => AB//CD
    let pq2 = NGramPhraseQuery::new(2, PhraseQuery::from_strings("f", &["AB", "BC", "CD"])?);

    let (q, same) = rewrite(&pq2, &searcher)?;
    Assert::assert_true(q.as_any().is::<PhraseQuery>());
    Assert::assert_false(same); // assertNotSame(pq2, q)
    let rewritten2 = as_phrase_query(&q);
    assert_array_equals(&[term("f", "AB"), term("f", "CD")], rewritten2.get_terms());
    assert_array_equals(&[0, 2], rewritten2.get_positions());

    // tri-gram test ABCDEFGH => ABC/BCD/CDE/DEF/EFG/FGH => ABC///DEF//FGH
    let pq3 = NGramPhraseQuery::new(
        3,
        PhraseQuery::from_strings("f", &["ABC", "BCD", "CDE", "DEF", "EFG", "FGH"])?,
    );

    let (q, same) = rewrite(&pq3, &searcher)?;
    Assert::assert_true(q.as_any().is::<PhraseQuery>());
    Assert::assert_false(same); // assertNotSame(pq3, q)
    let rewritten3 = as_phrase_query(&q);
    assert_array_equals(
        &[term("f", "ABC"), term("f", "DEF"), term("f", "FGH")],
        rewritten3.get_terms(),
    );
    assert_array_equals(&[0, 3, 5], rewritten3.get_positions());
    Ok(())
}
