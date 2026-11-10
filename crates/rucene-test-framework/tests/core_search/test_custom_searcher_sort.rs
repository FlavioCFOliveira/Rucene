//! Port of `lucene/core/src/test/org/apache/lucene/search/TestCustomSearcherSort.java`.
//!
//! Unit test for sorting code.

use std::collections::BTreeMap;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{DateTools, Document, Resolution, SortedDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::IndexReader;
use rucene::search::index_searcher::{IndexSearcher, IndexSearcherOverrides};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{BooleanQueryBuilder, Occur, Query, ScoreDoc, TopDocs, TopFieldDocs};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, score_docs_of, set_up, tear_down, tq,
};
use crate::core_support::{assert_equals, random, verbose};

/// The instance of `TestCustomSearcherSort`.
#[derive(Default)]
pub struct CustomSearcherSort {
    index: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    query: Option<Arc<dyn Query>>,
    index_size: i32,
}

/// `TestCustomSearcherSort`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<CustomSearcherSort>(
        "org.apache.lucene.search.TestCustomSearcherSort",
        vec![
            set_up(custom_set_up),
            tear_down(|f: &mut CustomSearcherSort| {
                close_reader(f.reader.take())?;
                close_dir(f.index.take())
            }),
            ftest(
                "testFieldSortCustomSearcher",
                test_field_sort_custom_searcher,
            ),
            ftest(
                "testFieldSortSingleSearcher",
                test_field_sort_single_searcher,
            ),
        ],
    )
}

/// Create index and query for test cases.
fn custom_set_up(f: &mut CustomSearcherSort) -> Result<()> {
    f.index_size = LuceneTestCase::at_least(2000);
    let index = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index.as_directory())?;
    let random_gen = RandomGen::new(random());
    for i in 0..f.index_size {
        // don't decrease; if to low the
        // problem doesn't show up
        let mut doc = Document::new();
        if (i % 5) != 0 {
            // some documents must not have an entry in the first
            // sort field
            doc.add(Box::new(SortedDocValuesField::new(
                "publicationDate_",
                BytesRef::from_char_sequence(&random_gen.get_lucene_date()),
            )));
        }
        if (i % 7) == 0 {
            // some documents to match the query (see below)
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "content",
                "test",
                Store::YES,
            )));
        }
        // every document has a defined 'mandant' field
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "mandant",
            &(i % 3).to_string(),
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    f.reader = Some(writer.get_reader()?);
    writer.close()?;
    f.query = Some(tq("content", "test"));
    f.index = Some(index);
    Ok(())
}

fn cust_sort() -> Result<Sort> {
    Sort::new_fields(vec![
        SortField::new(Some("publicationDate_".to_string()), SortFieldType::String)?,
        SortField::FIELD_SCORE,
    ])
}

/// Run the test using two CustomSearcher instances.
fn test_field_sort_custom_searcher(f: &mut CustomSearcherSort) -> Result<()> {
    let cust_sort = cust_sort()?;
    let searcher = custom_searcher(f, 2)?;
    // search and check hits
    match_hits(f, &searcher, &cust_sort)
}

/// Run the test using one CustomSearcher wrapped by a MultiSearcher.
fn test_field_sort_single_searcher(f: &mut CustomSearcherSort) -> Result<()> {
    let cust_sort = cust_sort()?;
    let searcher = custom_searcher(f, 2)?;
    // search and check hits
    match_hits(f, &searcher, &cust_sort)
}

/// `new CustomSearcher(reader, switcher)`.
fn custom_searcher(f: &CustomSearcherSort, switcher: i32) -> Result<IndexSearcher> {
    let reader = Arc::clone(f.reader.as_ref().expect("setUp ran")) as Arc<dyn IndexReader>;
    let mut searcher = IndexSearcher::new(reader)?;
    searcher.set_overrides(Arc::new(CustomSearcher { switcher }));
    Ok(searcher)
}

/// Make sure the documents returned by the search match the expected list.
fn match_hits(f: &CustomSearcherSort, searcher: &IndexSearcher, sort: &Sort) -> Result<()> {
    let query = f.query.as_ref().expect("setUp ran");
    // make a query without sorting first
    let hits_by_rank = searcher.search(Arc::clone(query), i32::MAX)?.score_docs;
    check_hits(&hits_by_rank, "Sort by rank: "); // check for duplicates
    let mut result_map: BTreeMap<i32, usize> = BTreeMap::new();
    // store hits in TreeMap - TreeMap does not allow duplicates; existing
    // entries are silently overwritten
    for (hitid, hit) in hits_by_rank.iter().enumerate() {
        result_map.insert(hit.doc, hitid); // Key: Lucene Document ID, Value: Hits-Objekt Index
    }

    // now make a query using the sort criteria
    let result_sort = score_docs_of(
        &searcher
            .search_with_sort(Arc::clone(query), i32::MAX, sort)?
            .score_docs,
    );
    check_hits(&result_sort, "Sort by custom criteria: "); // check for duplicates

    // besides the sorting both sets of hits must be identical
    for hit in &result_sort {
        let id_hit_date = hit.doc; // document ID from sorted search
        if !result_map.contains_key(&id_hit_date) {
            log(&format!(
                "ID {id_hit_date} not found. Possibliy a duplicate."
            ));
        }
        Assert::assert_true(result_map.contains_key(&id_hit_date)); // same ID must be in the
                                                                    // Map from the rank-sorted
                                                                    // search
                                                                    // every hit must appear once in both result sets --> remove it from the
                                                                    // Map.
                                                                    // At the end the Map must be empty!
        result_map.remove(&id_hit_date);
    }
    if result_map.is_empty() {
        // log("All hits matched");
    } else {
        log(&format!("Couldn't match {} hits.", result_map.len()));
    }
    assert_equals(&result_map.len(), &0);
    Ok(())
}

/// Check the hits for duplicates.
fn check_hits(hits: &[ScoreDoc], prefix: &str) {
    let mut id_map: BTreeMap<i32, usize> = BTreeMap::new();
    for (docnum, hit) in hits.iter().enumerate() {
        let lucene_id = hit.doc;
        if let Some(previous) = id_map.get(&lucene_id) {
            let message = format!(
                "{prefix}Duplicate key for hit index = {docnum}, previous index = {previous}, Lucene ID = {lucene_id}"
            );
            log(&message);
        } else {
            id_map.insert(lucene_id, docnum);
        }
    }
}

/// Simply write to console - chosen to be independent of log4j etc
fn log(message: &str) {
    if verbose() {
        println!("{message}");
    }
}

/// `public static class CustomSearcher extends IndexSearcher`: its two
/// `search` overrides, which add the `mandant` clause.
#[derive(Debug)]
struct CustomSearcher {
    switcher: i32,
}

impl CustomSearcher {
    fn wrap(&self, query: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
        let mut bq = BooleanQueryBuilder::new();
        bq.add(query, Occur::MUST)?;
        bq.add(tq("mandant", &self.switcher.to_string()), Occur::MUST)?;
        Ok(Arc::new(bq.build()))
    }
}

impl IndexSearcherOverrides for CustomSearcher {
    fn search_with_sort(
        &self,
        searcher: &IndexSearcher,
        query: Arc<dyn Query>,
        n_docs: i32,
        sort: &Sort,
    ) -> Result<TopFieldDocs> {
        searcher.super_search_with_sort(self.wrap(query)?, n_docs, sort)
    }

    fn search(
        &self,
        searcher: &IndexSearcher,
        query: Arc<dyn Query>,
        n_docs: i32,
    ) -> Result<TopDocs> {
        searcher.super_search(self.wrap(query)?, n_docs)
    }
}

/// The test's private `RandomGen`.
struct RandomGen {
    random: Arc<dyn Random>,
    /// `base.getTimeInMillis()` of `new GregorianCalendar(TimeZone.getDefault(),
    /// Locale.getDefault())` after `base.set(1980, 1, 1)`: 1980-02-01 at the
    /// current local time of day.
    base_millis: i64,
}

impl RandomGen {
    fn new(random: Arc<dyn Random>) -> Self {
        // **Divergence from Lucene 10.5.0.** Java's calendar resolves the
        // local fields with the full `GregorianCalendar` machinery; this
        // computes the same instant from the default zone's offsets. Before
        // the class rule set a default zone, UTC stands for the platform's.
        let zone = LuceneTestCase::get_default_time_zone();
        let offset = |utc: i64| -> i64 {
            zone.as_ref()
                .map(|z| i64::from(z.zone_info().get_offset(utc)))
                .unwrap_or(0)
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        const DAY: i64 = 86_400_000;
        let time_of_day = (now + offset(now)).rem_euclid(DAY);
        // 1980-02-01 is day 3683 after the epoch.
        let local = 3683 * DAY + time_of_day;
        let base_millis = local - offset(local - offset(local));
        RandomGen {
            random,
            base_millis,
        }
    }

    /// Just to generate some different Lucene Date strings
    fn get_lucene_date(&self) -> String {
        DateTools::time_to_string(
            self.base_millis + i64::from(self.random.next_int()) - i64::from(i32::MIN),
            Resolution::DAY,
        )
    }
}
