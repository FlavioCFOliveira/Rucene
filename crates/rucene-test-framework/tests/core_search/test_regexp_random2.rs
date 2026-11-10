//! Port of `lucene/core/src/test/org/apache/lucene/search/TestRegexpRandom2.java`.
//!
//! Create an index with random unicode terms Generates random regexps, and
//! validates against a simple impl.
//!
//! `TestFieldCacheRewriteMethod extends TestRegexpRandom2` and overrides
//! `assertSame`: the fixture is generic over [`AssertSame`], the overridable
//! method, and the subclass reuses `setUp`, `tearDown` and `testRegexps`.
//!
//! `setUp` reuses one `Field` and one `SortedDocValuesField` across the
//! documents it adds (`setStringValue`/`setBytesValue`); a Rucene
//! `Document` owns its fields, so each document is rebuilt from the mutated
//! field and the current doc-values bytes, without drawing again.

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::document::{Document, Field, SortedDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::terms::{AcceptStatus, FilteredTermsEnum, FilteredTermsEnumFilter};
use rucene::index::{Term, Terms, TermsEnum};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, multi_term_query_eq,
    multi_term_query_hash, multi_term_rewrite, MultiTermQuery, RewriteMethod,
};
use rucene::search::{DefaultProvider, Query, QueryVisitor, RegexpQuery, TopDocs};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{
    Automaton, CharacterRunAutomaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::unicode_util::UnicodeUtil;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::automaton::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::{TestDirectory, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    close_dir, close_reader, copy_field, fixture_class, ftest, object_to_string, set_up, tear_down,
    term,
};
use crate::core_support::{det, random};

/// The overridable `protected void assertSame(String regexp)`.
pub trait AssertSame: Send + 'static {
    fn assert_same(f: &RegexpRandom2Fixture<Self>, regexp: &str) -> Result<()>
    where
        Self: Sized;
}

/// The instance fields of `TestRegexpRandom2` (and of its subclass).
pub struct RegexpRandom2Fixture<V> {
    pub searcher1: Option<IndexSearcher>,
    pub searcher2: Option<IndexSearcher>,
    pub searcher3: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    dir: Option<TestDirectory>,
    pub field_name: String,
    variant: PhantomData<fn() -> V>,
}

impl<V> Default for RegexpRandom2Fixture<V> {
    fn default() -> Self {
        RegexpRandom2Fixture {
            searcher1: None,
            searcher2: None,
            searcher3: None,
            reader: None,
            dir: None,
            field_name: String::new(),
            variant: PhantomData,
        }
    }
}

impl<V> RegexpRandom2Fixture<V> {
    pub fn searcher1(&self) -> &IndexSearcher {
        self.searcher1.as_ref().expect("setUp ran")
    }

    pub fn searcher2(&self) -> &IndexSearcher {
        self.searcher2.as_ref().expect("setUp ran")
    }

    pub fn searcher3(&self) -> &IndexSearcher {
        self.searcher3.as_ref().expect("setUp ran")
    }
}

/// `TestRegexpRandom2` itself.
pub struct RegexpRandom2;

/// `TestRegexpRandom2`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<RegexpRandom2Fixture<RegexpRandom2>>(
        "org.apache.lucene.search.TestRegexpRandom2",
        methods::<RegexpRandom2>(),
    )
}

/// `setUp`, `tearDown` and `testRegexps`, bound to `V`'s `assertSame`.
pub fn methods<V: AssertSame>() -> Vec<Method> {
    vec![
        set_up(regexp_random2_set_up::<V>),
        tear_down(|f: &mut RegexpRandom2Fixture<V>| {
            drop((f.searcher1.take(), f.searcher2.take(), f.searcher3.take()));
            close_reader(f.reader.take())?;
            close_dir(f.dir.take())
        }),
        ftest("testRegexps", test_regexps::<V>),
    ]
}

fn regexp_random2_set_up<V>(f: &mut RegexpRandom2Fixture<V>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    f.field_name = if random().next_boolean() {
        "field".to_string()
    } else {
        String::new()
    }; // sometimes use an empty string as field name
    let analyzer = MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false);
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let mut field: Field = LuceneTestCase::new_string_field(&f.field_name, "", Store::NO);
    // `Field dvField = new SortedDocValuesField(fieldName, newBytesRef())`.
    let mut dv_value: BytesRef = LuceneTestCase::new_bytes_ref();
    let mut terms: Vec<String> = Vec::new();
    let num = LuceneTestCase::at_least(200);
    for _ in 0..num {
        let s = TestUtil::random_unicode_string(&*random());
        field.set_string_value(s.clone())?;
        dv_value = LuceneTestCase::new_bytes_ref_str(&s); // dvField.setBytesValue
        terms.push(s);
        let mut doc = Document::new();
        doc.add(Box::new(copy_field(&field)?));
        doc.add(Box::new(SortedDocValuesField::new(
            &f.field_name,
            dv_value.clone(),
        )));
        writer.add_document(&doc)?;
    }
    drop(dv_value);

    if *VERBOSE {
        // utf16 order
        terms.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        println!("UTF16 order:");
        for s in &terms {
            println!("  {}", UnicodeUtil::to_hex_string(s));
        }
    }

    let reader = writer.get_reader()?;
    f.searcher1 = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.searcher2 = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.searcher3 = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.dir = Some(dir);
    writer.close()?;
    Ok(())
}

/// a stupid regexp query that just blasts thru the terms
///
/// (`private static class DumbRegexpQuery extends MultiTermQuery`.)
#[derive(Clone)]
struct DumbRegexpQuery {
    field: String,
    rewrite_method: Arc<dyn RewriteMethod>,
    automaton: Arc<Automaton>,
}

impl fmt::Debug for DumbRegexpQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_query_string(""))
    }
}

impl DumbRegexpQuery {
    fn new(term: &Term, flags: i32) -> Result<Self> {
        let re = RegExp::with_flags(&term.text(), flags)?;
        let automaton = det(Operations::determinize(
            &re.to_automaton()?,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        Ok(DumbRegexpQuery {
            field: term.field().to_string(),
            rewrite_method: constant_score_blended_rewrite(),
            automaton: Arc::new(automaton),
        })
    }
}

/// `private class SimpleAutomatonTermsEnum extends FilteredTermsEnum`: its
/// `accept`.
struct SimpleAutomatonTermsEnum {
    run_automaton: CharacterRunAutomaton,
}

impl FilteredTermsEnumFilter for SimpleAutomatonTermsEnum {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        // `utf16.copyUTF8Bytes(term.bytes, term.offset, term.length)`.
        let utf16 = String::from_utf8_lossy(&term.bytes[term.offset..term.offset + term.length])
            .into_owned();
        Ok(if self.run_automaton.run(&utf16) {
            AcceptStatus::Yes
        } else {
            AcceptStatus::No
        })
    }
}

impl Query for DumbRegexpQuery {
    fn is_multi_term_query(&self) -> bool {
        true
    }

    fn to_query_string(&self, field: &str) -> String {
        format!(
            "{field}{}",
            object_to_string(
                "org.apache.lucene.util.automaton.Automaton",
                &*self.automaton
            )
        )
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
        let Some(that) = obj.as_any().downcast_ref::<DumbRegexpQuery>() else {
            return false;
        };
        if !multi_term_query_eq(self, that) {
            return false;
        }
        // `Automaton` does not override `equals`: identity.
        Arc::ptr_eq(&self.automaton, &that.automaton)
    }

    fn query_hash(&self) -> u64 {
        multi_term_query_hash(self)
    }
}

impl MultiTermQuery for DumbRegexpQuery {
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
        let filter = SimpleAutomatonTermsEnum {
            run_automaton: CharacterRunAutomaton::new((*self.automaton).clone())?,
        };
        // `setInitialSeekTerm(newBytesRef(""))`.
        Ok(Box::new(FilteredTermsEnum::new_with_seek(
            terms.iterator()?,
            Box::new(filter),
            LuceneTestCase::new_bytes_ref_str(""),
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

/// test a bunch of random regular expressions
fn test_regexps<V: AssertSame>(f: &mut RegexpRandom2Fixture<V>) -> Result<()> {
    let num = LuceneTestCase::at_least(200);
    for _ in 0..num {
        let reg = AutomatonTestUtil::random_regexp(&*random());
        if *VERBOSE {
            println!("TEST: regexp='{reg}'");
        }
        V::assert_same(f, &reg)?;
    }
    Ok(())
}

/// `searcher.search(query, 25)`.
pub fn search25(searcher: &IndexSearcher, query: &Arc<dyn Query>) -> Result<TopDocs> {
    searcher.search(Arc::clone(query), 25)
}

impl AssertSame for RegexpRandom2 {
    /// check that the # of hits is the same as from a very simple
    /// regexpquery implementation.
    fn assert_same(f: &RegexpRandom2Fixture<Self>, regexp: &str) -> Result<()> {
        let smart: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, regexp),
            RegExp::NONE,
        )?);
        let nfa_query: Arc<dyn Query> = Arc::new(RegexpQuery::with_everything(
            term(&f.field_name, regexp),
            RegExp::NONE,
            0,
            &DefaultProvider,
            0,
            // TODO: The NFA query is not able to use rewrite method that will utilize the
            // concurrency
            constant_score_boolean_rewrite(),
            false,
        )?);
        let dumb: Arc<dyn Query> = Arc::new(DumbRegexpQuery::new(
            &term(&f.field_name, regexp),
            RegExp::NONE,
        )?);

        let smart_docs = search25(f.searcher1(), &smart)?;
        let dumb_docs = search25(f.searcher2(), &dumb)?;
        let nfa_docs = search25(f.searcher3(), &nfa_query)?;

        CheckHits::check_equal(&*smart, &smart_docs.score_docs, &dumb_docs.score_docs);
        CheckHits::check_equal(&*nfa_query, &nfa_docs.score_docs, &dumb_docs.score_docs);
        Ok(())
    }
}
