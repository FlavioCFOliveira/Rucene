//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBoolean2.java`.
//!
//! Test BooleanQuery2 against BooleanQuery by overriding the standard query
//! parser. This also tests the scoring order of BooleanQuery.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::search::boolean_scorer::SIZE as BOOLEAN_SCORER_SIZE;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::sort::{Sort, SortField};
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{BooleanQueryBuilder, Occur, Query, ScoreDoc};
use rucene::store::DEFAULT_IO_CONTEXT;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{close_dir, close_reader, score_docs_of, static_hook, term, tq};
use crate::core_support::{assert_equals, class, random, test, verbose};

/// num "extra" docs containing value in "field2" added to the "big" clone of
/// the index
const NUM_EXTRA_DOCS: i32 = 6000;

pub const FIELD: &str = "field";

/// The static fields of `TestBoolean2`.
struct Statics {
    searcher: IndexSearcher,
    single_segment_searcher: IndexSearcher,
    big_searcher: IndexSearcher,
    reader: Arc<dyn DirectoryReader>,
    little_reader: Arc<dyn DirectoryReader>,
    single_segment_reader: Arc<dyn DirectoryReader>,
    /// num of empty docs injected between every doc in the (main) index
    num_filler_docs: i32,
    /// num of empty docs injected prior to the first doc in the (main) index
    pre_filler_docs: i32,
    directory: TestDirectory,
    single_segment_directory: TestDirectory,
    dir2: TestDirectory,
    mul_factor: i32,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn with_statics<T>(body: impl FnOnce(&mut Statics) -> Result<T>) -> Result<T> {
    let mut guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    body(guard.as_mut().expect("beforeClass ran"))
}

/// `TestBoolean2`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBoolean2",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testQueries01", test_queries01),
            test("testQueries02", test_queries02),
            test("testQueries03", test_queries03),
            test("testQueries04", test_queries04),
            test("testQueries05", test_queries05),
            test("testQueries06", test_queries06),
            test("testQueries07", test_queries07),
            test("testQueries08", test_queries08),
            test("testQueries09", test_queries09),
            test("testRandomQueries", test_random_queries),
        ],
    )
}

fn copy_of(dir: &TestDirectory) -> Result<TestDirectory> {
    let copy = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let from = dir.as_directory();
    let to = copy.as_directory();
    for name in from.list_all()? {
        if name.starts_with("extra") {
            continue;
        }
        to.copy_from(&*from, &name, &name, &*DEFAULT_IO_CONTEXT)?;
        to.sync(&[name])?;
    }
    Ok(copy)
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> rucene::index::index_writer_config::IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

const DOC_FIELDS: [&str; 4] = [
    "w1 w2 w3 w4 w5",
    "w1 w3 w2 w3",
    "w1 xx w2 yy w3",
    "w1 w3 xx w2 yy mm",
];

fn before_class() -> Result<()> {
    let num_filler_docs = if random().next_boolean() {
        0
    } else {
        BOOLEAN_SCORER_SIZE
    };
    let pre_filler_docs = TestUtil::next_int(&*random(), 0, num_filler_docs / 2);
    if verbose() {
        println!("TEST: NUM_FILLER_DOCS={num_filler_docs} PRE_FILLER_DOCS={pre_filler_docs}");
    }

    let directory = if num_filler_docs * pre_filler_docs > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir())
    } else {
        LuceneTestCase::new_directory()
    };

    let iwc = mock_iwc();
    iwc.set_codec(TestUtil::get_default_codec())?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), iwc)?;
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_omit_norms(true)?;

    let mut doc = Document::new();
    for _filler in 0..pre_filler_docs {
        writer.add_document(&doc)?;
    }
    for text in DOC_FIELDS {
        doc.add(Box::new(Field::new(FIELD, text.to_string(), ft.clone())?));
        writer.add_document(&doc)?;

        doc = Document::new();
        for _filler in 0..num_filler_docs {
            writer.add_document(&doc)?;
        }
    }
    writer.close()?;
    let little_reader = directory_reader::open(directory.as_directory())?;
    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&little_reader));
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));

    let single_segment_directory = if num_filler_docs * pre_filler_docs > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir())
    } else {
        LuceneTestCase::new_directory()
    };

    {
        let from = directory.as_directory();
        let to = single_segment_directory.as_directory();
        for file_name in from.list_all()? {
            if file_name.starts_with("extra") {
                continue;
            }
            to.copy_from(&*from, &file_name, &file_name, &*DEFAULT_IO_CONTEXT)?;
            to.sync(&[file_name])?;
        }
    }

    let iwc = mock_iwc();
    iwc.set_codec(TestUtil::get_default_codec())?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    {
        let w = IndexWriter::new(single_segment_directory.as_directory(), iwc)?;
        let merged = w.force_merge_with(1, true);
        merged.and(w.close())?;
    }
    let single_segment_reader = directory_reader::open(single_segment_directory.as_directory())?;
    let mut single_segment_searcher =
        LuceneTestCase::new_searcher(Arc::clone(&single_segment_reader));
    single_segment_searcher.set_similarity(Arc::clone(searcher.get_similarity()));

    let dir2 = copy_of(&directory)?;

    let mut mul_factor = 1;
    let mut doc_count;
    if verbose() {
        println!("\nTEST: now copy index...");
    }
    loop {
        if verbose() {
            println!("\nTEST: cycle...");
        }
        let mut copy = copy_of(&dir2)?;

        let iwc = mock_iwc();
        iwc.set_codec(TestUtil::get_default_codec())?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir2.as_directory(), iwc)?;
        w.add_indexes_dirs(&[copy.as_directory()])?;
        copy.close()?;
        doc_count = w.get_doc_stats()?.max_doc;
        w.close()?;
        mul_factor *= 2;
        if doc_count >= 3000 * num_filler_docs {
            break;
        }
    }

    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    iwc.set_codec(TestUtil::get_default_codec())?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir2.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(Field::new(
        "field2",
        "xxx".to_string(),
        ft.clone(),
    )?));
    for _ in 0..NUM_EXTRA_DOCS / 2 {
        w.add_document(&doc)?;
    }
    let mut doc = Document::new();
    doc.add(Box::new(Field::new(
        "field2",
        "big bad bug".to_string(),
        ft.clone(),
    )?));
    for _ in 0..NUM_EXTRA_DOCS / 2 {
        w.add_document(&doc)?;
    }
    let reader = w.get_reader()?;
    let big_searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    w.close()?;

    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        searcher,
        single_segment_searcher,
        big_searcher,
        reader,
        little_reader,
        single_segment_reader,
        num_filler_docs,
        pre_filler_docs,
        directory,
        single_segment_directory,
        dir2,
        mul_factor,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(s) = statics {
        drop((s.searcher, s.single_segment_searcher, s.big_searcher));
        close_reader(Some(s.reader))?;
        close_reader(Some(s.little_reader))?;
        close_reader(Some(s.single_segment_reader))?;
        close_dir(Some(s.dir2))?;
        close_dir(Some(s.directory))?;
        close_dir(Some(s.single_segment_directory))?;
    }
    Ok(())
}

/// `new TopScoreDocCollectorManager(topDocsToCheck, Integer.MAX_VALUE)`.
fn top_score(top_docs_to_check: i32) -> Result<TopScoreDocCollectorManager> {
    TopScoreDocCollectorManager::with_num_hits(top_docs_to_check, i32::MAX)
}

/// `queriesTest(Query, int[])`.
pub fn queries_test(query: Arc<dyn Query>, exp_doc_nrs: &[i32]) -> Result<()> {
    with_statics(|s| {
        let mut exp_doc_nrs = exp_doc_nrs.to_vec();
        if 0 < s.num_filler_docs {
            for e in exp_doc_nrs.iter_mut() {
                *e = s.pre_filler_docs + ((s.num_filler_docs + 1) * *e);
            }
        }

        let top_docs_to_check = LuceneTestCase::at_least(1000);
        let collector_manager = top_score(top_docs_to_check)?;
        let hits1: Vec<ScoreDoc> = s
            .searcher
            .search_with_manager(Arc::clone(&query), &collector_manager)?
            .score_docs;
        let collector_manager = top_score(top_docs_to_check)?;
        let hits2: Vec<ScoreDoc> = s
            .searcher
            .search_with_manager(Arc::clone(&query), &collector_manager)?
            .score_docs;

        CheckHits::check_hits_query(&*query, &hits1, &hits2, &exp_doc_nrs);

        let collector_manager = top_score(top_docs_to_check)?;
        let top_docs = s
            .single_segment_searcher
            .search_with_manager(Arc::clone(&query), &collector_manager)?;
        let hits2 = top_docs.score_docs.clone();
        CheckHits::check_hits_query(&*query, &hits1, &hits2, &exp_doc_nrs);

        assert_equals(
            &(i64::from(s.mul_factor) * top_docs.total_hits.value()),
            &i64::from(s.big_searcher.count(Arc::clone(&query))?),
        );

        let collector_manager = top_score(top_docs_to_check)?;
        let hits1 = s
            .big_searcher
            .search_with_manager(Arc::clone(&query), &collector_manager)?
            .score_docs;
        let collector_manager = top_score(top_docs_to_check)?;
        let hits2 = s
            .big_searcher
            .search_with_manager(Arc::clone(&query), &collector_manager)?
            .score_docs;

        CheckHits::check_equal(&*query, &hits1, &hits2);
        Ok(())
    })
}

fn bq(clauses: &[(&str, Occur)]) -> Result<Arc<dyn Query>> {
    let mut query = BooleanQueryBuilder::new();
    for (text, occur) in clauses {
        query.add(tq(FIELD, text), *occur)?;
    }
    Ok(Arc::new(query.build()))
}

fn test_queries01() -> Result<()> {
    let query = bq(&[("w3", Occur::MUST), ("xx", Occur::MUST)])?;
    queries_test(query, &[2, 3])
}

fn test_queries02() -> Result<()> {
    let query = bq(&[("w3", Occur::MUST), ("xx", Occur::SHOULD)])?;
    queries_test(query, &[2, 3, 1, 0])
}

fn test_queries03() -> Result<()> {
    let query = bq(&[("w3", Occur::SHOULD), ("xx", Occur::SHOULD)])?;
    queries_test(query, &[2, 3, 1, 0])
}

fn test_queries04() -> Result<()> {
    let query = bq(&[("w3", Occur::SHOULD), ("xx", Occur::MUST_NOT)])?;
    queries_test(query, &[1, 0])
}

fn test_queries05() -> Result<()> {
    let query = bq(&[("w3", Occur::MUST), ("xx", Occur::MUST_NOT)])?;
    queries_test(query, &[1, 0])
}

fn test_queries06() -> Result<()> {
    let query = bq(&[
        ("w3", Occur::MUST),
        ("xx", Occur::MUST_NOT),
        ("w5", Occur::MUST_NOT),
    ])?;
    queries_test(query, &[1])
}

fn test_queries07() -> Result<()> {
    let query = bq(&[
        ("w3", Occur::MUST_NOT),
        ("xx", Occur::MUST_NOT),
        ("w5", Occur::MUST_NOT),
    ])?;
    queries_test(query, &[])
}

fn test_queries08() -> Result<()> {
    let query = bq(&[
        ("w3", Occur::MUST),
        ("xx", Occur::SHOULD),
        ("w5", Occur::MUST_NOT),
    ])?;
    queries_test(query, &[2, 3, 1])
}

fn test_queries09() -> Result<()> {
    let query = bq(&[
        ("w3", Occur::MUST),
        ("xx", Occur::MUST),
        ("w2", Occur::MUST),
        ("zz", Occur::SHOULD),
    ])?;
    queries_test(query, &[2, 3])
}

fn test_random_queries() -> Result<()> {
    let vals = ["w1", "w2", "w3", "w4", "w5", "xx", "yy", "zzz"];

    let mut q1: Option<Arc<dyn Query>> = None;
    let result = (|| -> Result<()> {
        let num = LuceneTestCase::at_least(3);
        for _ in 0..num {
            let level = random().next_int_bound(3);
            let rnd = JdkRandom::new(random().next_long());
            let allow_must = random().next_boolean();
            let q: Arc<dyn Query> =
                Arc::new(rand_bool_query(&rnd, allow_must, level, FIELD, &vals, None)?.build());
            q1 = Some(Arc::clone(&q));

            let sort = Sort::new_fields(vec![SortField::FIELD_DOC])?;

            with_statics(|s| {
                QueryUtils::check_searcher(&*random(), &q, Some(&s.searcher)); // baseline sim
                let random_sim = Arc::clone(s.big_searcher.get_similarity());
                s.searcher.set_similarity(random_sim); // random sim
                let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    QueryUtils::check_searcher(&*random(), &q, Some(&s.searcher));
                }));
                s.searcher
                    .set_similarity(Arc::new(ClassicSimilarity::new())); // restore
                if let Err(p) = checked {
                    std::panic::resume_unwind(p);
                }

                let hits1 = s
                    .searcher
                    .search_with_manager(
                        Arc::clone(&q),
                        &TopFieldCollectorManager::with_num_hits(sort.clone(), 1000, 1)?,
                    )?
                    .score_docs;
                let hits1 = score_docs_of(&hits1);
                let top_docs = s.searcher.search_with_manager(
                    Arc::clone(&q),
                    &TopFieldCollectorManager::with_num_hits(sort.clone(), 1000, 1)?,
                )?;
                let hits2 = score_docs_of(&top_docs.score_docs);
                CheckHits::check_equal(&*q, &hits1, &hits2);

                let mut q3 = BooleanQueryBuilder::new();
                q3.add(Arc::clone(&q), Occur::SHOULD)?;
                q3.add(
                    Arc::new(PrefixQuery::new(term("field2", "b"))?),
                    Occur::SHOULD,
                )?;
                assert_equals(
                    &(i64::from(s.mul_factor) * top_docs.total_hits.value()
                        + i64::from(NUM_EXTRA_DOCS / 2)),
                    &i64::from(s.big_searcher.count(Arc::new(q3.build()))?),
                );

                let hits1 = s
                    .big_searcher
                    .search_with_manager(
                        Arc::clone(&q),
                        &TopFieldCollectorManager::with_num_hits(sort.clone(), s.mul_factor, 1)?,
                    )?
                    .score_docs;
                let hits2 = s
                    .big_searcher
                    .search_with_manager(
                        Arc::clone(&q),
                        &TopFieldCollectorManager::with_num_hits(sort.clone(), s.mul_factor, 1)?,
                    )?
                    .score_docs;
                CheckHits::check_equal(&*q, &score_docs_of(&hits1), &score_docs_of(&hits2));
                Ok(())
            })?;
        }
        Ok(())
    })();
    if result.is_err() {
        println!(
            "failed query: {}",
            q1.map(|q| rucene::search::query::query_to_string(&*q))
                .unwrap_or_else(|| "null".to_string())
        );
    }
    result
}

/// `TestBoolean2.Callback`.
pub trait Callback {
    fn post_create(&self, q: &mut BooleanQueryBuilder) -> Result<()>;
}

/// `randBoolQuery(Random, boolean, int, String, String[], Callback)`.
pub fn rand_bool_query(
    rnd: &dyn Random,
    allow_must: bool,
    level: i32,
    field: &str,
    vals: &[&str],
    cb: Option<&dyn Callback>,
) -> Result<BooleanQueryBuilder> {
    let mut current = BooleanQueryBuilder::new();
    let len = vals.len() as i32;
    let mut i = 0;
    while i < rnd.next_int_bound(len) + 1 {
        let mut q_type = 0; // term query
        if level > 0 {
            q_type = rnd.next_int_bound(10);
        }
        let q: Arc<dyn Query> = if q_type < 3 {
            tq(field, vals[rnd.next_int_bound(len) as usize])
        } else if q_type < 4 {
            let t1 = vals[rnd.next_int_bound(len) as usize];
            let t2 = vals[rnd.next_int_bound(len) as usize];
            // slop increases possibility of matching
            Arc::new(PhraseQuery::with_slop(10, field, &[t1, t2])?)
        } else if q_type < 7 {
            Arc::new(WildcardQuery::new(term(field, "w*"))?)
        } else {
            Arc::new(rand_bool_query(rnd, allow_must, level - 1, field, vals, cb)?.build())
        };

        let r = rnd.next_int_bound(10);
        let occur = if r < 2 {
            Occur::MUST_NOT
        } else if r < 5 {
            if allow_must {
                Occur::MUST
            } else {
                Occur::SHOULD
            }
        } else {
            Occur::SHOULD
        };

        current.add(q, occur)?;
        i += 1;
    }
    if let Some(cb) = cb {
        cb.post_create(&mut current)?;
    }
    Ok(current)
}
