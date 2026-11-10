//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocValuesRewriteMethod.java`.
//!
//! Tests the DocValuesRewriteMethod.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, SortedSetDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::doc_values_rewrite_method::DocValuesRewriteMethod;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{DefaultProvider, Query, RegexpQuery};
use rucene::util::automaton::{RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::unicode_util::UnicodeUtil;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::automaton::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    close_dir, close_reader, fixture_class, ftest, set_up, tear_down, term,
};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_search::test_regexp_random2::search25;
use crate::core_support::random;

/// The instance of `TestDocValuesRewriteMethod`.
#[derive(Default)]
pub struct DocValuesRewrite {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
    field_name: String,
}

/// `TestDocValuesRewriteMethod`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<DocValuesRewrite>(
        "org.apache.lucene.search.TestDocValuesRewriteMethod",
        vec![
            set_up(dv_rewrite_set_up),
            tear_down(|f: &mut DocValuesRewrite| {
                drop(f.searcher.take());
                close_reader(f.reader.take())?;
                close_dir(f.dir.take())
            }),
            ftest("testRegexps", test_regexps),
            ftest("testEquals", test_equals),
        ],
    )
}

fn dv_rewrite_set_up(f: &mut DocValuesRewrite) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    f.field_name = if random().next_boolean() {
        "field".to_string()
    } else {
        String::new()
    }; // sometimes use an empty string as field name
    let analyzer = MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false);
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Arc::new(analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let mut terms: Vec<String> = Vec::new();
    let num = LuceneTestCase::at_least(200);
    for i in 0..num {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::NO,
        )));
        let num_terms = random().next_int_bound(4);
        for _ in 0..num_terms {
            let s = TestUtil::random_unicode_string(&*random());
            doc.add(Box::new(LuceneTestCase::new_string_field(
                &f.field_name,
                &s,
                Store::NO,
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                &f.field_name,
                BytesRef::from_char_sequence(&s),
            )));
            doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                &format!("{}_with-skip", f.field_name),
                BytesRef::from_char_sequence(&s),
            )));
            terms.push(s);
        }
        writer.add_document(&doc)?;
    }

    if *VERBOSE {
        // utf16 order
        terms.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        println!("UTF16 order:");
        for s in &terms {
            println!("  {} {}", UnicodeUtil::to_hex_string(s), s);
        }
    }

    let num_deletions = random().next_int_bound(num / 10);
    for _ in 0..num_deletions {
        writer.delete_documents(term("id", &random().next_int_bound(num).to_string()))?;
    }

    let reader = writer.get_reader()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    writer.close()?;
    Ok(())
}

/// test a bunch of random regular expressions
fn test_regexps(f: &mut DocValuesRewrite) -> Result<()> {
    let num = LuceneTestCase::at_least(1000);
    for _ in 0..num {
        let reg = AutomatonTestUtil::random_regexp(&*random());
        if *VERBOSE {
            println!("TEST: regexp={reg}");
        }
        assert_same(f, &reg)?;
    }
    Ok(())
}

/// `new RegexpQuery(new Term(field, regexp), RegExp.NONE, 0, name -> null,
/// Operations.DEFAULT_DETERMINIZE_WORK_LIMIT, new DocValuesRewriteMethod())`.
fn doc_values_regexp(field: &str, regexp: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(RegexpQuery::with_everything(
        term(field, regexp),
        RegExp::NONE,
        0,
        &DefaultProvider,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        Arc::new(DocValuesRewriteMethod),
        true,
    )?))
}

/// check that the # of hits is the same as if the query is run against the
/// inverted index
fn assert_same(f: &DocValuesRewrite, regexp: &str) -> Result<()> {
    let searcher = f.searcher.as_ref().expect("setUp ran");
    let doc_values = doc_values_regexp(&f.field_name, regexp)?;
    let doc_values_with_skip = doc_values_regexp(&format!("{}_with-skip", f.field_name), regexp)?;
    let inverted: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
        term(&f.field_name, regexp),
        RegExp::NONE,
    )?);

    let inverted_docs = search25(searcher, &inverted)?;
    let doc_values_docs = search25(searcher, &doc_values)?;
    let doc_values_with_skip_docs = search25(searcher, &doc_values_with_skip)?;

    CheckHits::check_equal(
        &*inverted,
        &inverted_docs.score_docs,
        &doc_values_docs.score_docs,
    );
    CheckHits::check_equal(
        &*inverted,
        &inverted_docs.score_docs,
        &doc_values_with_skip_docs.score_docs,
    );
    Ok(())
}

fn test_equals(f: &mut DocValuesRewrite) -> Result<()> {
    {
        let a1: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, "[aA]"),
            RegExp::NONE,
        )?);
        let a2: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, "[aA]"),
            RegExp::NONE,
        )?);
        let b: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, "[bB]"),
            RegExp::NONE,
        )?);
        assert_query_equals(&a1, &a2);
        Assert::assert_false(a1.query_eq(&*b));
    }

    {
        let a1 = doc_values_regexp(&f.field_name, "[aA]")?;
        let a2 = doc_values_regexp(&f.field_name, "[aA]")?;
        let b = doc_values_regexp(&f.field_name, "[bB]")?;
        assert_query_equals(&a1, &a2);
        Assert::assert_false(a1.query_eq(&*b));
        QueryUtils::check(&*a1);
    }
    Ok(())
}
