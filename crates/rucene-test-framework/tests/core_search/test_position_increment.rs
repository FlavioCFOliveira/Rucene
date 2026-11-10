//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPositionIncrement.java`.
//!
//! Term position unit test.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStreamComponents, Tokenizer,
    TokenizerLogic,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::IndexReader;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::Query;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::analysis::tokenizer_components;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::term;
use crate::core_support::{assert_equals, class, random, test};

/// `TestPositionIncrement`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestPositionIncrement",
        vec![test("testSetPosition", test_set_position)],
    )
}

/// `static final boolean VERBOSE = false;`
#[allow(dead_code)]
const VERBOSE: bool = false;

// TODO: use CannedTokenStream
const TOKENS: [&str; 5] = ["1", "2", "3", "4", "5"];
const INCREMENTS: [i32; 5] = [1, 2, 1, 0, 1];

/// The anonymous `Tokenizer` of `testSetPosition`'s analyzer.
#[derive(Debug)]
struct FixedTokenizer {
    i: usize,
    pos_incr_att: Att,
    term_att: Att,
    offset_att: Att,
}

impl TokenizerLogic for FixedTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.i == TOKENS.len() {
            return Ok(false);
        }
        source.clear_attributes();
        self.term_att
            .char_term_mut()
            .append_string(Some(TOKENS[self.i]));
        self.offset_att
            .offset_mut()
            .set_offset(self.i as i32, self.i as i32);
        self.pos_incr_att
            .pos_inc_mut()
            .set_position_increment(INCREMENTS[self.i]);
        self.i += 1;
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.i = 0;
        Ok(())
    }
}

/// The anonymous `Analyzer` of `testSetPosition`.
#[derive(Debug)]
struct FixedAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for FixedAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.TestPositionIncrement$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let pos_incr_att = Att::add(&mut source, pos_inc_id())?;
        let term_att = Att::add(&mut source, char_term_id())?;
        let offset_att = Att::add(&mut source, offset_id())?;
        let t = Tokenizer::with_source(
            source,
            FixedTokenizer {
                i: 0,
                pos_incr_att,
                term_att,
                offset_att,
            },
        );
        Ok(tokenizer_components(Rc::new(RefCell::new(t))))
    }
}

/// `searcher.search(q, 1000).scoreDocs.length`.
fn hits(
    searcher: &rucene::search::index_searcher::IndexSearcher,
    q: Arc<dyn Query>,
) -> Result<usize> {
    Ok(searcher.search(q, 1000)?.score_docs.len())
}

/// Port of `TestPositionIncrement.testSetPosition`.
fn test_set_position() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(FixedAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let mut store = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new(&*random(), store.as_directory(), analyzer)?;
    let mut d = Document::new();
    d.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "bogus",
        Store::YES,
    )));
    writer.add_document(&d)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let ir: Arc<dyn IndexReader> = Arc::clone(searcher.get_index_reader());
    let mut pos = MultiTerms::get_term_postings_enum(
        &ir,
        "field",
        &BytesRef::from_char_sequence("1"),
        POSTINGS_ENUM_ALL,
    )?
    .expect("the term exists");
    pos.next_doc()?;
    // first token should be at position 0
    assert_equals(&0, &pos.next_position()?);

    let mut pos = MultiTerms::get_term_postings_enum(
        &ir,
        "field",
        &BytesRef::from_char_sequence("2"),
        POSTINGS_ENUM_ALL,
    )?
    .expect("the term exists");
    pos.next_doc()?;
    // second token should be at position 2
    assert_equals(&2, &pos.next_position()?);

    let phrase = |a: &str, b: &str| -> Result<Arc<dyn Query>> {
        Ok(Arc::new(PhraseQuery::from_strings("field", &[a, b])?))
    };
    let at = |terms: &[(&str, i32)]| -> Result<Arc<dyn Query>> {
        let mut builder = PhraseQueryBuilder::new();
        for (t, p) in terms {
            builder.add_at(term("field", t), *p)?;
        }
        Ok(Arc::new(builder.build()?))
    };

    let q = phrase("1", "2")?;
    assert_equals(&0, &hits(&searcher, q)?);

    // same as previous, using the builder with implicit positions
    let mut builder = PhraseQueryBuilder::new();
    builder.add(term("field", "1"))?;
    builder.add(term("field", "2"))?;
    let q: Arc<dyn Query> = Arc::new(builder.build()?);
    assert_equals(&0, &hits(&searcher, q)?);

    // same as previous, just specify positions explicitely.
    assert_equals(&0, &hits(&searcher, at(&[("1", 0), ("2", 1)])?)?);

    // specifying correct positions should find the phrase.
    assert_equals(&1, &hits(&searcher, at(&[("1", 0), ("2", 2)])?)?);

    assert_equals(&1, &hits(&searcher, phrase("2", "3")?)?);

    assert_equals(&0, &hits(&searcher, phrase("3", "4")?)?);

    // phrase query would find it when correct positions are specified.
    assert_equals(&1, &hits(&searcher, at(&[("3", 0), ("4", 0)])?)?);

    // phrase query should fail for non existing searched term
    // even if there exist another searched terms in the same searched position.
    assert_equals(&0, &hits(&searcher, at(&[("3", 0), ("9", 0)])?)?);

    // multi-phrase query should succed for non existing searched term
    // because there exist another searched terms in the same searched position.
    let mut mqb = MultiPhraseQueryBuilder::new();
    mqb.add_at(vec![term("field", "3"), term("field", "9")], 0)?;
    assert_equals(&1, &hits(&searcher, Arc::new(mqb.build()))?);

    assert_equals(&1, &hits(&searcher, phrase("2", "4")?)?);

    assert_equals(&1, &hits(&searcher, phrase("3", "5")?)?);

    assert_equals(&1, &hits(&searcher, phrase("4", "5")?)?);

    assert_equals(&0, &hits(&searcher, phrase("2", "5")?)?);

    drop(searcher);
    reader.close()?;
    store.close()?;
    Ok(())
}
