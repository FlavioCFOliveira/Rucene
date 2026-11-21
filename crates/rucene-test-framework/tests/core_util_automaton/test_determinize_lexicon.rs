//! Port of `lucene/core/src/test/org/apache/lucene/util/automaton/TestDeterminizeLexicon.java`:
//! not thorough, but tries to test determinism correctness somewhat
//! randomly, by determinizing a huge random lexicon.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::automaton::{Automata, Automaton, ByteRunAutomaton, ByteRunnable, Operations};
use rucene_test_framework::util::automaton::automaton_test_util::AutomatonTestUtil;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, det, random, shuffle, test};

/// `TestDeterminizeLexicon`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![test("testLexicon", test_lexicon)];
    class(
        "org.apache.lucene.util.automaton.TestDeterminizeLexicon",
        methods,
    )
}

/// The instance fields `automata` and `terms`.
struct Fields {
    automata: Vec<Automaton>,
    terms: Vec<String>,
}

fn test_lexicon() -> Result<()> {
    let mut this = Fields {
        automata: Vec::new(),
        terms: Vec::new(),
    };
    let num = LuceneTestCase::at_least(1);
    for _ in 0..num {
        this.automata.clear();
        this.terms.clear();
        for _ in 0..5000 {
            let random_string = TestUtil::random_unicode_string(&*random());
            this.terms.push(random_string.clone());
            this.automata.push(Automata::make_string(&random_string));
        }
        assert_lexicon(&mut this)?;
    }
    Ok(())
}

/// `assertLexicon()`.
fn assert_lexicon(this: &mut Fields) -> Result<()> {
    shuffle(&mut this.automata, &*random());
    let lex = Operations::union(&this.automata);
    let lex = det(Operations::determinize(&lex, 1000000));
    Assert::assert_true(AutomatonTestUtil::is_finite(&lex));
    for s in &this.terms {
        Assert::assert_true(Operations::run(&lex, s));
    }
    if *TEST_NIGHTLY {
        // TODO: very wasteful of RAM to do this without minimizing first.
        let lex_byte = ByteRunAutomaton::new(lex, false)?;
        for s in &this.terms {
            let bytes = s.as_bytes();
            Assert::assert_true(lex_byte.run_range(bytes, 0, bytes.len()));
        }
    }
    Ok(())
}
