//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPrefixRandom.java`.
//!
//! Create an index with random unicode terms Generates random prefix queries,
//! and validates against a simple impl.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::Store;
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::terms::{AcceptStatus, FilteredTermsEnum, FilteredTermsEnumFilter};
use rucene::index::{Term, Terms, TermsEnum};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, multi_term_query_eq, multi_term_query_hash, multi_term_rewrite,
    MultiTermQuery, RewriteMethod,
};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::{Query, QueryVisitor};
use rucene::util::attribute::AttributeSource;
use rucene::util::string_helper::StringHelper;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    close_dir, close_reader, doc_of, fixture_class, ftest, set_up, tear_down, term,
};
use crate::core_support::random;

/// The instance fields of `TestPrefixRandom`.
#[derive(Default)]
struct Fixture {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
}

/// `TestPrefixRandom`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<Fixture>(
        "org.apache.lucene.search.TestPrefixRandom",
        vec![
            set_up(fixture_set_up),
            tear_down(|f: &mut Fixture| {
                drop(f.searcher.take());
                close_reader(f.reader.take())?;
                close_dir(f.dir.take())
            }),
            ftest("testPrefixes", test_prefixes),
        ],
    )
}

fn fixture_set_up(f: &mut Fixture) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false);
    let mut config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;

    let mut field = LuceneTestCase::new_string_field("field", "", Store::NO);
    let num = LuceneTestCase::at_least(1000);
    for _ in 0..num {
        field.set_string_value(TestUtil::random_unicode_string_max(&*random(), 10))?;
        writer.add_document(&doc_of(&[&field])?)?;
    }
    let reader = writer.get_reader()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    writer.close()?;
    Ok(())
}

/// a stupid prefix query that just blasts thru the terms
///
/// (`private static class DumbPrefixQuery extends MultiTermQuery`.)
#[derive(Clone)]
struct DumbPrefixQuery {
    field: String,
    rewrite_method: Arc<dyn RewriteMethod>,
    prefix: BytesRef,
}

impl DumbPrefixQuery {
    fn new(term: &Term) -> Self {
        DumbPrefixQuery {
            field: term.field().to_string(),
            rewrite_method: constant_score_blended_rewrite(),
            prefix: term.bytes().clone(),
        }
    }
}

impl fmt::Debug for DumbPrefixQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_query_string(""))
    }
}

/// `private static class SimplePrefixTermsEnum extends FilteredTermsEnum`:
/// its `accept`.
struct SimplePrefixTermsEnum {
    prefix: BytesRef,
}

impl FilteredTermsEnumFilter for SimplePrefixTermsEnum {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        Ok(if StringHelper::starts_with(term, &self.prefix) {
            AcceptStatus::Yes
        } else {
            AcceptStatus::No
        })
    }
}

impl Query for DumbPrefixQuery {
    fn is_multi_term_query(&self) -> bool {
        true
    }

    /// `field + ":" + prefix`: `BytesRef.toString()`.
    fn to_query_string(&self, field: &str) -> String {
        format!("{field}:{}", self.prefix)
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        multi_term_rewrite(self, index_searcher)
    }

    fn query_eq(&self, obj: &dyn Query) -> bool {
        if !self.same_class_as(obj) {
            return false;
        }
        let Some(that) = obj.as_any().downcast_ref::<DumbPrefixQuery>() else {
            return false;
        };
        if !multi_term_query_eq(self, that) {
            return false;
        }
        self.prefix == that.prefix
    }

    fn query_hash(&self) -> u64 {
        multi_term_query_hash(self)
    }
}

impl MultiTermQuery for DumbPrefixQuery {
    fn get_field(&self) -> &str {
        &self.field
    }

    fn get_rewrite_method(&self) -> Arc<dyn RewriteMethod> {
        Arc::clone(&self.rewrite_method)
    }

    fn get_terms_enum_with_atts(
        &self,
        terms: &Arc<dyn Terms>,
        _atts: &mut AttributeSource,
    ) -> Result<Box<dyn TermsEnum>> {
        // `setInitialSeekTerm(new BytesRef(""))`.
        Ok(Box::new(FilteredTermsEnum::new_with_seek(
            terms.iterator()?,
            Box::new(SimplePrefixTermsEnum {
                prefix: self.prefix.clone(),
            }),
            BytesRef::from_char_sequence(""),
        )))
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn to_multi_term_query_arc(&self) -> Arc<dyn MultiTermQuery> {
        Arc::new(self.clone())
    }

    fn to_query_arc(&self) -> Arc<dyn Query> {
        Arc::new(self.clone())
    }
}

/// test a bunch of random prefixes
fn test_prefixes(f: &mut Fixture) -> Result<()> {
    let num = LuceneTestCase::at_least(100);
    for _ in 0..num {
        assert_same(f, &TestUtil::random_unicode_string_max(&*random(), 5))?;
    }
    Ok(())
}

/// check that the # of hits is the same as from a very simple prefixquery
/// implementation.
fn assert_same(f: &Fixture, prefix: &str) -> Result<()> {
    let searcher = f.searcher.as_ref().expect("setUp ran");
    let smart: Arc<dyn Query> = Arc::new(PrefixQuery::new(term("field", prefix))?);
    let dumb: Arc<dyn Query> = Arc::new(DumbPrefixQuery::new(&term("field", prefix)));

    let smart_docs = searcher.search(Arc::clone(&smart), 25)?;
    let dumb_docs = searcher.search(dumb, 25)?;
    CheckHits::check_equal(&*smart, &smart_docs.score_docs, &dumb_docs.score_docs);
    Ok(())
}
