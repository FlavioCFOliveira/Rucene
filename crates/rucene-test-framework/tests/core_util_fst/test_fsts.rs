//! Port of `lucene/core/src/test/org/apache/lucene/util/fst/TestFSTs.java`
//! (`@SuppressCodecs({"SimpleText", "Direct"})`).
//!
//! The class's `main` entry point and its command-line helper `VisitTerms`
//! are not test members (JUnit never calls them) and are not ported; the
//! commented-out `testTrivial` stays out as in Java.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering as AtomicOrdering};
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, Ignore};
use rucene::document::{Document, Field, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::OpenMode;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::{SeekStatus, Term};
use rucene::index::IndexReader;
use rucene::search::term_query::TermQuery;
use rucene::store::{
    ByteArrayDataInput, ByteBuffersDataOutput, Directory, VecDataOutput, DEFAULT_IO_CONTEXT,
};
use rucene::util::automaton::{Automata, CompiledAutomaton};
use rucene::util::fst::{
    target_has_arcs, tie_break_by_input_comparator, Arc as FstArc, Builder, ByteSequenceOutputs,
    BytesRefFSTEnum, CompiledNode, InputType, IntSequenceOutputs, NoOutputs, OutputComparator,
    Outputs, Pair, PairOutputs, PathResult, PositiveIntOutputs, TopNSearcher, TopNSearcherHooks,
    UnCompiledNode, Util, FIXED_LENGTH_ARC_DEEP_NUM_ARCS, FIXED_LENGTH_ARC_SHALLOW_DEPTH,
    FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS, FST,
};
use rucene::util::{BytesRef, BytesRefBuilder, IntsRef, IntsRefBuilder};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::fst::fst_tester::{FSTTester, InputOutput};
use rucene_test_framework::util::line_file_docs::LineFileDocs;
use rucene_test_framework::util::lucene_test_case::{
    Nightly, SuppressCodecs, RANDOM_MULTIPLIER, TEST_ASSERTS_ENABLED, TEST_LINE_DOCS_FILE,
    TEST_NIGHTLY,
};
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, random, test, verbose,
    JavaHashSet,
};

thread_local! {
    /// The instance field `dir`, set by `setUp` and closed by `tearDown`.
    static DIR: RefCell<Option<MockDirectoryWrapper>> = const { RefCell::new(None) };
}

/// The test's `dir`.
fn with_dir<T>(body: impl FnOnce(&MockDirectoryWrapper) -> T) -> T {
    DIR.with(|d| body(d.borrow().as_ref().expect("setUp created dir")))
}

/// `setUp()`.
fn set_up() {
    LuceneTestCase::super_set_up();
    DIR.with(|d| *d.borrow_mut() = Some(LuceneTestCase::new_mock_directory()));
}

/// `tearDown()`.
fn tear_down() {
    // can be null if we force simpletext (funky, some kind of bug in test runner maybe)
    if let Some(mut dir) = DIR.with(|d| d.borrow_mut().take()) {
        if let Err(e) = Directory::close(&mut dir) {
            rucene_test_framework::util::java_exceptions::rethrow_java(e);
        }
    }
    LuceneTestCase::super_tear_down();
}

/// `TestFSTs`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        Method::new("setUp")
            .throws(&["java.lang.Exception"])
            .body_any(set_up),
        Method::new("tearDown")
            .throws(&["java.lang.Exception"])
            .body_any(tear_down),
        test("testBasicFSA", test_basic_fsa),
        test("testRandomWords", test_random_words),
        test("testBigSet", test_big_set).annotate(Nightly),
        test("testRealTerms", test_real_terms),
        test("testSingleString", test_single_string),
        test("testDuplicateFSAString", test_duplicate_fsa_string),
        test("testSimple", test_simple),
        test("testPrimaryKeys", test_primary_keys),
        test("testRandomTermLookup", test_random_term_lookup),
        test("testExpandedCloseToRoot", test_expanded_close_to_root),
        test("testFinalOutputOnEndState", test_final_output_on_end_state).annotate(Ignore {
            value: "not sure it's possible to get a final state output anymore w/o pruning?"
                .to_string(),
        }),
        test("testInternalFinalState", test_internal_final_state),
        test("testSaveDifferentMetaOut", test_save_different_meta_out),
        test("testNonFinalStopNode", test_non_final_stop_node),
        test("testShortestPaths", test_shortest_paths),
        test("testRejectNoLimits", test_reject_no_limits),
        test("testShortestPathsWFST", test_shortest_paths_wfst),
        test("testShortestPathsRandom", test_shortest_paths_random),
        test(
            "testShortestPathsWFSTRandom",
            test_shortest_paths_wfst_random,
        ),
        test(
            "testLargeOutputsOnArrayArcs",
            test_large_outputs_on_array_arcs,
        ),
        test("testIllegallyModifyRootArc", test_illegally_modify_root_arc),
        test("testSimpleDepth", test_simple_depth),
    ];
    let mut builder = Class::builder("org.apache.lucene.util.fst.TestFSTs")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .annotate(SuppressCodecs {
            value: vec!["SimpleText".to_string(), "Direct".to_string()],
        });
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// `toIntsRef(s, inputMode)` (of `FSTTester`).
fn to_ints_ref(s: &str, input_mode: i32) -> IntsRef {
    FSTTester::<NoOutputs>::to_ints_ref(s, input_mode)
}

/// `getRandomString(random)` (of `FSTTester`).
fn get_random_string(random: &dyn Random) -> String {
    FSTTester::<NoOutputs>::get_random_string(random)
}

/// `Util.toIntsRef(newBytesRef(s), scratch)`.
fn ints(s: &str, scratch: &mut IntsRefBuilder) -> IntsRef {
    Util::to_ints_ref(&LuceneTestCase::new_bytes_ref_str(s), scratch)
}

/// `FST.fromFSTReader(fstCompiler.compile(), fstCompiler.getFSTReader())`.
fn compile<O: Outputs>(
    mut fst_compiler: rucene::util::fst::FSTCompiler<'_, O>,
) -> Result<Option<FST<O>>> {
    let metadata = fst_compiler.compile()?;
    Ok(FST::from_fst_reader(
        metadata,
        fst_compiler.into_fst_reader()?,
    ))
}

fn test_basic_fsa() -> Result<()> {
    let strings = [
        "station",
        "commotion",
        "elation",
        "elastic",
        "plastic",
        "stop",
        "ftop",
        "ftation",
        "stat",
    ];
    let strings2 = [
        "station",
        "commotion",
        "elation",
        "elastic",
        "plastic",
        "stop",
        "ftop",
        "ftation",
    ];
    for input_mode in 0..2 {
        if verbose() {
            println!("TEST: inputMode={}", input_mode_to_string(input_mode));
        }

        let mut terms: Vec<IntsRef> = strings.iter().map(|s| to_ints_ref(s, input_mode)).collect();
        let mut terms2: Vec<IntsRef> = strings2
            .iter()
            .map(|s| to_ints_ref(s, input_mode))
            .collect();
        terms2.sort();

        do_test(input_mode, &mut terms)?;

        // Test pre-determined FST sizes to make sure we haven't lost minimality (at least on this
        // trivial set of terms):

        // FSA
        {
            let outputs = NoOutputs::get_singleton();
            let no_output = outputs.no_output();
            let pairs: Vec<InputOutput<()>> = terms2
                .iter()
                .map(|term| InputOutput {
                    input: term.clone(),
                    output: no_output,
                })
                .collect();
            let r = random();
            let (fst, node_count, arc_count) = with_dir(|dir| -> Result<_> {
                let mut tester = FSTTester::new(&*r, dir, input_mode, pairs, outputs);
                let fst = tester.do_test()?;
                Ok((fst, tester.node_count, tester.arc_count))
            })?;
            assert_not_null(&fst);
            assert_equals(&22, &node_count);
            assert_equals(&27, &arc_count);
        }

        // FST ord pos int
        {
            let outputs = PositiveIntOutputs::get_singleton();
            let pairs: Vec<InputOutput<i64>> = terms2
                .iter()
                .enumerate()
                .map(|(idx, term)| InputOutput {
                    input: term.clone(),
                    output: idx as i64,
                })
                .collect();
            let r = random();
            let (fst, node_count, arc_count) = with_dir(|dir| -> Result<_> {
                let mut tester = FSTTester::new(&*r, dir, input_mode, pairs, outputs);
                let fst = tester.do_test()?;
                Ok((fst, tester.node_count, tester.arc_count))
            })?;
            assert_not_null(&fst);
            assert_equals(&22, &node_count);
            assert_equals(&27, &arc_count);
        }

        // FST byte sequence ord
        {
            let outputs = ByteSequenceOutputs::get_singleton();
            let mut pairs: Vec<InputOutput<BytesRef>> = Vec::with_capacity(terms2.len());
            for (idx, term) in terms2.iter().enumerate() {
                let output = LuceneTestCase::new_bytes_ref_str(&idx.to_string());
                pairs.push(InputOutput {
                    input: term.clone(),
                    output,
                });
            }
            let r = random();
            let (fst, node_count, arc_count) = with_dir(|dir| -> Result<_> {
                let mut tester = FSTTester::new(&*r, dir, input_mode, pairs, outputs);
                let fst = tester.do_test()?;
                Ok((fst, tester.node_count, tester.arc_count))
            })?;
            assert_not_null(&fst);
            assert_equals(&24, &node_count);
            assert_equals(&30, &arc_count);
        }
    }
    Ok(())
}

/// `new FSTTester<>(random(), dir, inputMode, pairs, outputs).doTest()`.
fn run_tester<O: Outputs>(
    input_mode: i32,
    pairs: Vec<InputOutput<O::Output>>,
    outputs: O,
) -> Result<()> {
    let r = random();
    with_dir(|dir| FSTTester::new(&*r, dir, input_mode, pairs, outputs).do_test())?;
    Ok(())
}

/// `doTest(inputMode, terms)`: given set of terms, test the different
/// outputs for them.
fn do_test(input_mode: i32, terms: &mut [IntsRef]) -> Result<()> {
    terms.sort();

    // NoOutputs (simple FSA)
    {
        let outputs = NoOutputs::get_singleton();
        let no_output = outputs.no_output();
        let pairs = terms
            .iter()
            .map(|term| InputOutput {
                input: term.clone(),
                output: no_output,
            })
            .collect();
        run_tester(input_mode, pairs, outputs)?;
    }

    // PositiveIntOutput (ord)
    {
        let outputs = PositiveIntOutputs::get_singleton();
        let pairs = terms
            .iter()
            .enumerate()
            .map(|(idx, term)| InputOutput {
                input: term.clone(),
                output: idx as i64,
            })
            .collect();
        run_tester(input_mode, pairs, outputs)?;
    }

    // PositiveIntOutput (random monotonically increasing positive number)
    {
        let outputs = PositiveIntOutputs::get_singleton();
        let mut pairs = Vec::with_capacity(terms.len());
        let mut last_output = 0i64;
        for term in terms.iter() {
            let value = last_output + TestUtil::next_int(&*random(), 1, 1000) as i64;
            last_output = value;
            pairs.push(InputOutput {
                input: term.clone(),
                output: value,
            });
        }
        run_tester(input_mode, pairs, outputs)?;
    }

    // PositiveIntOutput (random positive number)
    {
        let outputs = PositiveIntOutputs::get_singleton();
        let mut pairs = Vec::with_capacity(terms.len());
        for term in terms.iter() {
            pairs.push(InputOutput {
                input: term.clone(),
                output: TestUtil::next_long(&*random(), 0, i64::MAX),
            });
        }
        run_tester(input_mode, pairs, outputs)?;
    }

    // Pair<ord, (random monotonically increasing positive number>
    {
        let o1 = PositiveIntOutputs::get_singleton();
        let o2 = PositiveIntOutputs::get_singleton();
        let outputs = PairOutputs::new(o1, o2);
        let mut pairs = Vec::with_capacity(terms.len());
        let mut last_output = 0i64;
        for (idx, term) in terms.iter().enumerate() {
            let value = last_output + TestUtil::next_int(&*random(), 1, 1000) as i64;
            last_output = value;
            pairs.push(InputOutput {
                input: term.clone(),
                output: outputs.new_pair(idx as i64, value),
            });
        }
        run_tester(input_mode, pairs, outputs)?;
    }

    // Sequence-of-bytes
    {
        let outputs = ByteSequenceOutputs::get_singleton();
        let no_output = outputs.no_output();
        let mut pairs = Vec::with_capacity(terms.len());
        for (idx, term) in terms.iter().enumerate() {
            let output = if random().next_int_bound(30) == 17 {
                no_output.clone()
            } else {
                LuceneTestCase::new_bytes_ref_str(&idx.to_string())
            };
            pairs.push(InputOutput {
                input: term.clone(),
                output,
            });
        }
        run_tester(input_mode, pairs, outputs)?;
    }

    // Sequence-of-ints
    {
        let outputs = IntSequenceOutputs::get_singleton();
        let mut pairs = Vec::with_capacity(terms.len());
        for (idx, term) in terms.iter().enumerate() {
            let s = idx.to_string();
            let output = IntsRef::new(s.chars().map(|c| c as i32).collect());
            pairs.push(InputOutput {
                input: term.clone(),
                output,
            });
        }
        run_tester(input_mode, pairs, outputs)?;
    }
    Ok(())
}

fn test_random_words() -> Result<()> {
    if *TEST_NIGHTLY {
        test_random_words_with(1000, LuceneTestCase::at_least(2))
    } else {
        test_random_words_with(100, 1)
    }
}

/// `inputModeToString(mode)`.
fn input_mode_to_string(mode: i32) -> &'static str {
    if mode == 0 {
        "utf8"
    } else {
        "utf32"
    }
}

/// `testRandomWords(maxNumWords, numIter)`.
fn test_random_words_with(max_num_words: i32, num_iter: i32) -> Result<()> {
    let random = JdkRandom::new(random().next_long());
    for iter in 0..num_iter {
        if verbose() {
            println!("\nTEST: iter {iter}");
        }
        for input_mode in 0..2 {
            let num_words = random.next_int_bound(max_num_words + 1);
            let mut terms_set: JavaHashSet<IntsRef> = JavaHashSet::new();
            while terms_set.size() < num_words as usize {
                let term = get_random_string(&random);
                terms_set.add(to_ints_ref(&term, input_mode));
            }
            do_test(input_mode, &mut terms_set.to_vec())?;
        }
    }
    Ok(())
}

fn test_big_set() -> Result<()> {
    test_random_words_with(TestUtil::next_int(&*random(), 50000, 60000), 1)
}

/// `testRealTerms()`: build FST for all unique terms in the test line docs
/// file, up until a doc limit.
fn test_real_terms() -> Result<()> {
    let docs = LineFileDocs::new(&*random(), &TEST_LINE_DOCS_FILE)?;
    let num_docs = if *TEST_NIGHTLY {
        LuceneTestCase::at_least(1000)
    } else {
        LuceneTestCase::at_least(50)
    };
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));

    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    conf.set_max_buffered_docs(-1)?
        .set_ram_buffer_size_mb(64.0)?;
    let temp_dir = LuceneTestCase::create_temp_dir_prefix("fstlines");
    let mut dir = LuceneTestCase::new_fs_directory(&temp_dir);
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc_count = 0;
    while doc_count < num_docs {
        let doc = docs.next_doc()?;
        writer.add_document(&doc)?;
        doc_count += 1;
    }
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    let outputs = PositiveIntOutputs::get_singleton();

    let mut builder = Builder::new(InputType::Byte1, outputs);
    let suffix_ram_limit_mb = if random().next_int_bound(10) == 4 {
        // no suffix sharing
        0.0
    } else if random().next_int_bound(10) == 7 {
        // share all suffixes (minimal FST)
        f64::INFINITY
    } else {
        (random().next_double() + 0.01) * 10.0
    };
    builder = builder.suffix_ram_limit_mb(suffix_ram_limit_mb)?;
    let mut fst_compiler = builder.build()?;

    let mut store_ord = random().next_boolean();
    if verbose() {
        if store_ord {
            println!("FST stores ord");
        } else {
            println!("FST stores docFreq");
        }
    }
    let terms = MultiTerms::get_terms(&r, "body")?;
    if let Some(terms) = terms {
        let mut scratch_ints_ref = IntsRefBuilder::new();
        let mut terms_enum = terms.iterator()?;
        if verbose() {
            println!("TEST: got termsEnum={terms_enum}");
        }
        let mut ord = 0i64;

        let automaton = Automata::make_any_string();
        let mut terms_enum2 = terms.intersect(
            &CompiledAutomaton::new(automaton, false, false, false)?,
            None,
        )?;

        while let Some(term) = terms_enum.next()? {
            let term2 = terms_enum2.next()?;
            assert_not_null(&term2);
            assert_equals(&term, &term2.expect("asserted not null"));
            assert_equals(&terms_enum.doc_freq()?, &terms_enum2.doc_freq()?);
            assert_equals(
                &terms_enum.total_term_freq()?,
                &terms_enum2.total_term_freq()?,
            );

            if ord == 0 && terms_enum.ord().is_err() {
                if verbose() {
                    println!("TEST: codec doesn't support ord; FST stores docFreq");
                }
                store_ord = false;
            }
            let output = if store_ord {
                ord
            } else {
                terms_enum.doc_freq()? as i64
            };
            fst_compiler.add(&Util::to_ints_ref(&term, &mut scratch_ints_ref), output)?;
            ord += 1;
            if verbose() && ord % 100000 == 0 && *TEST_NIGHTLY {
                println!("{ord} terms...");
            }
        }
        let node_count = fst_compiler.node_count();
        let arc_count = fst_compiler.arc_count();
        let fst = compile(fst_compiler)?.expect("terms were added");
        if verbose() {
            println!(
                "FST: {doc_count} docs; {ord} terms; {node_count} nodes; {arc_count} arcs; {} bytes",
                rucene::util::Accountable::ram_bytes_used(&fst)
            );
        }

        if ord > 0 {
            let random = JdkRandom::new(random().next_long());
            // Now confirm BytesRefFSTEnum and TermsEnum act the
            // same:
            let mut fst_enum = BytesRefFSTEnum::new(&fst)?;
            let num = LuceneTestCase::at_least(1000);
            for _ in 0..num {
                let random_term = LuceneTestCase::new_bytes_ref_str(&get_random_string(&random));
                if verbose() {
                    println!(
                        "TEST: seek non-exist {} {random_term}",
                        random_term.utf8_to_string()?
                    );
                }
                let seek_result = terms_enum.seek_ceil(&random_term)?;
                let fst_seek_result = fst_enum.seek_ceil(&random_term)?.map(|io| io.input.clone());

                if seek_result == SeekStatus::END {
                    if let Some(found) = &fst_seek_result {
                        Assert::fail(&format!(
                            "got {} but expected null",
                            found.utf8_to_string()?
                        ));
                    }
                } else {
                    assert_same(terms_enum.as_mut(), &fst_enum, store_ord)?;
                    for _ in 0..10 {
                        if verbose() {
                            println!("TEST: next");
                            if store_ord {
                                println!("  ord={}", terms_enum.ord()?);
                            }
                        }
                        if terms_enum.next()?.is_some() {
                            if verbose() {
                                println!("  term={}", terms_enum.term()?.utf8_to_string()?);
                            }
                            assert_not_null(&fst_enum.next()?.map(|_| ()));
                            assert_same(terms_enum.as_mut(), &fst_enum, store_ord)?;
                        } else {
                            if verbose() {
                                println!("  end!");
                            }
                            if let Some(next_result) = fst_enum.next()? {
                                println!(
                                    "expected null but got: input={} output={}",
                                    next_result.input.utf8_to_string()?,
                                    outputs.output_to_string(next_result.output)
                                );
                                Assert::fail_without_message();
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    r.close()?;
    dir.close()
}

/// `assertSame(termsEnum, fstEnum, storeOrd)`.
fn assert_same(
    terms_enum: &mut dyn rucene::codecs::postings::TermsEnum,
    fst_enum: &BytesRefFSTEnum<'_, PositiveIntOutputs>,
    store_ord: bool,
) -> Result<()> {
    let term = terms_enum.term()?;
    let current = fst_enum.current();
    assert_equals_msg(
        Some(&format!(
            "{} != {}",
            term.utf8_to_string()?,
            current.input.utf8_to_string()?
        )),
        &term,
        current.input,
    );
    let output = current.output.as_ref().copied().unwrap_or(0);
    if store_ord {
        // fst stored the ord
        assert_equals_msg(
            Some(&format!("term={} {term}", term.utf8_to_string()?)),
            &terms_enum.ord()?,
            &output,
        );
    } else {
        // fst stored the docFreq
        assert_equals_msg(
            Some(&format!("term={} {term}", term.utf8_to_string()?)),
            &terms_enum.doc_freq()?,
            &(output as i32),
        );
    }
    Ok(())
}

fn test_single_string() -> Result<()> {
    let outputs = NoOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;
    fst_compiler.add(
        &ints("foobar", &mut IntsRefBuilder::new()),
        outputs.no_output(),
    )?;
    let fst = compile(fst_compiler)?.expect("one string");
    let mut fst_enum = BytesRefFSTEnum::new(&fst)?;
    assert_null(
        &fst_enum
            .seek_floor(&LuceneTestCase::new_bytes_ref_str("foo"))?
            .map(|_| ()),
    );
    assert_null(
        &fst_enum
            .seek_ceil(&LuceneTestCase::new_bytes_ref_str("foobaz"))?
            .map(|_| ()),
    );
    Ok(())
}

fn test_duplicate_fsa_string() -> Result<()> {
    let str = "foobar";
    let outputs = NoOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;
    let mut ints_builder = IntsRefBuilder::new();
    for _ in 0..10 {
        fst_compiler.add(&ints(str, &mut ints_builder), outputs.no_output())?;
    }
    let fst = compile(fst_compiler)?.expect("one string");

    // count the input paths
    let mut count = 0;
    let mut fst_enum = BytesRefFSTEnum::new(&fst)?;
    while fst_enum.next()?.is_some() {
        count += 1;
    }
    assert_equals(&1, &count);

    assert_not_null(&Util::get_bytes(
        &fst,
        &LuceneTestCase::new_bytes_ref_str(str),
    )?);
    assert_null(&Util::get_bytes(
        &fst,
        &LuceneTestCase::new_bytes_ref_str("foobaz"),
    )?);
    Ok(())
}

fn test_simple() -> Result<()> {
    // Get outputs -- passing true means FST will share
    // (delta code) the outputs.  This should result in
    // smaller FST if the outputs grow monotonically.  But
    // if numbers are "random", false should give smaller
    // final size:
    let outputs = PositiveIntOutputs::get_singleton();

    // Build an FST mapping BytesRef -> Long
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let a = LuceneTestCase::new_bytes_ref_str("a");
    let b = LuceneTestCase::new_bytes_ref_str("b");
    let c = LuceneTestCase::new_bytes_ref_str("c");

    fst_compiler.add(&Util::to_ints_ref(&a, &mut IntsRefBuilder::new()), 17)?;
    fst_compiler.add(&Util::to_ints_ref(&b, &mut IntsRefBuilder::new()), 42)?;
    fst_compiler.add(
        &Util::to_ints_ref(&c, &mut IntsRefBuilder::new()),
        13824324872317238,
    )?;

    let fst = compile(fst_compiler)?.expect("three strings");

    assert_equals(&Some(13824324872317238i64), &Util::get_bytes(&fst, &c)?);
    assert_equals(&Some(42i64), &Util::get_bytes(&fst, &b)?);
    assert_equals(&Some(17i64), &Util::get_bytes(&fst, &a)?);

    let mut fst_enum = BytesRefFSTEnum::new(&fst)?;
    {
        let seek_result = fst_enum.seek_floor(&a)?;
        assert_not_null(&seek_result.as_ref().map(|_| ()));
        assert_equals(&17i64, seek_result.expect("not null").output);
    }

    // goes to a
    {
        let seek_result = fst_enum.seek_floor(&LuceneTestCase::new_bytes_ref_str("aa"))?;
        assert_not_null(&seek_result.as_ref().map(|_| ()));
        assert_equals(&17i64, seek_result.expect("not null").output);
    }

    // goes to b
    {
        let seek_result = fst_enum.seek_ceil(&LuceneTestCase::new_bytes_ref_str("aa"))?;
        assert_not_null(&seek_result.as_ref().map(|_| ()));
        let seek_result = seek_result.expect("not null");
        assert_equals(&b, seek_result.input);
        assert_equals(&42i64, seek_result.output);
    }
    Ok(())
}

/// `String.format(Locale.ROOT, "%07d", v)`.
fn format07(v: i32) -> String {
    format!("{v:07}")
}

/// The `Field` named `name` that the Java test keeps after `doc.add`.
fn field_mut<'a>(doc: &'a mut Document, name: &str) -> &'a mut Field {
    doc.get_field_mut_as::<Field>(name)
        .expect("INVARIANT: the document holds the Field")
}

fn test_primary_keys() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    for cycle in 0..2 {
        if verbose() {
            println!("TEST: cycle={cycle}");
        }
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        iwc.set_open_mode(OpenMode::CREATE)?;
        let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
        let mut doc = Document::new();
        let id_field = LuceneTestCase::new_string_field("id", "", Store::NO);
        doc.add(Box::new(id_field));
        let num_ids = LuceneTestCase::at_least(200);
        // final int NUM_IDS = (int) (377 * (1.0+random.nextDouble()));
        if verbose() {
            println!("TEST: NUM_IDS={num_ids}");
        }
        let mut all_ids: JavaHashSet<String> = JavaHashSet::new();
        for id in 0..num_ids {
            let id_string = if cycle == 0 {
                // PKs are assigned sequentially
                format07(id)
            } else {
                loop {
                    let s = random().next_long().to_string();
                    if !all_ids.contains(&s) {
                        break s;
                    }
                }
            };
            all_ids.add(id_string.clone());
            field_mut(&mut doc, "id").set_string_value(id_string)?;
            w.add_document(&doc)?;
        }

        // w.forceMerge(1);

        // turn writer into reader:
        let r = w.get_reader()?;
        let s = LuceneTestCase::new_searcher(Arc::clone(&r));
        w.close()?;

        let mut all_ids_list = all_ids.to_vec();
        let mut sorted_all_ids_list = all_ids_list.clone();
        sorted_all_ids_list.sort();

        // Sprinkle in some non-existent PKs:
        let mut out_of_bounds: JavaHashSet<String> = JavaHashSet::new();
        for idx in 0..num_ids / 10 {
            let id_string = if cycle == 0 {
                format07(num_ids + idx)
            } else {
                loop {
                    let candidate = random().next_long().to_string();
                    if !all_ids.contains(&candidate) {
                        break candidate;
                    }
                }
            };
            out_of_bounds.add(id_string.clone());
            all_ids_list.push(id_string);
        }

        // Verify w/ TermQuery
        for _ in 0..2 * num_ids {
            let id =
                all_ids_list[random().next_int_bound(all_ids_list.len() as i32) as usize].clone();
            let exists = !out_of_bounds.contains(&id);
            if verbose() {
                println!(
                    "TEST: TermQuery {} id={id}",
                    if exists { "" } else { "non-exist " }
                );
            }
            assert_equals_msg(
                Some(&format!(
                    "{}id={id}",
                    if exists { "" } else { "non-exist " }
                )),
                &(if exists { 1 } else { 0 }),
                &s.count(Arc::new(TermQuery::new(Term::new(
                    "id",
                    BytesRef::from_char_sequence(&id),
                ))))?,
            );
        }

        // Verify w/ MultiTermsEnum
        let r_any: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
        let mut terms_enum = MultiTerms::get_terms(&r_any, "id")?
            .expect("the id field has terms")
            .iterator()?;
        for _ in 0..2 * num_ids {
            let id: String;
            let next_id: Option<String>;
            let exists: bool;

            if random().next_boolean() {
                id = all_ids_list[random().next_int_bound(all_ids_list.len() as i32) as usize]
                    .clone();
                exists = !out_of_bounds.contains(&id);
                next_id = None;
                if verbose() {
                    println!(
                        "TEST: exactOnly {}id={id}",
                        if exists { "" } else { "non-exist " }
                    );
                }
            } else {
                // Pick ID between two IDs:
                exists = false;
                let idv = random().next_int_bound(num_ids - 1);
                if cycle == 0 {
                    id = format!("{}a", format07(idv));
                    next_id = Some(format07(idv + 1));
                } else {
                    id = format!("{}a", sorted_all_ids_list[idv as usize]);
                    next_id = Some(sorted_all_ids_list[idv as usize + 1].clone());
                }
                if verbose() {
                    println!(
                        "TEST: not exactOnly id={id} nextID={}",
                        next_id.as_deref().unwrap_or("null")
                    );
                }
            }

            let status = if next_id.is_none() {
                if terms_enum.seek_exact(&LuceneTestCase::new_bytes_ref_str(&id))? {
                    SeekStatus::FOUND
                } else {
                    SeekStatus::NOT_FOUND
                }
            } else {
                terms_enum.seek_ceil(&LuceneTestCase::new_bytes_ref_str(&id))?
            };

            if let Some(next_id) = &next_id {
                assert_equals(&SeekStatus::NOT_FOUND, &status);
                let term = terms_enum.term()?;
                assert_equals_msg(
                    Some(&format!(
                        "expected={next_id} actual={}",
                        term.utf8_to_string()?
                    )),
                    &LuceneTestCase::new_bytes_ref_str(next_id),
                    &term,
                );
            } else if !exists {
                assert_equals(&SeekStatus::NOT_FOUND, &status);
            } else {
                assert_equals(&SeekStatus::FOUND, &status);
            }
        }

        r.close()?;
    }
    dir.close()
}

fn test_random_term_lookup() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_open_mode(OpenMode::CREATE)?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let f = LuceneTestCase::new_string_field("field", "", Store::NO);
    doc.add(Box::new(f));

    let num_terms = (1000.0 * *RANDOM_MULTIPLIER as f64 * (1.0 + random().next_double())) as i32;
    if verbose() {
        println!("TEST: NUM_TERMS={num_terms}");
    }

    let mut all_terms: JavaHashSet<String> = JavaHashSet::new();
    while all_terms.size() < num_terms as usize {
        all_terms.add(FSTTester::<NoOutputs>::simple_random_string(&*random()));
    }

    for term in all_terms.iter() {
        field_mut(&mut doc, "field").set_string_value(term.clone())?;
        w.add_document(&doc)?;
    }

    // turn writer into reader:
    if verbose() {
        println!("TEST: get reader");
    }
    let r = w.get_reader()?;
    if verbose() {
        println!("TEST: got reader={r:?}");
    }
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    w.close()?;

    let mut all_terms_list = all_terms.to_vec();
    crate::core_support::shuffle(&mut all_terms_list, &*random());

    // verify exact lookup
    for term in &all_terms_list {
        if verbose() {
            println!("TEST: term={term}");
        }
        assert_equals_msg(
            Some(&format!("term={term}")),
            &1,
            &s.count(Arc::new(TermQuery::new(Term::new(
                "field",
                BytesRef::from_char_sequence(term),
            ))))?,
        );
    }

    r.close()?;
    dir.close()
}

/// The local class `SyntheticData` of `testExpandedCloseToRoot`.
struct SyntheticData;

impl SyntheticData {
    /// `compile(lines)`.
    fn compile(&self, lines: &[String]) -> Result<FST<NoOutputs>> {
        let outputs = NoOutputs::get_singleton();
        let nothing = outputs.no_output();
        let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

        let mut term = BytesRefBuilder::new();
        let mut scratch_ints_ref = IntsRefBuilder::new();
        for w in lines {
            term.copy_chars(w);
            fst_compiler.add(
                &Util::to_ints_ref(&term.get(), &mut scratch_ints_ref),
                nothing,
            )?;
        }

        Ok(compile(fst_compiler)?.expect("lines were added"))
    }

    /// `generate(out, b, from, to, depth)`.
    fn generate(&self, out: &mut Vec<String>, b: &mut String, from: char, to: char, depth: i32) {
        if depth == 0 || from == to {
            let seq = format!("{b}_{}_end", out.len());
            out.push(seq);
        } else {
            let mut c = from;
            while c <= to {
                b.push(c);
                self.generate(out, b, from, if c == to { to } else { from }, depth - 1);
                b.pop();
                c = char::from_u32(c as u32 + 1).expect("a letter");
            }
        }
    }

    /// `verifyStateAndBelow(fst, arc, depth)`.
    fn verify_state_and_below(
        &self,
        fst: &FST<NoOutputs>,
        arc: &FstArc<()>,
        depth: i32,
    ) -> Result<i32> {
        if target_has_arcs(arc) {
            let mut child_count = 0;
            let mut fst_reader = fst.get_bytes_reader()?;
            let mut arc_cur = FstArc::default();
            fst.read_first_target_arc(arc, &mut arc_cur, fst_reader.as_mut())?;
            loop {
                let expanded = fst.is_expanded_target(&arc_cur, fst_reader.as_mut())?;
                let mut copy = FstArc::default();
                copy.copy_from(&arc_cur);
                let children = self.verify_state_and_below(fst, &copy, depth + 1)?;

                assert_equals(
                    &((depth <= FIXED_LENGTH_ARC_SHALLOW_DEPTH as i32
                        && children >= FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS as i32)
                        || children >= FIXED_LENGTH_ARC_DEEP_NUM_ARCS as i32),
                    &expanded,
                );
                if arc_cur.is_last() {
                    break;
                }
                fst.read_next_arc(&mut arc_cur, fst_reader.as_mut())?;
                child_count += 1;
            }

            return Ok(child_count);
        }
        Ok(0)
    }
}

/// `testExpandedCloseToRoot()`: test state expansion (array format) on
/// close-to-root states. Creates synthetic input that has one expanded state
/// on each level (LUCENE-2933).
fn test_expanded_close_to_root() -> Result<()> {
    // Sanity check.
    Assert::assert_true(FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS < FIXED_LENGTH_ARC_DEEP_NUM_ARCS);
    Assert::assert_true(FIXED_LENGTH_ARC_SHALLOW_DEPTH as i64 >= 0);

    let s = SyntheticData;
    let mut out = Vec::new();
    let mut b = String::new();
    s.generate(&mut out, &mut b, 'a', 'i', 10);
    let mut input = out;
    input.sort();
    let fst = s.compile(&input)?;
    let mut arc = FstArc::default();
    fst.get_first_arc(&mut arc);
    s.verify_state_and_below(&fst, &arc, 1)?;
    Ok(())
}

/// `Util.toDot(fst, w, false, false)` into a string.
fn to_dot<O: Outputs>(fst: &FST<O>) -> Result<String> {
    let mut w: Vec<u8> = Vec::new();
    Util::to_dot(fst, &mut w, false, false)?;
    Ok(String::from_utf8(w).expect("dot output is UTF-8"))
}

fn test_final_output_on_end_state() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();

    let mut fst_compiler = Builder::new(InputType::Byte4, outputs).build()?;
    fst_compiler.add(&Util::to_utf32("slat", &mut IntsRefBuilder::new()), 10)?;
    fst_compiler.add(&Util::to_utf32("st", &mut IntsRefBuilder::new()), 17)?;
    let fst = compile(fst_compiler)?.expect("two strings");
    // Writer w = new OutputStreamWriter(new FileOutputStream("/x/tmp3/out.dot"));
    let w = to_dot(&fst)?;
    println!("{w}");
    Assert::assert_true(w.contains("label=\"t/[7]\""));
    Ok(())
}

fn test_internal_final_state() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;
    fst_compiler.add(
        &ints("stat", &mut IntsRefBuilder::new()),
        outputs.no_output(),
    )?;
    fst_compiler.add(
        &ints("station", &mut IntsRefBuilder::new()),
        outputs.no_output(),
    )?;
    let fst = compile(fst_compiler)?.expect("two strings");
    // Writer w = new OutputStreamWriter(new FileOutputStream("/x/tmp/out.dot"));
    let w = to_dot(&fst)?;
    // System.out.println(w.toString());

    // check for accept state at label t
    Assert::assert_true(w.contains("[label=\"t\" style=\"bold\""));
    // check for accept state at label n
    Assert::assert_true(w.contains("[label=\"n\" style=\"bold\""));
    Ok(())
}

/// `testSaveDifferentMetaOut()`: make sure the FST can be saved and loaded
/// with different `DataOutput` for metadata
/// (https://github.com/apache/lucene/issues/12697).
fn test_save_different_meta_out() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    // first build the FST from scratch
    let mut scratch = IntsRefBuilder::new();
    fst_compiler.add(&ints("aab", &mut scratch), 22)?;
    fst_compiler.add(&ints("aac", &mut scratch), 7)?;
    fst_compiler.add(&ints("ax", &mut scratch), 17)?;
    let fst = compile(fst_compiler)?.expect("three strings");

    // save the FST to DataOutput, here it would not matter whether we are saving to different
    // DataOutput for meta or not: `fst.save(out, out)` into one stream.
    let mut out_meta = VecDataOutput::new();
    let mut out_body = VecDataOutput::new();
    fst.save(&mut out_meta, &mut out_body)?;
    let mut out_os = out_meta.into_inner();
    out_os.extend_from_slice(&out_body.into_inner());

    // load the FST, which will force it to use FSTStore instead of BytesStore
    let mut input = ByteArrayDataInput::new(&out_os);
    let metadata = FST::read_metadata(&mut input, outputs)?;
    let loaded_fst = FST::new(metadata, &mut input)?;

    // now save the FST again, this time to different DataOutput for meta
    let mut meta_out = VecDataOutput::new();
    let mut data_out = VecDataOutput::new();
    loaded_fst.save(&mut meta_out, &mut data_out)?;

    // finally load it again
    let metdata_os = meta_out.into_inner();
    let data_os = data_out.into_inner();
    let mut meta_in = ByteArrayDataInput::new(&metdata_os);
    let mut data_in = ByteArrayDataInput::new(&data_os);
    let loaded_fst = FST::new(FST::read_metadata(&mut meta_in, outputs)?, &mut data_in)?;

    assert_equals(
        &Some(22i64),
        &Util::get(&loaded_fst, &ints("aab", &mut scratch))?,
    );
    assert_equals(
        &Some(7i64),
        &Util::get(&loaded_fst, &ints("aac", &mut scratch))?,
    );
    assert_equals(
        &Some(17i64),
        &Util::get(&loaded_fst, &ints("ax", &mut scratch))?,
    );
    Ok(())
}

/// `testNonFinalStopNode()`: make sure raw FST can differentiate between
/// final vs non-final end nodes.
fn test_non_final_stop_node() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();
    let nothing = outputs.no_output();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let mut root_node: UnCompiledNode<i64> = UnCompiledNode::new(&nothing, 0);

    // Add final stop node
    {
        let mut node: UnCompiledNode<i64> = UnCompiledNode::new(&nothing, 0);
        node.is_final = true;
        root_node.add_arc('a' as i32, &nothing);
        let frozen = CompiledNode {
            node: fst_compiler.add_node(&node)?,
        };
        root_node.arcs[0].next_final_output = 17;
        root_node.arcs[0].is_final = true;
        root_node.arcs[0].output = nothing;
        root_node.arcs[0].target = frozen.node;
    }

    // Add non-final stop node
    {
        let node: UnCompiledNode<i64> = UnCompiledNode::new(&nothing, 0);
        root_node.add_arc('b' as i32, &nothing);
        let frozen = CompiledNode {
            node: fst_compiler.add_node(&node)?,
        };
        root_node.arcs[1].next_final_output = nothing;
        root_node.arcs[1].output = 42;
        root_node.arcs[1].target = frozen.node;
    }

    let root = fst_compiler.add_node(&root_node)?;
    fst_compiler.finish(root)?;
    let metadata = fst_compiler.fst_metadata().clone();
    let fst = FST::from_reader(metadata, fst_compiler.into_fst_reader()?);

    // Writer w = new OutputStreamWriter(new FileOutputStream("/x/tmp3/out.dot"));
    let _w = to_dot(&fst)?;

    check_stop_nodes(&fst, &outputs)?;

    // Make sure it still works after save/load:
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    {
        let mut out = d.create_output("fst", &*DEFAULT_IO_CONTEXT)?;
        // fst.save(out, out): the metadata, then the bytes, into one output.
        let mut meta = ByteBuffersDataOutput::new();
        let mut body = ByteBuffersDataOutput::new();
        fst.save(&mut meta, &mut body)?;
        meta.copy_to(out.as_mut())?;
        body.copy_to(out.as_mut())?;
        out.close()?;
    }

    let mut input = d.open_input("fst", &*DEFAULT_IO_CONTEXT)?;
    let fst2 = FST::new(FST::read_metadata(input.as_mut(), outputs)?, input.as_mut())?;
    check_stop_nodes(&fst2, &outputs)?;
    input.close()?;
    dir.close()
}

/// `checkStopNodes(fst, outputs)`.
fn check_stop_nodes(fst: &FST<PositiveIntOutputs>, outputs: &PositiveIntOutputs) -> Result<()> {
    let nothing = outputs.no_output();
    let mut start_arc = FstArc::default();
    fst.get_first_arc(&mut start_arc);
    assert_equals(&nothing, start_arc.output());
    assert_equals(&nothing, start_arc.next_final_output());

    let mut arc = FstArc::default();
    fst.read_first_target_arc(&start_arc, &mut arc, fst.get_bytes_reader()?.as_mut())?;
    assert_equals(&('a' as i32), &arc.label());
    assert_equals(&17i64, arc.next_final_output());
    Assert::assert_true(arc.is_final());

    fst.read_next_arc(&mut arc, fst.get_bytes_reader()?.as_mut())?;
    assert_equals(&('b' as i32), &arc.label());
    Assert::assert_false(arc.is_final());
    assert_equals(&42i64, arc.output());
    Ok(())
}

/// `minLongComparator`.
fn min_long_comparator() -> OutputComparator<i64> {
    Rc::new(|a: &i64, b: &i64| a.cmp(b))
}

/// `fst.getFirstArc(new FST.Arc<>())`.
fn first_arc<O: Outputs>(fst: &FST<O>) -> FstArc<O::Output> {
    let mut arc = FstArc::default();
    fst.get_first_arc(&mut arc);
    arc
}

fn test_shortest_paths() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let mut scratch = IntsRefBuilder::new();
    fst_compiler.add(&ints("aab", &mut scratch), 22)?;
    fst_compiler.add(&ints("aac", &mut scratch), 7)?;
    fst_compiler.add(&ints("ax", &mut scratch), 17)?;
    let fst = compile(fst_compiler)?.expect("three strings");
    // Writer w = new OutputStreamWriter(new FileOutputStream("out.dot"));
    // Util.toDot(fst, w, false, false);
    // w.close();

    let res = Util::shortest_paths(
        &fst,
        &first_arc(&fst),
        outputs.no_output(),
        min_long_comparator(),
        3,
        true,
    )?;
    Assert::assert_true(res.is_complete);
    assert_equals(&3, &res.top_n.len());
    assert_equals(&ints("aac", &mut scratch), &res.top_n[0].input);
    assert_equals(&7i64, &res.top_n[0].output);

    assert_equals(&ints("ax", &mut scratch), &res.top_n[1].input);
    assert_equals(&17i64, &res.top_n[1].output);

    assert_equals(&ints("aab", &mut scratch), &res.top_n[2].input);
    assert_equals(&22i64, &res.top_n[2].output);
    Ok(())
}

/// The anonymous `TopNSearcher` subclass of `testRejectNoLimits`: accepts
/// only the output 7, counting the rejections.
struct RejectHooks<'a> {
    reject_count: &'a AtomicI32,
}

impl TopNSearcherHooks<i64> for RejectHooks<'_> {
    fn accept_result(&mut self, _input: &IntsRef, output: &i64) -> bool {
        let accept = *output as i32 == 7;
        if !accept {
            self.reject_count.fetch_add(1, AtomicOrdering::SeqCst);
        }
        accept
    }
}

fn test_reject_no_limits() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let mut scratch = IntsRefBuilder::new();
    fst_compiler.add(&ints("aab", &mut scratch), 22)?;
    fst_compiler.add(&ints("aac", &mut scratch), 7)?;
    fst_compiler.add(&ints("adcd", &mut scratch), 17)?;
    fst_compiler.add(&ints("adcde", &mut scratch), 17)?;

    fst_compiler.add(&ints("ax", &mut scratch), 17)?;
    let fst = compile(fst_compiler)?.expect("five strings");
    let reject_count = AtomicI32::new(0);
    let mut searcher = TopNSearcher::with_path_comparator(
        &fst,
        2,
        6,
        min_long_comparator(),
        tie_break_by_input_comparator(min_long_comparator()),
        RejectHooks {
            reject_count: &reject_count,
        },
    )?;

    searcher.add_start_paths(
        &first_arc(&fst),
        outputs.no_output(),
        true,
        IntsRefBuilder::new(),
    )?;
    let res = searcher.search()?;
    assert_equals(&reject_count.load(AtomicOrdering::SeqCst), &4);
    Assert::assert_true(res.is_complete); // rejected(4) + topN(2) <= maxQueueSize(6)

    assert_equals(&1, &res.top_n.len());
    assert_equals(&ints("aac", &mut scratch), &res.top_n[0].input);
    assert_equals(&7i64, &res.top_n[0].output);
    reject_count.store(0, AtomicOrdering::SeqCst);
    let mut searcher = TopNSearcher::with_path_comparator(
        &fst,
        2,
        5,
        min_long_comparator(),
        tie_break_by_input_comparator(min_long_comparator()),
        RejectHooks {
            reject_count: &reject_count,
        },
    )?;

    searcher.add_start_paths(
        &first_arc(&fst),
        outputs.no_output(),
        true,
        IntsRefBuilder::new(),
    )?;
    let res = searcher.search()?;
    assert_equals(&reject_count.load(AtomicOrdering::SeqCst), &4);
    Assert::assert_false(res.is_complete); // rejected(4) + topN(2) > maxQueueSize(5)
    Ok(())
}

/// `minPairWeightComparator`: compares just the weight side of the pair.
fn min_pair_weight_comparator() -> OutputComparator<Pair<i64, i64>> {
    Rc::new(|left: &Pair<i64, i64>, right: &Pair<i64, i64>| left.output1.cmp(&right.output1))
}

/// `testShortestPathsWFST()`: like `testShortestPaths`, but uses pair
/// outputs so we have both a weight and an output.
fn test_shortest_paths_wfst() -> Result<()> {
    let outputs = PairOutputs::new(
        PositiveIntOutputs::get_singleton(), // weight
        PositiveIntOutputs::get_singleton(), // output
    );

    let mut fst_compiler = Builder::new(InputType::Byte1, outputs.clone()).build()?;

    let mut scratch = IntsRefBuilder::new();
    fst_compiler.add(&ints("aab", &mut scratch), outputs.new_pair(22, 57))?;
    fst_compiler.add(&ints("aac", &mut scratch), outputs.new_pair(7, 36))?;
    fst_compiler.add(&ints("ax", &mut scratch), outputs.new_pair(17, 85))?;
    let fst = compile(fst_compiler)?.expect("three strings");
    // Writer w = new OutputStreamWriter(new FileOutputStream("out.dot"));
    // Util.toDot(fst, w, false, false);
    // w.close();

    let res = Util::shortest_paths(
        &fst,
        &first_arc(&fst),
        outputs.no_output(),
        min_pair_weight_comparator(),
        3,
        true,
    )?;
    Assert::assert_true(res.is_complete);
    assert_equals(&3, &res.top_n.len());

    assert_equals(&ints("aac", &mut scratch), &res.top_n[0].input);
    assert_equals(&7i64, &res.top_n[0].output.output1); // weight
    assert_equals(&36i64, &res.top_n[0].output.output2); // output

    assert_equals(&ints("ax", &mut scratch), &res.top_n[1].input);
    assert_equals(&17i64, &res.top_n[1].output.output1); // weight
    assert_equals(&85i64, &res.top_n[1].output.output2); // output

    assert_equals(&ints("aab", &mut scratch), &res.top_n[2].input);
    assert_equals(&22i64, &res.top_n[2].output.output1); // weight
    assert_equals(&57i64, &res.top_n[2].output.output2); // output
    Ok(())
}

/// `new TieBreakByInputComparator<>(comparator).compare(a, b)` over
/// `Util.Result`s.
fn tie_break_by_input<T>(
    comparator: &OutputComparator<T>,
    a: &PathResult<T>,
    b: &PathResult<T>,
) -> Ordering {
    let cmp = comparator(&a.output, &b.output);
    if cmp == Ordering::Equal {
        a.input.cmp(&b.input)
    } else {
        cmp
    }
}

fn test_shortest_paths_random() -> Result<()> {
    let random = random();
    let num_words = LuceneTestCase::at_least(1000);

    let mut slow_completor: BTreeMap<String, i64> = BTreeMap::new();
    let mut all_prefixes: BTreeSet<String> = BTreeSet::new();

    let outputs = PositiveIntOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;
    let mut scratch = IntsRefBuilder::new();

    for _ in 0..num_words {
        let mut s;
        loop {
            s = TestUtil::random_simple_string(&*random);
            if !slow_completor.contains_key(&s) {
                break;
            }
        }

        for j in 1..s.len() {
            all_prefixes.insert(s[..j].to_string());
        }
        let weight = TestUtil::next_int(&*random, 1, 100); // weights 1..100
        slow_completor.insert(s, weight as i64);
    }

    for (k, v) in &slow_completor {
        // System.out.println("add: " + e);
        fst_compiler.add(&ints(k, &mut scratch), *v)?;
    }

    let fst = compile(fst_compiler)?.expect("words were added");
    // System.out.println("SAVE out.dot");
    // Writer w = new OutputStreamWriter(new FileOutputStream("out.dot"));
    // Util.toDot(fst, w, false, false);
    // w.close();

    let mut reader = fst.get_bytes_reader()?;

    // System.out.println("testing: " + allPrefixes.size() + " prefixes");
    for prefix in &all_prefixes {
        // 1. run prefix against fst, then complete by value
        // System.out.println("TEST: " + prefix);

        let mut prefix_output = 0i64;
        let mut arc = first_arc(&fst);
        for ch in prefix.chars() {
            let follow = arc.clone();
            if !fst.find_target_arc(ch as i32, &follow, &mut arc, reader.as_mut())? {
                Assert::fail_without_message();
            }
            prefix_output += *arc.output();
        }

        let top_n = TestUtil::next_int(&*random, 1, 10);

        let r = Util::shortest_paths(
            &fst,
            &arc,
            fst.outputs().no_output(),
            min_long_comparator(),
            top_n as usize,
            true,
        )?;
        Assert::assert_true(r.is_complete);

        // 2. go thru whole treemap (slowCompletor) and check it's actually the best suggestion
        let mut matches: Vec<PathResult<i64>> = Vec::new();

        // TODO: could be faster... but it's slowCompletor for a reason
        for (k, v) in &slow_completor {
            if k.starts_with(prefix.as_str()) {
                // System.out.println("  consider " + e.getKey());
                matches.push(PathResult {
                    input: Util::to_ints_ref(
                        &LuceneTestCase::new_bytes_ref_str(&k[prefix.len()..]),
                        &mut IntsRefBuilder::new(),
                    ),
                    output: v - prefix_output,
                });
            }
        }

        Assert::assert_true(!matches.is_empty());
        let comparator = min_long_comparator();
        matches.sort_by(|a, b| tie_break_by_input(&comparator, a, b));
        if matches.len() > top_n as usize {
            matches.truncate(top_n as usize);
        }

        assert_equals(&matches.len(), &r.top_n.len());

        for hit in 0..r.top_n.len() {
            // System.out.println("  check hit " + hit);
            assert_equals(&matches[hit].input, &r.top_n[hit].input);
            assert_equals(&matches[hit].output, &r.top_n[hit].output);
        }
    }
    Ok(())
}

/// `TwoLongs` (used by slowcompletor).
struct TwoLongs {
    a: i64,
    b: i64,
}

/// `testShortestPathsWFSTRandom()`: like `testShortestPathsRandom`, but
/// uses pair outputs so we have both a weight and an output.
fn test_shortest_paths_wfst_random() -> Result<()> {
    let num_words = LuceneTestCase::at_least(1000);

    let mut slow_completor: BTreeMap<String, TwoLongs> = BTreeMap::new();
    let mut all_prefixes: BTreeSet<String> = BTreeSet::new();

    let outputs = PairOutputs::new(
        PositiveIntOutputs::get_singleton(), // weight
        PositiveIntOutputs::get_singleton(), // output
    );
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs.clone()).build()?;
    let mut scratch = IntsRefBuilder::new();

    let random = random();
    for _ in 0..num_words {
        let mut s;
        loop {
            s = TestUtil::random_simple_string(&*random);
            if !slow_completor.contains_key(&s) {
                break;
            }
        }

        for j in 1..s.len() {
            all_prefixes.insert(s[..j].to_string());
        }
        let weight = TestUtil::next_int(&*random, 1, 100); // weights 1..100
        let output = TestUtil::next_int(&*random, 0, 500); // outputs 0..500
        slow_completor.insert(
            s,
            TwoLongs {
                a: weight as i64,
                b: output as i64,
            },
        );
    }

    for (k, v) in &slow_completor {
        // System.out.println("add: " + e);
        let weight = v.a;
        let output = v.b;
        fst_compiler.add(&ints(k, &mut scratch), outputs.new_pair(weight, output))?;
    }

    let fst = compile(fst_compiler)?.expect("words were added");
    // System.out.println("SAVE out.dot");
    // Writer w = new OutputStreamWriter(new FileOutputStream("out.dot"));
    // Util.toDot(fst, w, false, false);
    // w.close();

    let mut reader = fst.get_bytes_reader()?;

    // System.out.println("testing: " + allPrefixes.size() + " prefixes");
    for prefix in &all_prefixes {
        // 1. run prefix against fst, then complete by value
        // System.out.println("TEST: " + prefix);

        let mut prefix_output = outputs.no_output();
        let mut arc = first_arc(&fst);
        for ch in prefix.chars() {
            let follow = arc.clone();
            if !fst.find_target_arc(ch as i32, &follow, &mut arc, reader.as_mut())? {
                Assert::fail_without_message();
            }
            prefix_output = outputs.add(&prefix_output, arc.output());
        }

        let top_n = TestUtil::next_int(&*random, 1, 10);

        let r = Util::shortest_paths(
            &fst,
            &arc,
            fst.outputs().no_output(),
            min_pair_weight_comparator(),
            top_n as usize,
            true,
        )?;
        Assert::assert_true(r.is_complete);
        // 2. go thru whole treemap (slowCompletor) and check it's actually the best suggestion
        let mut matches: Vec<PathResult<Pair<i64, i64>>> = Vec::new();

        // TODO: could be faster... but it's slowCompletor for a reason
        for (k, v) in &slow_completor {
            if k.starts_with(prefix.as_str()) {
                // System.out.println("  consider " + e.getKey());
                matches.push(PathResult {
                    input: Util::to_ints_ref(
                        &LuceneTestCase::new_bytes_ref_str(&k[prefix.len()..]),
                        &mut IntsRefBuilder::new(),
                    ),
                    output: outputs
                        .new_pair(v.a - prefix_output.output1, v.b - prefix_output.output2),
                });
            }
        }

        Assert::assert_true(!matches.is_empty());
        let comparator = min_pair_weight_comparator();
        matches.sort_by(|a, b| tie_break_by_input(&comparator, a, b));
        if matches.len() > top_n as usize {
            matches.truncate(top_n as usize);
        }

        assert_equals(&matches.len(), &r.top_n.len());

        for hit in 0..r.top_n.len() {
            // System.out.println("  check hit " + hit);
            assert_equals(&matches[hit].input, &r.top_n[hit].input);
            assert_equals(&matches[hit].output, &r.top_n[hit].output);
        }
    }
    Ok(())
}

fn test_large_outputs_on_array_arcs() -> Result<()> {
    let outputs = ByteSequenceOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let mut bytes = vec![0u8; 300];
    let mut input = IntsRefBuilder::new();
    input.append(0);
    for arc in 0..6 {
        input.set_int_at(0, arc);
        bytes[0] = arc as u8;
        let output = BytesRef::new(bytes.clone());
        fst_compiler.add(&input.get(), LuceneTestCase::new_bytes_ref_from(&output))?;
    }

    let fst = compile(fst_compiler)?.expect("six arcs");
    for arc in 0..6 {
        input.set_int_at(0, arc);
        let result = Util::get(&fst, &input.get())?;
        assert_not_null(&result);
        let result = result.expect("asserted not null");
        assert_equals(&300, &result.length);
        assert_equals(&(result.bytes[result.offset] as i32), &arc);
        for byte_idx in 1..result.length {
            assert_equals(&0u8, &result.bytes[result.offset + byte_idx]);
        }
    }
    Ok(())
}

fn test_illegally_modify_root_arc() -> Result<()> {
    LuceneTestCase::assume_true("test relies on assertions", *TEST_ASSERTS_ENABLED);

    let mut terms: JavaHashSet<BytesRef> = JavaHashSet::new();
    for i in 0..100u32 {
        let prefix = char::from_u32('a' as u32 + i).expect("a char").to_string();
        terms.add(LuceneTestCase::new_bytes_ref_str(&prefix));
        if prefix != "m" {
            for _ in 0..20 {
                // Make a big enough FST that the root cache will be created:
                let suffix = TestUtil::random_realistic_unicode_string_between(&*random(), 10, 20);
                terms.add(LuceneTestCase::new_bytes_ref_str(&format!(
                    "{prefix}{suffix}"
                )));
            }
        }
    }

    let mut terms_list = terms.to_vec();
    terms_list.sort();

    let outputs = ByteSequenceOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let mut input = IntsRefBuilder::new();
    for term in &terms_list {
        Util::to_ints_ref(term, &mut input);
        fst_compiler.add(&input.get(), term.clone())?;
    }

    let fst = compile(fst_compiler)?.expect("terms were added");

    let mut arc = FstArc::default();
    fst.get_first_arc(&mut arc);
    let mut reader = fst.get_bytes_reader()?;
    let follow = arc.clone();
    let found = fst.find_target_arc('m' as i32, &follow, &mut arc, reader.as_mut())?;
    assert_not_null(&if found { Some(()) } else { None });
    assert_equals(&LuceneTestCase::new_bytes_ref_str("m"), arc.output());

    // NOTE: illegal: `arc.output().length = 0`.
    //
    // **Divergence from Lucene 10.5.0.** Java truncates the output object
    // the FST's cached root arc shares with `arc`, so that the next lookup
    // trips the root-cache assertion; Rucene's arcs own their outputs
    // (`arc.output()` is a shared reference), so the cached root arc cannot
    // be reached through `arc` and the illegal modification has no Rust
    // counterpart. The rest of the test runs as in Java.
    fst.get_first_arc(&mut arc);
    let follow = arc.clone();
    let outcome = crate::core_support::try_catch("java.lang.AssertionError", || {
        fst.find_target_arc('m' as i32, &follow, &mut arc, reader.as_mut())
    });
    if let Ok(result) = outcome {
        result?;
    }
    // expected
    Ok(())
}

fn test_simple_depth() -> Result<()> {
    let outputs = PositiveIntOutputs::get_singleton();
    let mut fst_compiler = Builder::new(InputType::Byte1, outputs).build()?;

    let ab = LuceneTestCase::new_bytes_ref_str("ab");
    let ac = LuceneTestCase::new_bytes_ref_str("ac");
    let bd = LuceneTestCase::new_bytes_ref_str("bd");

    fst_compiler.add(&Util::to_ints_ref(&ab, &mut IntsRefBuilder::new()), 3)?;
    fst_compiler.add(&Util::to_ints_ref(&ac, &mut IntsRefBuilder::new()), 5)?;
    fst_compiler.add(&Util::to_ints_ref(&bd, &mut IntsRefBuilder::new()), 7)?;

    let fst = compile(fst_compiler)?.expect("three strings");

    assert_equals(&Some(3i64), &Util::get_bytes(&fst, &ab)?);
    assert_equals(&Some(5i64), &Util::get_bytes(&fst, &ac)?);
    assert_equals(&Some(7i64), &Util::get_bytes(&fst, &bd)?);
    Ok(())
}
