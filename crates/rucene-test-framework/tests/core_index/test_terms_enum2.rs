//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermsEnum2.java`.

use std::collections::BTreeSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::{SeekStatus, Term};
use rucene::index::IndexReader;
use rucene::search::automaton_query::AutomatonQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::Query;
use rucene::util::automaton::{
    Automata, Automaton, CompiledAutomaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::automaton::AutomatonTestUtil;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, det, random};
use crate::support::{field_mut, npe};

/// The fields of `TestTermsEnum2`, set by `setUp()`.
#[derive(Default)]
struct TestTermsEnum2Instance {
    dir: Option<TestDirectory>,
    reader: Option<Arc<dyn IndexReader>>,
    searcher: Option<IndexSearcher>,
    /// the terms we put in the index
    terms: BTreeSet<BytesRef>,
    /// automata of the same
    terms_automaton: Option<Automaton>,
    num_iterations: i32,
}

impl TestTermsEnum2Instance {
    fn reader(&self) -> &Arc<dyn IndexReader> {
        self.reader.as_ref().unwrap_or_else(|| npe("reader"))
    }

    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().unwrap_or_else(|| npe("searcher"))
    }
}

/// A test method of the class.
fn method(name: &str, body: fn(&mut TestTermsEnum2Instance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestTermsEnum2Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// `TestTermsEnum2`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestTermsEnum2")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestTermsEnum2Instance::default))
        .method(method("setUp", set_up))
        .method(method("tearDown", tear_down))
        .method(method(
            "testFiniteVersusInfinite",
            test_finite_versus_infinite,
        ))
        .method(method("testSeeking", test_seeking))
        .method(method("testSeekingAndNexting", test_seeking_and_nexting))
        .method(method("testIntersect", test_intersect))
        .build()
}

/// `setUp()`.
fn set_up(inst: &mut TestTermsEnum2Instance) -> Result<()> {
    LuceneTestCase::super_set_up();
    inst.num_iterations = LuceneTestCase::at_least(50);
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false),
    )));
    iwc.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "",
        Store::YES,
    )));
    inst.terms = BTreeSet::new();

    let num = LuceneTestCase::at_least(200);
    for _ in 0..num {
        let s = TestUtil::random_unicode_string(&*random());
        field_mut(&mut doc, "field").set_string_value(s.clone())?;
        inst.terms.insert(BytesRef::from_char_sequence(&s));
        writer.add_document(&doc)?;
    }

    inst.terms_automaton = Some(Automata::make_string_union(inst.terms.iter().cloned())?);

    let reader = writer.get_reader()?;
    inst.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    inst.reader = Some(reader as Arc<dyn IndexReader>);
    writer.close()?;
    inst.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
fn tear_down(inst: &mut TestTermsEnum2Instance) -> Result<()> {
    inst.searcher = None;
    inst.reader
        .take()
        .unwrap_or_else(|| npe("reader"))
        .close()?;
    inst.dir.take().unwrap_or_else(|| npe("dir")).close()?;
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `Operations.determinize(new RegExp(reg, RegExp.NONE).toAutomaton(),
/// DEFAULT_DETERMINIZE_WORK_LIMIT)`.
fn random_determinized_regexp() -> Result<Automaton> {
    let reg = AutomatonTestUtil::random_regexp(&*random());
    Ok(det(Operations::determinize(
        &RegExp::with_flags(&reg, RegExp::NONE)?.to_automaton()?,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    )))
}

/// tests a pre-intersected automaton against the original
///
/// Port of `TestTermsEnum2.testFiniteVersusInfinite`.
fn test_finite_versus_infinite(inst: &mut TestTermsEnum2Instance) -> Result<()> {
    for _ in 0..inst.num_iterations {
        let automaton = random_determinized_regexp()?;
        let mut matched_terms: Vec<BytesRef> = Vec::new();
        for t in &inst.terms {
            if Operations::run(&automaton, &t.utf8_to_string()?) {
                matched_terms.push(t.clone());
            }
        }

        let alternate = Automata::make_string_union(matched_terms)?;
        // System.out.println("match " + matchedTerms.size() + " " + alternate.getNumberOfStates() + "
        // states, sigma=" + alternate.getStartPoints().length);
        // AutomatonTestUtil.minimizeSimple(alternate);
        // System.out.println("minimize done");
        let a1: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
            Term::from_text("field", ""),
            automaton,
        )?);
        let a2: Arc<dyn Query> = Arc::new(AutomatonQuery::new(
            Term::from_text("field", ""),
            alternate,
        )?);

        let orig_hits = inst.searcher().search(Arc::clone(&a1), 25)?.score_docs;
        let new_hits = inst.searcher().search(Arc::clone(&a2), 25)?.score_docs;
        CheckHits::check_equal(&*a1, &orig_hits, &new_hits);
    }
    Ok(())
}

/// seeks to every term accepted by some automata
///
/// Port of `TestTermsEnum2.testSeeking`.
fn test_seeking(inst: &mut TestTermsEnum2Instance) -> Result<()> {
    for _ in 0..inst.num_iterations {
        let automaton = random_determinized_regexp()?;
        let mut te = MultiTerms::get_terms(inst.reader(), "field")?
            .unwrap_or_else(|| npe("MultiTerms.getTerms"))
            .iterator()?;
        let mut unsorted_terms: Vec<BytesRef> = inst.terms.iter().cloned().collect();
        shuffle(&mut unsorted_terms, &*random());

        for term in &unsorted_terms {
            if Operations::run(&automaton, &term.utf8_to_string()?) {
                // term is accepted
                if random().next_boolean() {
                    // seek exact
                    Assert::assert_true(te.seek_exact(term)?);
                } else {
                    // seek ceil
                    assert_equals(&SeekStatus::FOUND, &te.seek_ceil(term)?);
                    assert_equals(term, &te.term()?);
                }
            }
        }
    }
    Ok(())
}

/// mixes up seek and next for all terms
///
/// Port of `TestTermsEnum2.testSeekingAndNexting`.
fn test_seeking_and_nexting(inst: &mut TestTermsEnum2Instance) -> Result<()> {
    for _ in 0..inst.num_iterations {
        let mut te = MultiTerms::get_terms(inst.reader(), "field")?
            .unwrap_or_else(|| npe("MultiTerms.getTerms"))
            .iterator()?;

        for term in &inst.terms {
            let c = random().next_int_bound(3);
            if c == 0 {
                assert_equals(&Some(term.clone()), &te.next()?);
            } else if c == 1 {
                assert_equals(&SeekStatus::FOUND, &te.seek_ceil(term)?);
                assert_equals(term, &te.term()?);
            } else {
                Assert::assert_true(te.seek_exact(term)?);
            }
        }
    }
    Ok(())
}

/// tests intersect: TODO start at a random term!
///
/// Port of `TestTermsEnum2.testIntersect`.
fn test_intersect(inst: &mut TestTermsEnum2Instance) -> Result<()> {
    for _ in 0..inst.num_iterations {
        let reg = AutomatonTestUtil::random_regexp(&*random());
        let automaton = RegExp::with_flags(&reg, RegExp::NONE)?.to_automaton()?;
        let automaton = det(Operations::determinize(
            &automaton,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        let ca = CompiledAutomaton::new(automaton.clone(), false, false, false)?;
        let mut te = MultiTerms::get_terms(inst.reader(), "field")?
            .unwrap_or_else(|| npe("MultiTerms.getTerms"))
            .intersect(&ca, None)?;
        let expected = det(Operations::determinize(
            &Operations::intersection(
                inst.terms_automaton
                    .as_ref()
                    .unwrap_or_else(|| npe("termsAutomaton")),
                &automaton,
            ),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        let mut found: BTreeSet<BytesRef> = BTreeSet::new();
        while te.next()?.is_some() {
            found.insert(BytesRef::deep_copy_of(&te.term()?));
        }

        let actual = det(Operations::determinize(
            &Automata::make_string_union(found)?,
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        ));
        Assert::assert_true(AutomatonTestUtil::same_language(&expected, &actual));
    }
    Ok(())
}
