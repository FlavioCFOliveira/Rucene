//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermsEnum.java`.
//!
//! The instance fields `d` and `r` that `makeIndex` and `close` share are a
//! [`Fixture`] each test method creates, as JUnit creates one instance per
//! method. Every `HashSet` whose iteration order reaches the test iterates in
//! Java's order (`JavaHashSet`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store};
use rucene::error::Result;
use rucene::index::automaton_terms_enum::AutomatonTermsEnum;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_NONE};
use rucene::index::terms::{EmptyTermsEnum, SeekStatus, Term, TermState, TermsEnum};
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::util::automaton::{
    Automata, Automaton, CompiledAutomaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::per_thread_pk_lookup::PerThreadPKLookup;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{
    Nightly, SuppressCodecs, TestDirectory, RANDOM_MULTIPLIER,
};
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::java_hash_set::JavaHashSet;
use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_equals, assert_null, class_annotated, det,
    iae_message, random, test, verbose,
};
use crate::support::npe;

/// `TestTermsEnum`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestTermsEnum",
        vec![
            test("test", test_),
            test("testIntersectRandom", test_intersect_random),
            test("testEasy", test_easy),
            test("testFloorBlocks", test_floor_blocks),
            test("testZeroTerms", test_zero_terms),
            test("testRandomTerms", test_random_terms),
            test("testIntersectBasic", test_intersect_basic),
            test("testIntersectStartTerm", test_intersect_start_term),
            test("testIntersectEmptyString", test_intersect_empty_string),
            test("testCommonPrefixTerms", test_common_prefix_terms),
            test("testVaryingTermsPerSegment", test_varying_terms_per_segment).annotate(Nightly),
            test("testIntersectRegexp", test_intersect_regexp),
            test(
                "testInvalidAutomatonTermsEnum",
                test_invalid_automaton_terms_enum,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
        },
    )
}

/// `Collections.binarySearch(list, key)` / `Arrays.binarySearch(array, key)`
/// over sorted, distinct elements: the index, or `-(insertion point) - 1`.
fn binary_search(sorted: &[BytesRef], key: &BytesRef) -> i32 {
    match sorted.binary_search(key) {
        Ok(i) => i as i32,
        Err(p) => -(p as i32) - 1,
    }
}

/// `MultiTerms.getTerms(r, field).iterator()`.
fn terms_iterator(r: &Arc<dyn IndexReader>, field: &str) -> Result<Box<dyn TermsEnum>> {
    MultiTerms::get_terms(r, field)?
        .unwrap_or_else(|| npe("MultiTerms.getTerms"))
        .iterator()
}

/// `te.term()`, `None` for Java's `null`.
fn term(te: &dyn TermsEnum) -> Result<Option<BytesRef>> {
    te.term()
}

/// Port of `TestTermsEnum.test`.
fn test_() -> Result<()> {
    // Java's local `Random random` shadows nothing: `random()` stays the test's.
    let line_random = JdkRandom::new(random().next_long());
    let docs = LineFileDocs::with_default_file(&line_random)?;
    let d = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    let w = RandomIndexWriter::new(&*random(), d.as_directory(), Arc::new(analyzer))?;
    let num_docs = LuceneTestCase::at_least(10);
    for _ in 0..num_docs {
        w.add_document(&*docs.next_doc()?.borrow())?;
    }
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;

    let mut terms: Vec<BytesRef> = Vec::new();
    let mut terms_enum = terms_iterator(&r, "body")?;
    while let Some(t) = terms_enum.next()? {
        terms.push(BytesRef::deep_copy_of(&t));
    }
    if verbose() {
        println!("TEST: {} terms", terms.len());
    }

    let mut upto: i32 = -1;
    let iters = LuceneTestCase::at_least(200);
    for _ in 0..iters {
        if upto != -1 && random().next_boolean() {
            // next
            if verbose() {
                println!("TEST: iter next");
            }
            let is_end = terms_enum.next()?.is_none();
            upto += 1;
            if is_end {
                if verbose() {
                    println!("  end");
                }
                assert_equals(&(upto as usize), &terms.len());
                upto = -1;
            } else {
                if verbose() {
                    println!(
                        "  got term={} expected={}",
                        term(&*terms_enum)?
                            .ok_or_else(rucene::error::LuceneError::null_pointer)?
                            .utf8_to_string()?,
                        terms[upto as usize].utf8_to_string()?
                    );
                }
                Assert::assert_true((upto as usize) < terms.len());
                randomizedtesting::junit::Assert::assert_equals_nullable(
                    Some(terms[upto as usize].clone()),
                    term(&*terms_enum)?,
                );
            }
        } else {
            let target: BytesRef;
            let exists: &str;
            if random().next_boolean() {
                // likely fake term
                if random().next_boolean() {
                    target = LuceneTestCase::new_bytes_ref_str(&TestUtil::random_simple_string(
                        &*random(),
                    ));
                } else {
                    target = LuceneTestCase::new_bytes_ref_str(
                        &TestUtil::random_realistic_unicode_string(&*random()),
                    );
                }
                exists = "likely not";
            } else {
                // real term
                target = terms[random().next_int_bound(terms.len() as i32) as usize].clone();
                exists = "yes";
            }

            upto = binary_search(&terms, &target);

            if random().next_boolean() {
                if verbose() {
                    println!(
                        "TEST: iter seekCeil target={} exists={exists}",
                        target.utf8_to_string()?
                    );
                }
                // seekCeil
                let status = terms_enum.seek_ceil(&target)?;
                if verbose() {
                    println!("  got {status:?}");
                }

                if upto < 0 {
                    upto = -(upto + 1);
                    if upto as usize >= terms.len() {
                        assert_equals(&SeekStatus::END, &status);
                        upto = -1;
                    } else {
                        assert_equals(&SeekStatus::NOT_FOUND, &status);
                        randomizedtesting::junit::Assert::assert_equals_nullable(
                            Some(terms[upto as usize].clone()),
                            term(&*terms_enum)?,
                        );
                    }
                } else {
                    assert_equals(&SeekStatus::FOUND, &status);
                    randomizedtesting::junit::Assert::assert_equals_nullable(
                        Some(terms[upto as usize].clone()),
                        term(&*terms_enum)?,
                    );
                }
            } else {
                if verbose() {
                    println!(
                        "TEST: iter seekExact target={} exists={exists}",
                        target.utf8_to_string()?
                    );
                }
                // seekExact
                let result = terms_enum.seek_exact(&target)?;
                if verbose() {
                    println!("  got {result}");
                }
                if upto < 0 {
                    Assert::assert_false(result);
                    upto = -1;
                } else {
                    Assert::assert_true(result);
                    randomizedtesting::junit::Assert::assert_equals_nullable(
                        Some(target.clone()),
                        term(&*terms_enum)?,
                    );
                }
            }
        }
    }

    r.close()?;
    d.close()?;
    docs.close();
    Ok(())
}

/// `private void addDoc(RandomIndexWriter w, Collection<String> terms, Map<BytesRef, Integer>
/// termToID, int id)`.
fn add_doc(
    w: &RandomIndexWriter,
    terms: &mut Vec<String>,
    term_to_id: &mut HashMap<BytesRef, i32>,
    id: i32,
) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("id", i64::from(id))));
    if verbose() {
        println!("TEST: addDoc id:{id} terms=[{}]", terms.join(", "));
    }
    for s2 in terms.iter() {
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "f",
            s2,
            Store::NO,
        )));
        term_to_id.insert(LuceneTestCase::new_bytes_ref_str(s2), id);
    }
    w.add_document(&doc)?;
    terms.clear();
    Ok(())
}

/// `private boolean accepts(CompiledAutomaton c, BytesRef b)`.
fn accepts(c: &CompiledAutomaton, b: &BytesRef) -> bool {
    let run_automaton = c
        .run_automaton
        .as_ref()
        .unwrap_or_else(|| npe("runAutomaton"));
    let mut state = 0;
    for &byte in b.slice() {
        Assert::assert_true(state != -1);
        state = run_automaton.step(state, i32::from(byte));
    }
    run_automaton.is_accept(state)
}

/// Tests Terms.intersect
///
/// Port of `TestTermsEnum.testIntersectRandom`.
fn test_intersect_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let num_terms = LuceneTestCase::at_least(300);
    // final int numTerms = 50;

    let mut terms: JavaHashSet<String> = JavaHashSet::new();
    let mut pending_terms: Vec<String> = Vec::new();
    let mut term_to_id: HashMap<BytesRef, i32> = HashMap::new();
    let mut id = 0;
    while terms.size() != num_terms as usize {
        let s = get_random_string();
        if !terms.contains(&s) {
            terms.add(s.clone());
            pending_terms.push(s);
            if random().next_int_bound(20) == 7 {
                add_doc(&w, &mut pending_terms, &mut term_to_id, id)?;
                id += 1;
            }
        }
    }
    add_doc(&w, &mut pending_terms, &mut term_to_id, id)?;

    let mut terms_array: Vec<BytesRef> = Vec::with_capacity(terms.size());
    let mut terms_set: HashSet<BytesRef> = HashSet::new();
    {
        for s in terms.iter() {
            let b = LuceneTestCase::new_bytes_ref_str(s);
            terms_array.push(b.clone());
            terms_set.insert(b);
        }
        terms_array.sort();
    }

    if verbose() {
        println!("\nTEST: indexed terms (unicode order):");
        for t in &terms_array {
            println!(
                "  {} -> id:{}",
                t.utf8_to_string()?,
                term_to_id
                    .get(t)
                    .map_or_else(|| "null".to_string(), |v| v.to_string())
            );
        }
    }

    let r: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;

    let mut doc_id_to_id = vec![0i32; r.max_doc() as usize];
    let mut values = MultiDocValues::get_numeric_values(&r, "id")?
        .unwrap_or_else(|| npe("MultiDocValues.getNumericValues"));
    for i in 0..r.max_doc() {
        assert_equals(&i, &values.next_doc()?);
        doc_id_to_id[i as usize] = values.long_value()? as i32;
    }

    let num_iterations = LuceneTestCase::at_least(3);
    for iter in 0..num_iterations {
        // TODO: can we also test infinite As here...?

        // From the random terms, pick some ratio and compile an
        // automaton:
        let mut accept_terms: JavaHashSet<String> = JavaHashSet::new();
        let mut sorted_accept_terms: std::collections::BTreeSet<BytesRef> =
            std::collections::BTreeSet::new();
        let keep_pct = random().next_double();
        #[allow(clippy::needless_late_init)] // Java's definitely-assigned local, set in each branch
        let a: Automaton;
        if iter == 0 {
            if verbose() {
                println!("\nTEST: empty automaton");
            }
            a = Automata::make_empty();
        } else {
            if verbose() {
                println!("\nTEST: keepPct={keep_pct}");
            }
            for s in terms.iter() {
                let s2 = if random().next_double() <= keep_pct {
                    s.clone()
                } else {
                    get_random_string()
                };
                accept_terms.add(s2.clone());
                sorted_accept_terms.insert(LuceneTestCase::new_bytes_ref_str(&s2));
            }
            a = Automata::make_string_union(sorted_accept_terms.iter().cloned())?;
        }

        let c = CompiledAutomaton::new(a.clone(), true, false, false)?;

        let mut accept_terms_array: Vec<BytesRef> = Vec::with_capacity(accept_terms.size());
        let mut accept_terms_set: HashSet<BytesRef> = HashSet::new();
        for s in accept_terms.iter() {
            let b = LuceneTestCase::new_bytes_ref_str(s);
            accept_terms_array.push(b.clone());
            Assert::assert_true(accepts(&c, &b));
            accept_terms_set.insert(b);
        }
        accept_terms_array.sort();

        if verbose() {
            println!("\nTEST: accept terms (unicode order):");
            for t in &accept_terms_array {
                println!(
                    "  {}{}",
                    t.utf8_to_string()?,
                    if terms_set.contains(t) {
                        " (exists)"
                    } else {
                        ""
                    }
                );
            }
            println!("{}", a.to_dot());
        }

        for iter2 in 0..100 {
            let start_term: Option<BytesRef> =
                if accept_terms_array.is_empty() || random().next_boolean() {
                    None
                } else {
                    Some(
                        accept_terms_array
                            [random().next_int_bound(accept_terms_array.len() as i32) as usize]
                            .clone(),
                    )
                };

            if verbose() {
                println!(
                    "\nTEST: iter2={iter2} startTerm={}",
                    match &start_term {
                        None => "<null>".to_string(),
                        Some(t) => t.utf8_to_string()?,
                    }
                );

                if let Some(start_term) = &start_term {
                    let run_automaton = c
                        .run_automaton
                        .as_ref()
                        .unwrap_or_else(|| npe("runAutomaton"));
                    let mut state = 0;
                    for &byte in start_term.slice() {
                        let label = i32::from(byte);
                        println!("  state={state} label={label}");
                        state = run_automaton.step(state, label);
                        Assert::assert_true(state != -1);
                    }
                    println!("  state={state}");
                }
            }

            let mut te = MultiTerms::get_terms(&r, "f")?
                .unwrap_or_else(|| npe("MultiTerms.getTerms"))
                .intersect(&c, start_term.as_ref())?;

            let mut loc: i32 = match &start_term {
                None => 0,
                Some(start_term) => {
                    let loc = binary_search(&terms_array, &BytesRef::deep_copy_of(start_term));
                    if loc < 0 {
                        -(loc + 1)
                    } else {
                        // startTerm exists in index
                        loc + 1
                    }
                }
            };
            while (loc as usize) < terms_array.len()
                && !accept_terms_set.contains(&terms_array[loc as usize])
            {
                loc += 1;
            }

            let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;
            while (loc as usize) < terms_array.len() {
                let expected = &terms_array[loc as usize];
                let actual = te.next()?;
                if verbose() {
                    println!(
                        "TEST:   next() expected={} actual={}",
                        expected.utf8_to_string()?,
                        match &actual {
                            None => "null".to_string(),
                            Some(a) => a.utf8_to_string()?,
                        }
                    );
                }
                assert_equals(&Some(expected.clone()), &actual);
                assert_equals(&1, &te.doc_freq()?);
                let mut pe = TestUtil::docs_of(
                    &*random(),
                    &mut *te,
                    postings_enum.take(),
                    POSTINGS_ENUM_NONE,
                )?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
                let doc_id = pe.next_doc()?;
                Assert::assert_true(doc_id != NO_MORE_DOCS);
                assert_equals(
                    &doc_id_to_id[doc_id as usize],
                    term_to_id
                        .get(expected)
                        .unwrap_or_else(|| npe("termToID.get")),
                );
                postings_enum = Some(pe);
                loop {
                    loc += 1;
                    if !((loc as usize) < terms_array.len()
                        && !accept_terms_set.contains(&terms_array[loc as usize]))
                    {
                        break;
                    }
                }
            }
            assert_null(&te.next()?);
        }
    }

    r.close()?;
    dir.close()
}

/// The instance fields `d` and `r`.
#[derive(Default)]
struct Fixture {
    d: Option<TestDirectory>,
    r: Option<Arc<dyn IndexReader>>,
}

/// `private final String FIELD = "field"`.
const FIELD: &str = "field";

impl Fixture {
    /// `private IndexReader makeIndex(String... terms)`.
    fn make_index(&mut self, terms: &[String]) -> Result<Arc<dyn IndexReader>> {
        let d = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));

        /*
        iwc.setCodec(new StandardCodec(minTermsInBlock, maxTermsInBlock));
        */

        let w = RandomIndexWriter::new_with_config(&*random(), d.as_directory(), iwc)?;
        for term in terms {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_string_field(
                FIELD,
                term,
                Store::NO,
            )));
            w.add_document(&doc)?;
        }
        // `d = newDirectory()` replaced the field before `close()` runs, so
        // Java's `close()` closes the old reader and the new directory.
        let old_d = self.d.replace(d);
        if self.r.is_some() {
            self.close()?;
        }
        drop(old_d);
        let r: Arc<dyn IndexReader> = w.get_reader()?;
        self.r = Some(Arc::clone(&r));
        w.close()?;
        Ok(r)
    }

    /// `private void close()`.
    fn close(&mut self) -> Result<()> {
        self.r.as_ref().unwrap_or_else(|| npe("r")).close()?;
        self.d.as_mut().unwrap_or_else(|| npe("d")).close()
    }
}

/// `private int docFreq(IndexReader r, String term)`.
fn doc_freq(r: &Arc<dyn IndexReader>, term: &str) -> Result<i32> {
    r.doc_freq(&Term::from_text(FIELD, term))
}

/// `String...` of literals.
fn strings(terms: &[&str]) -> Vec<String> {
    terms.iter().map(|s| s.to_string()).collect()
}

/// Port of `TestTermsEnum.testEasy`.
fn test_easy() -> Result<()> {
    let mut f = Fixture::default();
    // No floor arcs:
    let r = f.make_index(&strings(&[
        "aa0", "aa1", "aa2", "aa3", "bb0", "bb1", "bb2", "bb3", "aa",
    ]))?;

    // First term in block:
    assert_equals(&1, &doc_freq(&r, "aa0")?);

    // Scan forward to another term in same block
    assert_equals(&1, &doc_freq(&r, "aa2")?);

    assert_equals(&1, &doc_freq(&r, "aa")?);

    // Reset same block then scan forwards
    assert_equals(&1, &doc_freq(&r, "aa1")?);

    // Not found, in same block
    assert_equals(&0, &doc_freq(&r, "aa5")?);

    // Found, in same block
    assert_equals(&1, &doc_freq(&r, "aa2")?);

    // Not found in index:
    assert_equals(&0, &doc_freq(&r, "b0")?);

    // Found:
    assert_equals(&1, &doc_freq(&r, "aa2")?);

    // Found, rewind:
    assert_equals(&1, &doc_freq(&r, "aa0")?);

    // First term in block:
    assert_equals(&1, &doc_freq(&r, "bb0")?);

    // Scan forward to another term in same block
    assert_equals(&1, &doc_freq(&r, "bb2")?);

    // Reset same block then scan forwards
    assert_equals(&1, &doc_freq(&r, "bb1")?);

    // Not found, in same block
    assert_equals(&0, &doc_freq(&r, "bb5")?);

    // Found, in same block
    assert_equals(&1, &doc_freq(&r, "bb2")?);

    // Not found in index:
    assert_equals(&0, &doc_freq(&r, "b0")?);

    // Found:
    assert_equals(&1, &doc_freq(&r, "bb2")?);

    // Found, rewind:
    assert_equals(&1, &doc_freq(&r, "bb0")?);

    f.close()
}

// tests:
//   - test same prefix has non-floor block and floor block (ie, has 2 long outputs on same term
// prefix)
//   - term that's entirely in the index

/// Port of `TestTermsEnum.testFloorBlocks`.
fn test_floor_blocks() -> Result<()> {
    let mut f = Fixture::default();
    let terms = strings(&[
        "aa0", "aa1", "aa2", "aa3", "aa4", "aa5", "aa6", "aa7", "aa8", "aa9", "aa", "xx",
    ]);
    let r = f.make_index(&terms)?;
    // r = makeIndex("aa0", "aa1", "aa2", "aa3", "aa4", "aa5", "aa6", "aa7", "aa8", "aa9");

    // First term in first block:
    assert_equals(&1, &doc_freq(&r, "aa0")?);
    assert_equals(&1, &doc_freq(&r, "aa4")?);

    // No block
    assert_equals(&0, &doc_freq(&r, "bb0")?);

    // Second block
    assert_equals(&1, &doc_freq(&r, "aa4")?);

    // Backwards to prior floor block:
    assert_equals(&1, &doc_freq(&r, "aa0")?);

    // Forwards to last floor block:
    assert_equals(&1, &doc_freq(&r, "aa9")?);

    assert_equals(&0, &doc_freq(&r, "a")?);
    assert_equals(&1, &doc_freq(&r, "aa")?);
    assert_equals(&0, &doc_freq(&r, "a")?);
    assert_equals(&1, &doc_freq(&r, "aa")?);

    // Forwards to last floor block:
    assert_equals(&1, &doc_freq(&r, "xx")?);
    assert_equals(&1, &doc_freq(&r, "aa1")?);
    assert_equals(&0, &doc_freq(&r, "yy")?);

    assert_equals(&1, &doc_freq(&r, "xx")?);
    assert_equals(&1, &doc_freq(&r, "aa9")?);

    assert_equals(&1, &doc_freq(&r, "xx")?);
    assert_equals(&1, &doc_freq(&r, "aa4")?);

    let mut te = terms_iterator(&r, FIELD)?;
    while te.next()?.is_some() {
        // System.out.println("TEST: next term=" + te.term().utf8ToString());
    }

    Assert::assert_true(seek_exact(&mut *te, "aa1")?);
    assert_equals(&Some("aa2".to_string()), &next(&mut *te)?);
    Assert::assert_true(seek_exact(&mut *te, "aa8")?);
    assert_equals(&Some("aa9".to_string()), &next(&mut *te)?);
    assert_equals(&Some("xx".to_string()), &next(&mut *te)?);

    test_random_seeks(&r, &terms)?;
    f.close()
}

/// Port of `TestTermsEnum.testZeroTerms`.
fn test_zero_terms() -> Result<()> {
    let mut f = Fixture {
        d: Some(LuceneTestCase::new_directory()),
        ..Fixture::default()
    };
    let w = RandomIndexWriter::new_with_mock_analyzer(
        &*random(),
        f.d.as_ref().expect("set").as_directory(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "one two three",
        Store::NO,
    )));
    let mut doc = {
        drop(doc);
        Document::new()
    };
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field2",
        "one two three",
        Store::NO,
    )));
    w.add_document(&doc)?;
    w.commit()?;
    w.delete_documents(Term::from_text("field", "one"))?;
    w.force_merge(1)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;
    assert_equals(&1, &r.num_docs());
    assert_equals(&1, &r.max_doc());
    if let Some(terms) = MultiTerms::get_terms(&r, "field")? {
        assert_null(&terms.iterator()?.next()?);
    }
    r.close()?;
    f.d.as_mut().expect("set").close()
}

/// `private String getRandomString()`.
fn get_random_string() -> String {
    // return _TestUtil.randomSimpleString(random());
    TestUtil::random_realistic_unicode_string(&*random())
}

/// Port of `TestTermsEnum.testRandomTerms`.
fn test_random_terms() -> Result<()> {
    let mut f = Fixture::default();
    let len = TestUtil::next_int(&*random(), 1, LuceneTestCase::at_least(1000)) as usize;
    let mut terms: Vec<String> = vec![String::new(); len];
    let mut seen: HashSet<String> = HashSet::new();

    let allow_empty_string = random().next_boolean();

    if random().next_int_bound(10) == 7 && terms.len() > 2 {
        // Sometimes add a bunch of terms sharing a longish common prefix:
        let num_terms_same_prefix = random().next_int_bound((terms.len() / 2) as i32) as usize;
        if num_terms_same_prefix > 0 {
            let prefix = loop {
                let prefix = get_random_string();
                // `String.length()`: UTF-16 code units.
                if prefix.encode_utf16().count() < 5 {
                    continue;
                } else {
                    break prefix;
                }
            };
            while seen.len() < num_terms_same_prefix {
                let t = format!("{prefix}{}", get_random_string());
                if !seen.contains(&t) {
                    terms[seen.len()] = t.clone();
                    seen.insert(t);
                }
            }
        }
    }

    while seen.len() < terms.len() {
        let t = get_random_string();
        if !seen.contains(&t) && (allow_empty_string || !t.is_empty()) {
            terms[seen.len()] = t.clone();
            seen.insert(t);
        }
    }
    let r = f.make_index(&terms)?;
    test_random_seeks(&r, &terms)?;
    f.close()
}

/// sugar
fn seek_exact(te: &mut dyn TermsEnum, term: &str) -> Result<bool> {
    te.seek_exact(&LuceneTestCase::new_bytes_ref_str(term))
}

/// sugar
fn next(te: &mut dyn TermsEnum) -> Result<Option<String>> {
    match te.next()? {
        None => Ok(None),
        Some(br) => Ok(Some(br.utf8_to_string()?)),
    }
}

/// `private BytesRef getNonExistTerm(BytesRef[] terms)`.
fn get_non_exist_term(terms: &[BytesRef]) -> BytesRef {
    loop {
        let ts = get_random_string();
        let t = LuceneTestCase::new_bytes_ref_str(&ts);
        if binary_search(terms, &t) < 0 {
            return t;
        }
    }
}

/// `private record TermAndState(BytesRef term, TermState state)`.
struct TermAndState {
    term: BytesRef,
    state: Box<dyn TermState>,
}

/// `private void testRandomSeeks(IndexReader r, String... validTermStrings)`.
fn test_random_seeks(r: &Arc<dyn IndexReader>, valid_term_strings: &[String]) -> Result<()> {
    let mut valid_terms: Vec<BytesRef> = valid_term_strings
        .iter()
        .map(|s| LuceneTestCase::new_bytes_ref_str(s))
        .collect();
    valid_terms.sort();
    if verbose() {
        println!("TEST: {} terms:", valid_terms.len());
        for t in &valid_terms {
            println!("  {} {t}", t.utf8_to_string()?);
        }
    }
    let mut te = terms_iterator(r, FIELD)?;

    let end_loc = -(valid_terms.len() as i32) - 1;

    let mut term_states: Vec<TermAndState> = Vec::new();

    for _ in 0..100 * *RANDOM_MULTIPLIER {
        let t: BytesRef;
        let mut loc: i32;
        let term_state: Option<usize>;
        if random().next_int_bound(6) == 4 {
            // pick term that doens't exist:
            t = get_non_exist_term(&valid_terms);
            term_state = None;
            if verbose() {
                println!("\nTEST: invalid term={}", t.utf8_to_string()?);
            }
            loc = binary_search(&valid_terms, &t);
        } else if !term_states.is_empty() && random().next_int_bound(4) == 1 {
            let i = random().next_int_bound(term_states.len() as i32) as usize;
            t = term_states[i].term.clone();
            loc = binary_search(&valid_terms, &t);
            Assert::assert_true(loc >= 0);
            term_state = Some(i);
            if verbose() {
                println!("\nTEST: valid termState term={}", t.utf8_to_string()?);
            }
        } else {
            // pick valid term
            loc = random().next_int_bound(valid_terms.len() as i32);
            t = BytesRef::deep_copy_of(&valid_terms[loc as usize]);
            term_state = None;
            if verbose() {
                println!("\nTEST: valid term={}", t.utf8_to_string()?);
            }
        }

        // seekCeil or seekExact:
        let do_seek_exact = random().next_boolean();
        if let Some(i) = term_state {
            if verbose() {
                println!("  seekExact termState");
            }
            te.seek_term_state(&t, &*term_states[i].state)?;
        } else if do_seek_exact {
            if verbose() {
                println!("  seekExact");
            }
            assert_equals(&(loc >= 0), &te.seek_exact(&t)?);
        } else {
            if verbose() {
                println!("  seekCeil");
            }

            let result = te.seek_ceil(&t)?;
            if verbose() {
                println!("  got {result:?}");
            }

            if loc >= 0 {
                assert_equals(&SeekStatus::FOUND, &result);
            } else if loc == end_loc {
                assert_equals(&SeekStatus::END, &result);
            } else {
                if loc < -(valid_terms.len() as i32) {
                    return Err(crate::core_support::assertion_error(""));
                }
                assert_equals(&SeekStatus::NOT_FOUND, &result);
            }
        }

        if loc >= 0 {
            randomizedtesting::junit::Assert::assert_equals_nullable(Some(t.clone()), term(&*te)?);
        } else if do_seek_exact {
            // TermsEnum is unpositioned if seekExact returns false
            continue;
        } else if loc == end_loc {
            continue;
        } else {
            loc = -loc - 1;
            randomizedtesting::junit::Assert::assert_equals_nullable(
                Some(valid_terms[loc as usize].clone()),
                term(&*te)?,
            );
        }

        // Do a bunch of next's after the seek
        let num_next = random().next_int_bound(valid_terms.len() as i32);

        for _ in 0..num_next {
            if verbose() {
                println!("\nTEST: next loc={loc} of {}", valid_terms.len());
            }
            let t2 = te.next()?;
            loc += 1;
            if loc as usize == valid_terms.len() {
                assert_null(&t2);
                break;
            } else {
                assert_equals(&Some(valid_terms[loc as usize].clone()), &t2);
                if random().next_int_bound(40) == 17 && term_states.len() < 100 {
                    term_states.push(TermAndState {
                        term: valid_terms[loc as usize].clone(),
                        state: te.term_state()?,
                    });
                }
            }
        }
    }
    Ok(())
}

/// The writer of the `testIntersect*` tests: `newIndexWriterConfig(new
/// MockAnalyzer(random()))` with `new LogDocMergePolicy()`.
fn log_doc_writer(dir: &TestDirectory) -> Result<RandomIndexWriter> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)
}

/// `te.next().utf8ToString()`.
fn next_string(te: &mut dyn TermsEnum) -> Result<String> {
    te.next()?
        .unwrap_or_else(|| npe("te.next()"))
        .utf8_to_string()
}

/// `te.postings(null, PostingsEnum.NONE).nextDoc()`.
fn first_doc(te: &mut dyn TermsEnum) -> Result<i32> {
    te.postings(None, POSTINGS_ENUM_NONE)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?
        .next_doc()
}

/// Port of `TestTermsEnum.testIntersectBasic`.
fn test_intersect_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = log_doc_writer(&dir)?;
    for value in ["aaa", "bbb", "ccc"] {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            value,
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    w.force_merge(1)?;
    let r = w.get_reader()?;
    w.close()?;
    let sub = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let terms = sub.terms("field")?.unwrap_or_else(|| npe("terms"));
    let automaton = RegExp::with_flags(".*", RegExp::NONE)?.to_automaton()?;
    let ca = CompiledAutomaton::new(automaton, false, false, false)?;
    let mut te = terms.intersect(&ca, None)?;
    assert_equals("aaa", next_string(&mut *te)?.as_str());
    assert_equals(&0, &first_doc(&mut *te)?);
    assert_equals("bbb", next_string(&mut *te)?.as_str());
    assert_equals(&1, &first_doc(&mut *te)?);
    assert_equals("ccc", next_string(&mut *te)?.as_str());
    assert_equals(&2, &first_doc(&mut *te)?);
    assert_null(&te.next()?);

    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("abc")))?;
    assert_equals("bbb", next_string(&mut *te)?.as_str());
    assert_equals(&1, &first_doc(&mut *te)?);
    assert_equals("ccc", next_string(&mut *te)?.as_str());
    assert_equals(&2, &first_doc(&mut *te)?);
    assert_null(&te.next()?);

    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("aaa")))?;
    assert_equals("bbb", next_string(&mut *te)?.as_str());
    assert_equals(&1, &first_doc(&mut *te)?);
    assert_equals("ccc", next_string(&mut *te)?.as_str());
    assert_equals(&2, &first_doc(&mut *te)?);
    assert_null(&te.next()?);

    r.close()?;
    dir.close()
}

/// Port of `TestTermsEnum.testIntersectStartTerm`.
fn test_intersect_start_term() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = log_doc_writer(&dir)?;
    for value in ["abc", "abd", "acd", "bcd"] {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            value,
            Store::NO,
        )));
        w.add_document(&doc)?;
    }

    w.force_merge(1)?;
    let r = w.get_reader()?;
    w.close()?;
    let sub = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let terms = sub.terms("field")?.unwrap_or_else(|| npe("terms"));

    let automaton = RegExp::with_flags(".*d", RegExp::NONE)?.to_automaton()?;
    let automaton = det(Operations::determinize(
        &automaton,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
    ));
    let ca = CompiledAutomaton::new(automaton, false, false, false)?;

    // should seek to startTerm
    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("aad")))?;
    assert_equals("abd", next_string(&mut *te)?.as_str());
    assert_equals(&1, &first_doc(&mut *te)?);
    assert_equals("acd", next_string(&mut *te)?.as_str());
    assert_equals(&2, &first_doc(&mut *te)?);
    assert_equals("bcd", next_string(&mut *te)?.as_str());
    assert_equals(&3, &first_doc(&mut *te)?);
    assert_null(&te.next()?);

    // should fail to find ceil label on second arc, rewind
    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("add")))?;
    assert_equals("bcd", next_string(&mut *te)?.as_str());
    assert_equals(&3, &first_doc(&mut *te)?);
    assert_null(&te.next()?);

    // should reach end
    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("bcd")))?;
    assert_null(&te.next()?);
    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("ddd")))?;
    assert_null(&te.next()?);

    r.close()?;
    dir.close()
}

/// Port of `TestTermsEnum.testIntersectEmptyString`.
fn test_intersect_empty_string() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = log_doc_writer(&dir)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "abc",
        Store::NO,
    )));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    // add empty string to both documents, so that singletonDocID == -1.
    // For a FST-based term dict, we'll expect to see the first arc is
    // flaged with HAS_FINAL_OUTPUT
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "abc",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "",
        Store::NO,
    )));
    w.add_document(&doc)?;

    w.force_merge(1)?;
    let r = w.get_reader()?;
    w.close()?;
    let sub = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    let terms = sub.terms("field")?.unwrap_or_else(|| npe("terms"));

    let automaton = RegExp::with_flags(".*", RegExp::NONE)?.to_automaton()?; // accept ALL
    let ca = CompiledAutomaton::new(automaton, false, false, false)?;

    let mut te = terms.intersect(&ca, None)?;

    assert_equals("", next_string(&mut *te)?.as_str());
    let mut de = te
        .postings(None, POSTINGS_ENUM_NONE)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&0, &de.next_doc()?);
    assert_equals(&1, &de.next_doc()?);

    assert_equals("abc", next_string(&mut *te)?.as_str());
    let mut de = te
        .postings(None, POSTINGS_ENUM_NONE)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&0, &de.next_doc()?);
    assert_equals(&1, &de.next_doc()?);

    assert_null(&te.next()?);

    // pass empty string
    let mut te = terms.intersect(&ca, Some(&LuceneTestCase::new_bytes_ref_str("")))?;

    assert_equals("abc", next_string(&mut *te)?.as_str());
    let mut de = te
        .postings(None, POSTINGS_ENUM_NONE)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&0, &de.next_doc()?);
    assert_equals(&1, &de.next_doc()?);

    assert_null(&te.next()?);

    r.close()?;
    dir.close()
}

/// LUCENE-5667
///
/// Port of `TestTermsEnum.testCommonPrefixTerms`.
fn test_common_prefix_terms() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let mut terms: JavaHashSet<String> = JavaHashSet::new();
    // String prefix = TestUtil.randomSimpleString(random(), 1, 20);
    let prefix = TestUtil::random_realistic_unicode_string_between(&*random(), 1, 20);
    let num_terms = LuceneTestCase::at_least(100);
    if verbose() {
        println!("TEST: {num_terms} terms; prefix={prefix}");
    }
    while terms.size() < num_terms as usize {
        // terms.add(prefix + TestUtil.randomSimpleString(random(), 1, 20));
        terms.add(format!(
            "{prefix}{}",
            TestUtil::random_realistic_unicode_string_between(&*random(), 1, 20)
        ));
    }
    for term in terms.iter() {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            term,
            Store::YES,
        )));
        w.add_document(&doc)?;
    }
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    if verbose() {
        println!("\nTEST: reader={r}");
    }

    let mut terms_enum = terms_iterator(&r, "id")?;
    let mut postings_enum: Option<Box<dyn PostingsEnum>> = None;
    let mut pk_lookup = PerThreadPKLookup::new(&r, "id")?;
    let stored_fields = r.stored_fields()?;

    let iters = LuceneTestCase::at_least(num_terms * 3);
    let terms_list: Vec<String> = terms.to_vec();
    for _ in 0..iters {
        let term: String;
        let should_exist: bool;
        if random().next_boolean() {
            term = terms_list[random().next_int_bound(terms.size() as i32) as usize].clone();
            should_exist = true;
        } else {
            term = format!(
                "{prefix}{}",
                TestUtil::random_simple_string_between(&*random(), 1, 20)
            );
            should_exist = terms.contains(&term);
        }

        if verbose() {
            println!("\nTEST: try term={term}");
            println!("  shouldExist?={should_exist}");
        }

        let term_bytes_ref = LuceneTestCase::new_bytes_ref_str(&term);

        let actual_result = terms_enum.seek_exact(&term_bytes_ref)?;
        assert_equals(&should_exist, &actual_result);
        if should_exist {
            let mut pe = terms_enum
                .postings(postings_enum.take(), 0)?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            let doc_id = pe.next_doc()?;
            Assert::assert_true(doc_id != NO_MORE_DOCS);
            assert_equals(&doc_id, &pk_lookup.lookup(&term_bytes_ref)?);
            postings_enum = Some(pe);
            let doc = stored_fields.document(doc_id)?;
            assert_equals(&Some(term.clone()), &doc.get("id"));

            if random().next_int_bound(7) == 1 {
                terms_enum.next()?;
            }
        } else {
            assert_equals(&-1, &pk_lookup.lookup(&term_bytes_ref)?);
        }

        if random().next_int_bound(7) == 1 {
            let status = terms_enum.seek_ceil(&term_bytes_ref)?;
            if should_exist {
                assert_equals(&SeekStatus::FOUND, &status);
            } else {
                assert_not_equals(&SeekStatus::FOUND, &status);
            }
        }
    }

    r.close()?;
    w.close()?;
    d.close()
}

/// Stresses out many-terms-in-root-block case:
///
/// Port of `TestTermsEnum.testVaryingTermsPerSegment`.
fn test_varying_terms_per_segment() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let mut terms: JavaHashSet<BytesRef> = JavaHashSet::new();
    let max_terms = LuceneTestCase::at_least(1000);
    while terms.size() < max_terms as usize {
        terms.add(LuceneTestCase::new_bytes_ref_str(
            &TestUtil::random_simple_string_between(&*random(), 1, 40),
        ));
    }
    let terms_list: Vec<BytesRef> = terms.to_vec();
    let mut sb = String::new();
    for term_count in 0..max_terms as usize {
        if verbose() {
            println!(
                "\nTEST: termCount={term_count} add term={}",
                terms_list[term_count].utf8_to_string()?
            );
        }
        sb.push(' ');
        sb.push_str(&terms_list[term_count].utf8_to_string()?);
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        iwc.set_open_mode(rucene::index::index_writer_config::OpenMode::CREATE)?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &sb,
            Store::NO,
        )));
        w.add_document(&doc)?;
        let r: Arc<dyn IndexReader> = w.get_reader()?;
        let leaves = Arc::clone(&r).leaves()?;
        assert_equals(&1, &leaves.len());
        let mut te = leaves[0]
            .leaf_reader()
            .terms("field")?
            .unwrap_or_else(|| npe("terms"))
            .iterator()?;
        for i in 0..=term_count {
            Assert::assert_true_msg(
                &format!(
                    "term '{}' should exist but doesn't",
                    terms_list[i].utf8_to_string()?
                ),
                te.seek_exact(&terms_list[i])?,
            );
        }
        for i in term_count + 1..terms_list.len() {
            Assert::assert_false_msg(
                &format!("term '{}' shouldn't exist but does", terms_list[i]),
                te.seek_exact(&terms_list[i])?,
            );
        }
        r.close()?;
        w.close()?;
    }
    dir.close()
}

/// LUCENE-7576
///
/// Port of `TestTermsEnum.testIntersectRegexp`.
fn test_intersect_regexp() -> Result<()> {
    let d = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field",
        "foobar",
        Store::NO,
    )));
    w.add_document(&doc)?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    let terms = MultiTerms::get_terms(&r, "field")?.unwrap_or_else(|| npe("terms"));
    let automaton =
        CompiledAutomaton::from_automaton(RegExp::new("do_not_match_anything")?.to_automaton()?)?;
    let message = iae_message(|| terms.intersect(&automaton, None));
    assert_equals_msg(
        None,
        "please use CompiledAutomaton.getTermsEnum instead",
        message.as_str(),
    );
    r.close()?;
    w.close()?;
    d.close()
}

/// LUCENE-7576
///
/// Port of `TestTermsEnum.testInvalidAutomatonTermsEnum`.
fn test_invalid_automaton_terms_enum() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        AutomatonTermsEnum::new(
            Box::new(EmptyTermsEnum::new()),
            CompiledAutomaton::from_automaton(Automata::make_string("foo"))?,
        )
    });
    Ok(())
}
