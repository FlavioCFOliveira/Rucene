//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSearchAfter.java`.
//!
//! Tests IndexSearcher's searchAfter() method

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryDocValuesField, Document, DoubleDocValuesField, FloatDocValuesField,
    NumericDocValuesField, SortedDocValuesField, Store, StoredField,
};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexableField, Term};
use rucene::search::field_doc::FieldDoc;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::top_field_collector::TopFieldCollector;
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use rucene::search::{BooleanQueryBuilder, MatchAllDocsQuery, Occur, Query, ScoreDoc};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_array_equals, assert_equals, random, verbose};

/// The instance of `TestSearchAfter`.
#[derive(Default)]
pub struct SearchAfter {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
    iter: i32,
    all_sort_fields: Vec<SortField>,
}

/// `TestSearchAfter`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SearchAfter>(
        "org.apache.lucene.search.TestSearchAfter",
        vec![
            set_up(search_after_set_up),
            tear_down(search_after_tear_down),
            ftest("testQueries", test_queries),
        ],
    )
}

/// `new SortField(field, type, reverse)`.
fn sf(field: &str, t: SortFieldType, reverse: bool) -> SortField {
    SortField::new_reverse(Some(field.to_string()), t, reverse).expect("a valid sort field")
}

/// `new SortField(field, type, reverse, missingValue)`.
fn sf_missing(
    field: &str,
    t: SortFieldType,
    reverse: bool,
    missing: MissingValue,
) -> Result<SortField> {
    SortField::new_with_missing(Some(field.to_string()), t, reverse, Some(missing))
}

fn search_after_set_up(f: &mut SearchAfter) -> Result<()> {
    f.all_sort_fields = vec![
        sf("int", SortFieldType::Int, false),
        sf("long", SortFieldType::Long, false),
        sf("float", SortFieldType::Float, false),
        sf("double", SortFieldType::Double, false),
        sf("bytes", SortFieldType::String, false),
        sf("bytesval", SortFieldType::StringVal, false),
        sf("int", SortFieldType::Int, true),
        sf("long", SortFieldType::Long, true),
        sf("float", SortFieldType::Float, true),
        sf("double", SortFieldType::Double, true),
        sf("bytes", SortFieldType::String, true),
        sf("bytesval", SortFieldType::StringVal, true),
        SortField::FIELD_SCORE,
        SortField::FIELD_DOC,
    ];

    // Also test missing first / last for the "string" sorts:
    for field in ["bytes", "sortedbytesdocvalues"] {
        for rev in 0..2 {
            let reversed = rev == 0;
            let s = sf_missing(
                field,
                SortFieldType::String,
                reversed,
                MissingValue::StringFirst,
            )?;
            f.all_sort_fields.push(s);

            let s = sf_missing(
                field,
                SortFieldType::String,
                reversed,
                MissingValue::StringLast,
            )?;
            f.all_sort_fields.push(s);
        }
    }

    // Also test missing first / last for the "string_val" sorts:
    for field in ["sortedbytesdocvaluesval", "straightbytesdocvalues"] {
        for rev in 0..2 {
            let reversed = rev == 0;
            let s = sf_missing(
                field,
                SortFieldType::StringVal,
                reversed,
                MissingValue::StringFirst,
            )?;
            f.all_sort_fields.push(s);

            let s = sf_missing(
                field,
                SortFieldType::StringVal,
                reversed,
                MissingValue::StringLast,
            )?;
            f.all_sort_fields.push(s);
        }
    }

    let limit = f.all_sort_fields.len();
    for i in 0..limit {
        let s = f.all_sort_fields[i].clone();
        let field = s.field().unwrap_or_default().to_string();
        match s.field_type() {
            SortFieldType::Int => {
                let sf2 = sf_missing(
                    &field,
                    SortFieldType::Int,
                    s.reverse(),
                    MissingValue::Int(random().next_int()),
                )?;
                f.all_sort_fields.push(sf2);
            }
            SortFieldType::Long => {
                let sf2 = sf_missing(
                    &field,
                    SortFieldType::Long,
                    s.reverse(),
                    MissingValue::Long(random().next_long()),
                )?;
                f.all_sort_fields.push(sf2);
            }
            SortFieldType::Float => {
                let sf2 = sf_missing(
                    &field,
                    SortFieldType::Float,
                    s.reverse(),
                    MissingValue::Float(random().next_float()),
                )?;
                f.all_sort_fields.push(sf2);
            }
            SortFieldType::Double => {
                let sf2 = sf_missing(
                    &field,
                    SortFieldType::Double,
                    s.reverse(),
                    MissingValue::Double(random().next_double()),
                )?;
                f.all_sort_fields.push(sf2);
            }
            _ => {}
        }
    }

    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let num_docs = LuceneTestCase::at_least(200);
    let r = random();
    for i in 0..num_docs {
        let mut fields: Vec<Box<dyn IndexableField>> = Vec::new();
        fields.push(Box::new(LuceneTestCase::new_text_field(
            "english",
            &English::int_to_english(i),
            Store::NO,
        )));
        fields.push(Box::new(LuceneTestCase::new_text_field(
            "oddeven",
            if i % 2 == 0 { "even" } else { "odd" },
            Store::NO,
        )));
        fields.push(Box::new(NumericDocValuesField::new(
            "byte",
            i64::from(r.next_int() as i8),
        )));
        fields.push(Box::new(NumericDocValuesField::new(
            "short",
            i64::from(r.next_int() as i16),
        )));
        fields.push(Box::new(NumericDocValuesField::new(
            "int",
            i64::from(r.next_int()),
        )));
        fields.push(Box::new(NumericDocValuesField::new("long", r.next_long())));
        fields.push(Box::new(FloatDocValuesField::new("float", r.next_float())));
        fields.push(Box::new(DoubleDocValuesField::new(
            "double",
            r.next_double(),
        )));
        fields.push(Box::new(SortedDocValuesField::new(
            "bytes",
            BytesRef::from_char_sequence(&TestUtil::random_realistic_unicode_string(&*random())),
        )));
        fields.push(Box::new(BinaryDocValuesField::new(
            "bytesval",
            BytesRef::from_char_sequence(&TestUtil::random_realistic_unicode_string(&*random())),
        )));

        let mut document = Document::new();
        document.add(Box::new(StoredField::new_string("id", i.to_string())?));
        if verbose() {
            println!("  add doc id={i}");
        }
        for field in fields {
            // So we are sometimes missing that field:
            if random().next_int_bound(5) != 4 {
                if verbose() {
                    println!("    {}", field.name());
                }
                document.add(field);
            }
        }

        iw.add_document(&document)?;

        if random().next_int_bound(50) == 17 {
            iw.commit()?;
        }
    }
    let reader = iw.get_reader()?;
    iw.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    if verbose() {
        println!("  searcher={searcher:?}");
    }
    f.dir = Some(dir);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

fn search_after_tear_down(f: &mut SearchAfter) -> Result<()> {
    f.searcher = None;
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(dir) = f.dir.take() {
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestSearchAfter.testQueries`.
fn test_queries(f: &mut SearchAfter) -> Result<()> {
    // because the first page has a null 'after', we get a normal collector.
    // so we need to run the test a few times to ensure we will collect multiple
    // pages.
    let n = LuceneTestCase::at_least(20);
    for _i in 0..n {
        assert_query_sort(
            f,
            &(Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>),
            &SortArg::Null,
        )?;
        assert_query_sort(
            f,
            &(Arc::new(TermQuery::new(Term::from_text("english", "one"))) as Arc<dyn Query>),
            &SortArg::Null,
        )?;
        let mut bq = BooleanQueryBuilder::new();
        bq.add(
            Arc::new(TermQuery::new(Term::from_text("english", "one"))),
            Occur::SHOULD,
        )?;
        bq.add(
            Arc::new(TermQuery::new(Term::from_text("oddeven", "even"))),
            Occur::SHOULD,
        )?;
        assert_query_sort(f, &(Arc::new(bq.build()) as Arc<dyn Query>), &SortArg::Null)?;
    }
    Ok(())
}

/// The `Sort sort` argument of `assertQuery(Query, Sort)`: Java compares it
/// with `Sort.RELEVANCE` by identity.
enum SortArg {
    /// `null`.
    Null,
    /// `Sort.RELEVANCE`, the shared instance.
    Relevance,
    /// Any other sort.
    Other(Sort),
}

impl SortArg {
    fn sort(&self) -> Option<Sort> {
        match self {
            SortArg::Null => None,
            SortArg::Relevance => Some(Sort::new()),
            SortArg::Other(s) => Some(s.clone()),
        }
    }
}

/// `void assertQuery(Query query)`.
#[allow(dead_code)]
fn assert_query(f: &mut SearchAfter, query: &Arc<dyn Query>) -> Result<()> {
    assert_query_sort(f, query, &SortArg::Null)?;
    assert_query_sort(f, query, &SortArg::Relevance)?;
    // `Sort.INDEXORDER`.
    assert_query_sort(
        f,
        query,
        &SortArg::Other(Sort::new_fields(vec![SortField::FIELD_DOC])?),
    )?;
    let all_sort_fields = f.all_sort_fields.clone();
    for sort_field in all_sort_fields {
        assert_query_sort(
            f,
            query,
            &SortArg::Other(Sort::new_fields(vec![sort_field])?),
        )?;
    }
    for _i in 0..20 {
        let sort = get_random_sort(f)?;
        assert_query_sort(f, query, &SortArg::Other(sort))?;
    }
    Ok(())
}

/// `Sort getRandomSort()`.
fn get_random_sort(f: &SearchAfter) -> Result<Sort> {
    let n = TestUtil::next_int(&*random(), 2, 7);
    let mut sort_fields = Vec::with_capacity(n as usize);
    for _i in 0..n {
        sort_fields.push(
            f.all_sort_fields[random().next_int_bound(f.all_sort_fields.len() as i32) as usize]
                .clone(),
        );
    }
    Sort::new_fields(sort_fields)
}

/// The hits of a `TopDocs`, which are `FieldDoc`s when it is a
/// `TopFieldDocs` (`is_field_doc`).
struct Hits {
    total_hits: i64,
    score_docs: Vec<FieldDoc>,
    is_field_doc: bool,
}

/// `searcher.search(query, manager)` over a `TopScoreDocCollectorManager`
/// (`sort == null`) or a `TopFieldCollectorManager`.
fn search_hits(
    searcher: &IndexSearcher,
    query: &Arc<dyn Query>,
    sort: Option<&Sort>,
    num_hits: i32,
    after: Option<&FieldDoc>,
) -> Result<Hits> {
    match sort {
        None => {
            let manager =
                TopScoreDocCollectorManager::new(num_hits, after.map(|a| a.score_doc), i32::MAX)?;
            let top = searcher.search_with_manager(Arc::clone(query), &manager)?;
            Ok(Hits {
                total_hits: top.total_hits.value(),
                score_docs: top
                    .score_docs
                    .iter()
                    .map(|sd| FieldDoc {
                        score_doc: *sd,
                        fields: None,
                    })
                    .collect(),
                is_field_doc: false,
            })
        }
        Some(sort) => {
            let manager =
                TopFieldCollectorManager::new(sort.clone(), num_hits, after.cloned(), i32::MAX)?;
            let top = searcher.search_with_manager(Arc::clone(query), &manager)?;
            Ok(Hits {
                total_hits: top.total_hits.value(),
                score_docs: top.score_docs,
                is_field_doc: true,
            })
        }
    }
}

/// `TopFieldCollector.populateScores(hits.scoreDocs, searcher, query)`.
fn populate_scores(
    hits: &mut Hits,
    searcher: &IndexSearcher,
    query: &Arc<dyn Query>,
) -> Result<()> {
    let mut score_docs: Vec<ScoreDoc> = hits.score_docs.iter().map(|d| d.score_doc).collect();
    TopFieldCollector::populate_scores(&mut score_docs, searcher, Arc::clone(query))?;
    for (fd, sd) in hits.score_docs.iter_mut().zip(score_docs) {
        fd.score_doc.score = sd.score;
    }
    Ok(())
}

/// `void assertQuery(Query query, Sort sort)`.
fn assert_query_sort(
    f: &mut SearchAfter,
    query: &Arc<dyn Query>,
    sort_arg: &SortArg,
) -> Result<()> {
    let searcher = f.searcher.as_ref().expect("setUp ran").clone();
    let max_doc = searcher.get_index_reader().max_doc();
    let page_size = TestUtil::next_int(&*random(), 1, max_doc * 2);
    let sort = sort_arg.sort();
    if verbose() {
        println!(
            "\nassertQuery {}: query={} sort={} pageSize={page_size}",
            f.iter,
            rucene::search::query_to_string(&**query),
            sort.as_ref()
                .map_or("null".to_string(), |s| format!("{s:?}"))
        );
        f.iter += 1;
    }
    let do_scores = match sort_arg {
        SortArg::Null => false,
        SortArg::Relevance => true,
        SortArg::Other(_) => random().next_boolean(),
    };
    let mut all = search_hits(&searcher, query, sort.as_ref(), max_doc, None)?;
    if do_scores {
        populate_scores(&mut all, &searcher, query)?;
    }

    if verbose() {
        println!("  all.totalHits.value()={}", all.total_hits);
        let stored_fields = searcher.stored_fields()?;
        for (upto, score_doc) in all.score_docs.iter().enumerate() {
            println!(
                "    hit {upto}: id={:?} {:?}",
                stored_fields.document(score_doc.score_doc.doc)?.get("id"),
                score_doc
            );
        }
    }
    let mut page_start = 0usize;
    let mut last_bottom: Option<FieldDoc> = None;
    while (page_start as i64) < all.total_hits {
        if verbose() {
            println!("  iter lastBottom={last_bottom:?}");
        }
        let mut paged = search_hits(
            &searcher,
            query,
            sort.as_ref(),
            page_size,
            last_bottom.as_ref(),
        )?;
        if do_scores {
            populate_scores(&mut paged, &searcher, query)?;
        }

        if verbose() {
            println!("    {} hits on page", paged.score_docs.len());
        }

        if paged.score_docs.is_empty() {
            break;
        }
        assert_page(&searcher, page_start, &all, &paged)?;
        page_start += paged.score_docs.len();
        last_bottom = paged.score_docs.last().cloned();
    }
    assert_equals(&all.score_docs.len(), &page_start);
    Ok(())
}

/// `void assertPage(int pageStart, TopDocs all, TopDocs paged)`.
fn assert_page(
    searcher: &IndexSearcher,
    page_start: usize,
    all: &Hits,
    paged: &Hits,
) -> Result<()> {
    assert_equals(&all.total_hits, &paged.total_hits);
    let stored_fields = searcher.stored_fields()?;
    for i in 0..paged.score_docs.len() {
        let sd1 = &all.score_docs[page_start + i];
        let sd2 = &paged.score_docs[i];
        if verbose() {
            println!("    hit {}", page_start + i);
            println!(
                "      expected id={:?} {sd1:?}",
                stored_fields.document(sd1.score_doc.doc)?.get("id")
            );
            println!(
                "        actual id={:?} {sd2:?}",
                stored_fields.document(sd2.score_doc.doc)?.get("id")
            );
        }
        assert_equals(&sd1.score_doc.doc, &sd2.score_doc.doc);
        Assert::assert_equals_float_msg(None, sd1.score_doc.score, sd2.score_doc.score, 0.0);
        if all.is_field_doc {
            Assert::assert_true(paged.is_field_doc);
            assert_array_equals(
                sd1.fields.as_deref().unwrap_or_default(),
                sd2.fields.as_deref().unwrap_or_default(),
            );
        }
    }
    Ok(())
}
