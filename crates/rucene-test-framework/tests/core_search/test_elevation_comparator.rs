//! Port of `lucene/core/src/test/org/apache/lucene/search/TestElevationComparator.java`,
//! with the package-private `ElevationComparatorSource` it declares.
//!
//! The `priority` map is shared, as the Java field is, between the test
//! (which fills it in `getElevatedQuery`) and the comparator source (which
//! reads it), through an `Arc<Mutex<…>>`. The anonymous `FieldComparator`
//! and its `LeafFieldComparator` share `values` and `bottomVal`; Rucene's
//! `SharedFieldComparator` holds them, the leaf being the per-segment state
//! (`context` and `fieldname`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, SortedDocValuesField, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::DocValues;
use rucene::index::index_writer::IndexWriter;
use rucene::index::reader_context::LeafReaderContext;
use rucene::index::LeafReader;
use rucene::search::field_comparator::{FieldComparator, SharedFieldComparator, SlotComparator};
use rucene::search::field_comparator_source::FieldComparatorSource;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::pruning::Pruning;
use rucene::search::scorable::Scorable;
use rucene::search::similarities::{BM25Similarity, ClassicSimilarity};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    BooleanQueryBuilder, BoostQuery, Occur, Query, Sort, SortField, SortFieldType, SortValue,
    TopFieldCollectorManager,
};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, score_docs_of, set_up, term,
};
use crate::core_support::{assert_equals, random};

type Priority = Arc<Mutex<HashMap<BytesRef, i32>>>;

/// The instance of `TestElevationComparator`.
#[derive(Default)]
pub struct ElevationComparator {
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    searcher: Option<IndexSearcher>,
    priority: Priority,
}

/// `TestElevationComparator`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<ElevationComparator>(
        "org.apache.lucene.search.TestElevationComparator",
        vec![
            set_up(elevation_set_up),
            // `super.tearDown()` runs first here, then the reader and the
            // directory are closed.
            randomizedtesting::jdk::reflect::Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|f: &mut ElevationComparator| {
                    rucene_test_framework::util::LuceneTestCase::super_tear_down();
                    let result = (|| {
                        drop(f.searcher.take());
                        close_reader(f.reader.take())?;
                        close_dir(f.directory.take())
                    })();
                    if let Err(e) = result {
                        rucene_test_framework::util::java_exceptions::rethrow_java(e)
                    }
                }),
            ftest("testSorting", test_sorting),
            ftest("testSortingReversed", test_sorting_reversed),
        ],
    )
}

fn elevation_set_up(f: &mut ElevationComparator) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let mut config =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    config.set_max_buffered_docs(2)?;
    config.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy_merge_factor(
        1000,
    )?))?;
    config.set_similarity(Arc::new(ClassicSimilarity::new()))?;
    let writer = IndexWriter::new(directory.as_directory(), config)?;
    writer.add_document(&adoc(&["id", "a", "title", "ipod", "str_s", "a"])?)?;
    writer.add_document(&adoc(&["id", "b", "title", "ipod ipod", "str_s", "b"])?)?;
    writer.add_document(&adoc(&[
        "id",
        "c",
        "title",
        "ipod ipod ipod",
        "str_s",
        "c",
    ])?)?;
    writer.add_document(&adoc(&["id", "x", "title", "boosted", "str_s", "x"])?)?;
    writer.add_document(&adoc(&[
        "id",
        "y",
        "title",
        "boosted boosted",
        "str_s",
        "y",
    ])?)?;
    writer.add_document(&adoc(&[
        "id",
        "z",
        "title",
        "boosted boosted boosted",
        "str_s",
        "z",
    ])?)?;

    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;

    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(BM25Similarity::new()));
    f.directory = Some(directory);
    f.reader = Some(reader);
    f.searcher = Some(searcher);
    Ok(())
}

fn test_sorting(f: &mut ElevationComparator) -> Result<()> {
    run_test(f, false)
}

fn test_sorting_reversed(f: &mut ElevationComparator) -> Result<()> {
    run_test(f, true)
}

fn run_test(f: &mut ElevationComparator, reversed: bool) -> Result<()> {
    let mut newq = BooleanQueryBuilder::new();
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(term("title", "ipod")));

    newq.add(query, Occur::SHOULD)?;
    newq.add(
        get_elevated_query(f, &["id", "a", "id", "x"])?,
        Occur::SHOULD,
    )?;

    let sort = Sort::new_fields(vec![
        SortField::new_custom(
            Some("id".to_string()),
            Arc::new(ElevationComparatorSource::new(Arc::clone(&f.priority))),
            false,
        )?,
        SortField::new_reverse(None, SortFieldType::Score, reversed)?,
    ])?;

    let searcher = f.searcher.as_ref().expect("setUp ran");
    let top_docs = searcher.search_with_manager(
        Arc::new(newq.build()),
        &TopFieldCollectorManager::new(sort, 50, None, i32::MAX)?,
    )?;
    let score_docs = score_docs_of(&top_docs.score_docs);
    let n_docs_returned = score_docs.len();

    assert_equals(&4, &n_docs_returned);

    // 0 & 3 were elevated
    assert_equals(&0, &score_docs[0].doc);
    assert_equals(&3, &score_docs[1].doc);

    if reversed {
        assert_equals(&1, &score_docs[2].doc);
        assert_equals(&2, &score_docs[3].doc);
    } else {
        assert_equals(&2, &score_docs[2].doc);
        assert_equals(&1, &score_docs[3].doc);
    }

    /*
     StoredFields storedFields = searcher.storedFields();
     for (int i = 0; i < nDocsReturned; i++) {
      ScoreDoc scoreDoc = topDocs.scoreDocs[i];
      ids[i] = scoreDoc.doc;
      scores[i] = scoreDoc.score;
      documents[i] = storedFields.document(ids[i]);
      System.out.println("ids[i] = " + ids[i]);
      System.out.println("documents[i] = " + documents[i]);
      System.out.println("scores[i] = " + scores[i]);
    }
     */
    Ok(())
}

fn get_elevated_query(f: &ElevationComparator, vals: &[&str]) -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    let mut max = (vals.len() as i32 / 2) + 5;
    let mut i = 0;
    while i < vals.len() - 1 {
        b.add(
            Arc::new(TermQuery::new(term(vals[i], vals[i + 1]))),
            Occur::SHOULD,
        )?;
        f.priority
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(BytesRef::from_char_sequence(vals[i + 1]), max);
        max -= 1;
        // System.out.println(" pri doc=" + vals[i+1] + " pri=" + (1+max));
        i += 2;
    }
    let q: Arc<dyn Query> = Arc::new(b.build());
    Ok(Arc::new(BoostQuery::new(q, 0.0)?))
}

fn adoc(vals: &[&str]) -> Result<Document> {
    let mut doc = Document::new();
    let mut i = 0;
    while i + 2 < vals.len() {
        doc.add(Box::new(LuceneTestCase::new_text_field(
            vals[i],
            vals[i + 1],
            Store::YES,
        )));
        if vals[i] == "id" {
            doc.add(Box::new(SortedDocValuesField::new(
                vals[i],
                BytesRef::from_char_sequence(vals[i + 1]),
            )));
        }
        i += 2;
    }
    Ok(doc)
}

/// `class ElevationComparatorSource extends FieldComparatorSource`.
#[derive(Debug)]
pub struct ElevationComparatorSource {
    priority: Priority,
}

impl ElevationComparatorSource {
    pub fn new(boosts: Priority) -> Self {
        ElevationComparatorSource { priority: boosts }
    }
}

impl FieldComparatorSource for ElevationComparatorSource {
    fn new_comparator(
        &self,
        fieldname: &str,
        num_hits: usize,
        _pruning: Pruning,
        _reversed: bool,
    ) -> Result<Box<dyn FieldComparator>> {
        Ok(Box::new(SharedFieldComparator::new(ElevationSlots {
            priority: Arc::clone(&self.priority),
            fieldname: fieldname.to_string(),
            values: vec![0; num_hits],
            bottom_val: 0,
        })))
    }
}

/// The anonymous `FieldComparator<Integer>`.
#[derive(Debug)]
struct ElevationSlots {
    priority: Priority,
    fieldname: String,
    values: Vec<i32>,
    bottom_val: i32,
}

/// The anonymous `LeafFieldComparator`: its `context`.
struct ElevationLeaf {
    reader: Arc<dyn LeafReader>,
}

impl ElevationSlots {
    fn doc_val(&self, leaf: &ElevationLeaf, doc: i32) -> Result<i32> {
        let mut id_index = DocValues::get_sorted(&*leaf.reader, &self.fieldname)?;
        if id_index.advance(doc)? == doc {
            let term = id_index.lookup_ord(id_index.ord_value()?)?;
            let prio = self
                .priority
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&term)
                .copied();
            Ok(prio.unwrap_or(0))
        } else {
            Ok(0)
        }
    }
}

impl SlotComparator for ElevationSlots {
    type Leaf = ElevationLeaf;

    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        // values will be small enough that there is no overflow concern
        self.values[slot2 as usize] - self.values[slot1 as usize]
    }

    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        // values will be small enough that there is no overflow concern
        match (first, second) {
            (SortValue::Int(first), SortValue::Int(second)) => second - first,
            _ => randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.ClassCastException",
            ),
        }
    }

    fn set_top_value(&mut self, _value: SortValue) {
        randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.UnsupportedOperationException",
        )
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Int(self.values[slot as usize])
    }

    fn new_leaf(&mut self, context: &LeafReaderContext) -> Result<ElevationLeaf> {
        Ok(ElevationLeaf {
            reader: context.leaf_reader(),
        })
    }

    fn set_bottom(&mut self, _leaf: &mut ElevationLeaf, slot: i32) -> Result<()> {
        self.bottom_val = self.values[slot as usize];
        Ok(())
    }

    fn compare_top(
        &mut self,
        _leaf: &mut ElevationLeaf,
        _doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn compare_bottom(
        &mut self,
        leaf: &mut ElevationLeaf,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        Ok(self.doc_val(leaf, doc)? - self.bottom_val)
    }

    fn copy(
        &mut self,
        leaf: &mut ElevationLeaf,
        slot: i32,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<()> {
        self.values[slot as usize] = self.doc_val(leaf, doc)?;
        Ok(())
    }

    fn set_scorer(&mut self, _leaf: &mut ElevationLeaf, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}
