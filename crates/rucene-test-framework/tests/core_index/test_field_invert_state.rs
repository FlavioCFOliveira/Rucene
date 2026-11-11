//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFieldInvertState.java`.
//!
//! * `NeverForgetsSimilarity.lastState` holds a copy of the state
//!   `computeNorm` received, taken when it is called: Java keeps the
//!   reference, which the indexing chain does not change after
//!   `computeNorm` for the single document these tests index.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::TokenStream;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::FieldInvertState;
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene_test_framework::analysis::{CannedTokenStream, MockAnalyzer, Token};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};

/// `TestFieldInvertState`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFieldInvertState",
        vec![
            test("testBasic", test_basic),
            test("testRandom", test_random),
        ],
    )
}

/// Similarity holds onto the FieldInvertState for subsequent verification.
#[derive(Debug, Default)]
struct NeverForgetsSimilarity {
    last_state: Mutex<Option<FieldInvertState>>,
}

static INSTANCE: LazyLock<Arc<NeverForgetsSimilarity>> =
    LazyLock::new(|| Arc::new(NeverForgetsSimilarity::default()));

impl NeverForgetsSimilarity {
    /// `lastState`.
    fn last_state(&self) -> FieldInvertState {
        self.last_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| crate::support::npe("lastState"))
    }
}

impl Similarity for NeverForgetsSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestFieldInvertState$NeverForgetsSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        *self.last_state.lock().unwrap_or_else(|e| e.into_inner()) = Some(state.clone());
        Ok(1)
    }

    fn scorer<'a>(
        &'a self,
        _boost: f32,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }
}

/// `new Field(name, new CannedTokenStream(tokens), TextField.TYPE_NOT_STORED)`.
fn canned_field(name: &str, tokens: Vec<Token>) -> Result<Field> {
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(CannedTokenStream::new(tokens)));
    Field::new_with_token_stream(
        name,
        stream,
        FieldType::new_from(TextField::type_not_stored()),
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))` with
/// `setSimilarity(NeverForgetsSimilarity.INSTANCE)`.
fn config() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_similarity(Some(Arc::clone(&*INSTANCE) as Arc<dyn Similarity>))?;
    Ok(iwc)
}

/// Port of `TestFieldInvertState.testBasic`.
fn test_basic() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let field = canned_field(
        "field",
        vec![
            Token::with_offsets("a", 0, 1),
            Token::with_offsets("b", 2, 3),
            Token::with_offsets("c", 4, 5),
        ],
    )?;
    doc.add(Box::new(field));
    w.add_document(&doc)?;
    let fis = INSTANCE.last_state();
    assert_equals(&1, &fis.max_term_frequency());
    assert_equals(&3, &fis.unique_term_count());
    assert_equals(&0, &fis.num_overlap());
    assert_equals(&3, &fis.length());
    w.close()?;
    dir.close()
}

/// Port of `TestFieldInvertState.testRandom`.
fn test_random() -> Result<()> {
    let num_unique_tokens = TestUtil::next_int(&*random(), 1, 25);
    let dir = LuceneTestCase::new_directory();
    let iwc = config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();

    let num_tokens = LuceneTestCase::at_least(10000);
    let mut tokens: Vec<Token> = Vec::with_capacity(num_tokens as usize);
    let mut counts: HashMap<char, i32> = HashMap::new();
    let mut num_stacked = 0;
    let mut max_term_freq = 0;
    let mut pos = -1;
    for i in 0..num_tokens {
        let token_char = (b'a' + random().next_int_bound(num_unique_tokens) as u8) as char;
        let new_count = match counts.get(&token_char) {
            None => 1,
            Some(old_count) => 1 + old_count,
        };
        counts.insert(token_char, new_count);
        max_term_freq = max_term_freq.max(new_count);

        // `new Token(...)` then `token.setPositionIncrement(0)`: the token is
        // built with its final increment.
        let pos_inc = if i > 0 && random().next_int_bound(7) == 3 {
            num_stacked += 1;
            0
        } else {
            pos += 1;
            1
        };
        let token = Token::with_pos_inc(&token_char.to_string(), pos_inc, 2 * i, 2 * i + 1);
        tokens.push(token);
    }

    let field = canned_field("field", tokens)?;
    doc.add(Box::new(field));
    w.add_document(&doc)?;
    let fis = INSTANCE.last_state();
    assert_equals(&max_term_freq, &fis.max_term_frequency());
    assert_equals(&(counts.len() as i32), &fis.unique_term_count());
    assert_equals(&num_stacked, &fis.num_overlap());
    assert_equals(&num_tokens, &fis.length());
    assert_equals(&pos, &fis.position());

    w.close()?;
    dir.close()
}
