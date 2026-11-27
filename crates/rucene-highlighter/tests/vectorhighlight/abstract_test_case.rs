/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `AbstractTestCase.java`, the abstract base of the
//! `vectorhighlight` tests: its instance fields are [`Fixture`], its
//! `setUp`/`tearDown` are [`set_up`]/[`tear_down`] (each subclass registers
//! them), and its protected helpers are the free functions of this module.

// The helpers of the abstract base each subclass uses a subset of.
#![allow(dead_code)]

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
    Tokenizer, TokenizerLogic,
};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::{IndexReader, Term};
use rucene::search::boost_query::BoostQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::{IndexSearcher, TermQuery};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{KEYWORD, WHITESPACE};
use rucene_test_framework::analysis::{tokenizer_components, MockAnalyzer};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::{fixture_class, set_up as su, tear_down as td};

pub const F: &str = "f";
pub const F1: &str = "f1";
pub const F2: &str = "f2";

/// empty data in multi valued field
pub const SHORT_MV_VALUES: [&str; 5] = ["", "", "a b c", "", "d e"];

pub const LONG_MV_VALUES: [&str; 2] = [
    "Followings are the examples of customizable parameters and actual examples of customization:",
    "The most search engines use only one of these methods. Even the search engines that says they can use the both methods basically",
];

/// test data for LUCENE-1448 bug
pub const BI_MV_VALUES: [&str; 2] = [
    "\nLucene/Solr does not require such additional hardware.",
    "\nWhen you talk about processing speed, the",
];

pub const STR_MV_VALUES: [&str; 3] = ["abc", "defg", "hijkl"];

/// The instance fields of `AbstractTestCase`.
#[derive(Default)]
pub struct Fixture {
    pub dir: Option<TestDirectory>,
    pub analyzer_w: Option<Arc<dyn Analyzer>>,
    pub analyzer_b: Option<Arc<dyn Analyzer>>,
    pub analyzer_k: Option<Arc<dyn Analyzer>>,
    pub reader: Option<Arc<dyn IndexReader>>,
    pub searcher: Option<IndexSearcher>,
}

impl Fixture {
    /// `reader`.
    pub fn reader(&self) -> &dyn IndexReader {
        &**self.reader.as_ref().expect("an index was made")
    }

    /// `dir`.
    pub fn dir(&self) -> Arc<dyn rucene::store::Directory> {
        self.dir.as_ref().expect("setUp ran").as_directory()
    }
}

/// `public abstract class AbstractTestCase extends LuceneTestCase`'s
/// subclass `name`, with `setUp`, `tearDown` and `methods`.
pub fn vh_class(name: &str, mut methods: Vec<Method>) -> Arc<Class> {
    let mut all = vec![su(set_up), td(tear_down)];
    all.append(&mut methods);
    fixture_class::<Fixture>(name, all)
}

fn set_up(f: &mut Fixture) -> Result<()> {
    f.analyzer_w = Some(Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    )));
    f.analyzer_b = Some(Arc::new(BigramAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    }));
    f.analyzer_k = Some(Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&KEYWORD),
        false,
    )));
    f.dir = Some(LuceneTestCase::new_directory());
    Ok(())
}

fn tear_down(f: &mut Fixture) -> Result<()> {
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    f.searcher = None;
    f.dir.take().expect("setUp ran").close()?;
    Ok(())
}

/// `tq(text)`.
pub fn tq(text: &str) -> Arc<dyn Query> {
    tq_bft(1.0, F, text)
}

/// `tq(boost, text)`.
pub fn tq_b(boost: f32, text: &str) -> Arc<dyn Query> {
    tq_bft(boost, F, text)
}

/// `tq(field, text)`.
pub fn tq_f(field: &str, text: &str) -> Arc<dyn Query> {
    tq_bft(1.0, field, text)
}

/// `tq(boost, field, text)`.
pub fn tq_bft(boost: f32, field: &str, text: &str) -> Arc<dyn Query> {
    let mut query: Arc<dyn Query> =
        Arc::new(TermQuery::new(Term::from_text(field.to_string(), text)));
    if boost != 1.0 {
        query = Arc::new(BoostQuery::new(query, boost).expect("a valid boost"));
    }
    query
}

/// `pqF(texts...)`.
pub fn pq_f(texts: &[&str]) -> Arc<dyn Query> {
    pq_full(1.0, 0, F, texts)
}

/// `pqF(boost, texts...)`.
pub fn pq_f_b(boost: f32, texts: &[&str]) -> Arc<dyn Query> {
    pq_full(boost, 0, F, texts)
}

/// `pqF(boost, slop, texts...)`.
pub fn pq_f_bs(boost: f32, slop: i32, texts: &[&str]) -> Arc<dyn Query> {
    pq_full(boost, slop, F, texts)
}

/// `pq(field, texts...)`.
pub fn pq(field: &str, texts: &[&str]) -> Arc<dyn Query> {
    pq_full(1.0, 0, field, texts)
}

/// `pq(boost, field, texts...)`.
pub fn pq_b(boost: f32, field: &str, texts: &[&str]) -> Arc<dyn Query> {
    pq_full(boost, 0, field, texts)
}

/// `pq(boost, slop, field, texts...)`.
pub fn pq_full(boost: f32, slop: i32, field: &str, texts: &[&str]) -> Arc<dyn Query> {
    let mut query: Arc<dyn Query> =
        Arc::new(PhraseQuery::with_slop(slop, field, texts).expect("a valid phrase"));
    if boost != 1.0 {
        query = Arc::new(BoostQuery::new(query, boost).expect("a valid boost"));
    }
    query
}

/// `dmq(queries...)`.
pub fn dmq(queries: Vec<Arc<dyn Query>>) -> Arc<dyn Query> {
    dmq_t(0.0, queries)
}

/// `dmq(tieBreakerMultiplier, queries...)`.
pub fn dmq_t(tie_breaker_multiplier: f32, queries: Vec<Arc<dyn Query>>) -> Arc<dyn Query> {
    Arc::new(DisjunctionMaxQuery::new(queries, tie_breaker_multiplier).expect("a valid dmq"))
}

/// `assertCollectionQueries(actual, expected...)`.
pub fn assert_collection_queries(actual: &[Arc<dyn Query>], expected: &[Arc<dyn Query>]) {
    Assert::assert_equals(expected.len(), actual.len());
    for query in expected {
        Assert::assert_true(actual.iter().any(|q| q.query_eq(&**query)));
    }
}

/// `analyze(text, field, analyzer)`.
pub fn analyze(text: &str, field: &str, analyzer: &dyn Analyzer) -> Result<Vec<BytesRef>> {
    let mut bytes_refs = Vec::new();
    let stream = analyzer.token_stream_from_str(field, text)?;
    let mut token_stream = stream.borrow_mut();
    let term_attribute = Att::get(
        token_stream.attribute_source(),
        rucene_test_framework::analysis::attributes::term_to_bytes_ref_id(),
    )
    .expect("a TermToBytesRefAttribute");
    token_stream.reset()?;
    while token_stream.increment_token()? {
        bytes_refs.push(BytesRef::deep_copy_of(
            &term_attribute.term_to_bytes_ref().get_bytes_ref(),
        ));
    }
    token_stream.end()?;
    token_stream.close()?;
    Ok(bytes_refs)
}

/// `toPhraseQuery(bytesRefs, field)`.
pub fn to_phrase_query(bytes_refs: &[BytesRef], field: &str) -> PhraseQuery {
    PhraseQuery::from_bytes(field, bytes_refs).expect("a valid phrase")
}

/// `static final class BigramAnalyzer extends Analyzer`.
#[derive(Debug)]
pub struct BigramAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for BigramAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.vectorhighlight.AbstractTestCase$BigramAnalyzer"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            basic_ngram_tokenizer(),
        ))))
    }
}

/// `static final class BasicNGramTokenizer extends Tokenizer`.
pub type BasicNGramTokenizer = Tokenizer<BasicNGramLogic>;

const DEFAULT_N_SIZE: usize = 2;
const DEFAULT_DELIMITERS: &str = " \t\n.,";
const BUFFER_SIZE: usize = 4096;

/// The body of `BasicNGramTokenizer`.
#[derive(Debug)]
pub struct BasicNGramLogic {
    n: usize,
    delimiters: Vec<u16>,
    start_term: usize,
    len_term: usize,
    start_offset: i32,
    next_start_offset: i32,
    ch: i32,
    snippet: Option<Vec<u16>>,
    snippet_buffer: Vec<u16>,
    char_buffer: Vec<u16>,
    char_buffer_index: usize,
    char_buffer_len: i32,
}

impl BasicNGramLogic {
    fn new() -> Self {
        Self {
            n: DEFAULT_N_SIZE,
            delimiters: DEFAULT_DELIMITERS.encode_utf16().collect(),
            start_term: 0,
            len_term: 0,
            start_offset: 0,
            next_start_offset: 0,
            ch: 0,
            snippet: None,
            snippet_buffer: Vec::new(),
            char_buffer: vec![0; BUFFER_SIZE],
            char_buffer_index: BUFFER_SIZE,
            char_buffer_len: 0,
        }
    }

    fn get_final_offset(&self) -> i32 {
        self.next_start_offset
    }

    fn get_next_partial_snippet(&mut self, reader: &mut dyn CharReader) -> Result<bool> {
        if let Some(snippet) = &self.snippet {
            if snippet.len() >= self.start_term + 1 + self.n {
                self.start_term += 1;
                self.start_offset += 1;
                self.len_term = self.n;
                return Ok(true);
            }
        }
        self.get_next_snippet(reader)
    }

    fn get_next_snippet(&mut self, reader: &mut dyn CharReader) -> Result<bool> {
        self.start_term = 0;
        self.start_offset = self.next_start_offset;
        self.snippet_buffer.clear();
        loop {
            if self.ch != -1 {
                self.ch = self.read_char_from_buffer(reader)?;
            }
            if self.ch == -1 {
                break;
            } else if !self.is_delimiter(self.ch) {
                self.snippet_buffer.push(self.ch as u16);
            } else if !self.snippet_buffer.is_empty() {
                break;
            } else {
                self.start_offset += 1;
            }
        }
        if self.snippet_buffer.is_empty() {
            return Ok(false);
        }
        let snippet = self.snippet_buffer.clone();
        self.len_term = if snippet.len() >= self.n {
            self.n
        } else {
            snippet.len()
        };
        self.snippet = Some(snippet);
        Ok(true)
    }

    fn read_char_from_buffer(&mut self, reader: &mut dyn CharReader) -> Result<i32> {
        if self.char_buffer_index as i32 >= self.char_buffer_len {
            let read = reader.read(&mut self.char_buffer)?;
            // `Reader.read` answers -1 at the end of the stream, Rucene's 0
            self.char_buffer_len = if read == 0 { -1 } else { read as i32 };
            if self.char_buffer_len == -1 {
                return Ok(-1);
            }
            self.char_buffer_index = 0;
        }
        let c = i32::from(self.char_buffer[self.char_buffer_index]);
        self.char_buffer_index += 1;
        self.next_start_offset += 1;
        Ok(c)
    }

    fn is_delimiter(&self, c: i32) -> bool {
        self.delimiters.iter().any(|&d| i32::from(d) == c)
    }
}

impl TokenizerLogic for BasicNGramLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if !self.get_next_partial_snippet(reader)? {
            return Ok(false);
        }
        source.clear_attributes();
        let term_att = Att::add(source, char_term_id())?;
        let offset_att = Att::add(source, offset_id())?;
        let snippet = self.snippet.as_ref().expect("a snippet");
        let text =
            String::from_utf16_lossy(&snippet[self.start_term..self.start_term + self.len_term]);
        {
            let mut term = term_att.char_term_mut();
            term.set_empty();
            term.append_string(Some(&text));
        }
        let start = Tokenizer::<Self>::correct_offset_of(Some(&*reader), self.start_offset);
        let end = Tokenizer::<Self>::correct_offset_of(
            Some(&*reader),
            self.start_offset + self.len_term as i32,
        );
        offset_att.offset_mut().set_offset(start, end);
        Ok(true)
    }

    fn end(&mut self, source: &mut AttributeSource, _input: Option<&dyn CharReader>) -> Result<()> {
        let offset_att = Att::add(source, offset_id())?;
        offset_att
            .offset_mut()
            .set_offset(self.get_final_offset(), self.get_final_offset());
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.start_term = 0;
        self.next_start_offset = 0;
        self.snippet = None;
        self.snippet_buffer.clear();
        self.char_buffer_index = BUFFER_SIZE;
        self.char_buffer_len = 0;
        self.ch = 0;
        Ok(())
    }
}

/// `new BasicNGramTokenizer()`: its `termAtt` and `offsetAtt` are added at
/// construction.
pub fn basic_ngram_tokenizer() -> BasicNGramTokenizer {
    let mut t = Tokenizer::new(BasicNGramLogic::new());
    t.attribute_source_mut()
        .add_attribute_interface::<dyn OffsetAttribute>()
        .expect("the default factory implements OffsetAttribute");
    t
}

/// `make1d1fIndex(value)`.
pub fn make1d1f_index(f: &mut Fixture, value: &str) -> Result<()> {
    make1dmf_index(f, &[value])
}

/// `make1d1fIndexB(value)`.
pub fn make1d1f_index_b(f: &mut Fixture, value: &str) -> Result<()> {
    make1dmf_index_b(f, &[value])
}

/// `make1dmfIndex(values...)`.
pub fn make1dmf_index(f: &mut Fixture, values: &[&str]) -> Result<()> {
    let analyzer = Arc::clone(f.analyzer_w.as_ref().expect("setUp ran"));
    make1dmf_index_with(f, analyzer, values)
}

/// `make1dmfIndexB(values...)`.
pub fn make1dmf_index_b(f: &mut Fixture, values: &[&str]) -> Result<()> {
    let analyzer = Arc::clone(f.analyzer_b.as_ref().expect("setUp ran"));
    make1dmf_index_with(f, analyzer, values)
}

/// `new FieldType(TextField.TYPE_STORED)` with term vectors, offsets and
/// positions.
pub fn tv_type(stored: bool) -> Result<Arc<FieldType>> {
    let custom_type = FieldType::new_from(if stored {
        TextField::type_stored()
    } else {
        TextField::type_not_stored()
    });
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_offsets(true)?;
    custom_type.set_store_term_vector_positions(true)?;
    Ok(Arc::new(custom_type))
}

/// `make1dmfIndex(analyzer, values...)`.
pub fn make1dmf_index_with(
    f: &mut Fixture,
    analyzer: Arc<dyn Analyzer>,
    values: &[&str],
) -> Result<()> {
    let config = IndexWriterConfig::with_analyzer(Some(analyzer));
    config.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(f.dir(), config)?;
    let mut doc = Document::new();
    let custom_type = tv_type(true)?;
    for value in values {
        doc.add(Field::new(F, value.to_string(), Arc::clone(&custom_type))?);
    }
    writer.add_document(&doc)?;
    writer.close()?;
    reopen(f, true)
}

/// `if (reader != null) reader.close(); reader = DirectoryReader.open(dir);`
/// and, when `searcher`, `searcher = newSearcher(reader);`.
pub fn reopen(f: &mut Fixture, searcher: bool) -> Result<()> {
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    let reader = directory_reader::open(f.dir())?;
    if searcher {
        f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    }
    f.reader = Some(reader);
    Ok(())
}

/// `make1dmfIndexNA(values...)`.
pub fn make1dmf_index_na(f: &mut Fixture, values: &[&str]) -> Result<()> {
    let analyzer = Arc::clone(f.analyzer_k.as_ref().expect("setUp ran"));
    make1dmf_index_with(f, analyzer, values)
}

/// `makeIndexShortMV()`.
pub fn make_index_short_mv(f: &mut Fixture) -> Result<()> {
    make1dmf_index(f, &SHORT_MV_VALUES)
}

/// `makeIndexLongMV()`.
pub fn make_index_long_mv(f: &mut Fixture) -> Result<()> {
    make1dmf_index(f, &LONG_MV_VALUES)
}

/// `makeIndexLongMVB()`.
pub fn make_index_long_mvb(f: &mut Fixture) -> Result<()> {
    make1dmf_index_b(f, &BI_MV_VALUES)
}

/// `makeIndexStrMV()`.
pub fn make_index_str_mv(f: &mut Fixture) -> Result<()> {
    make1dmf_index_na(f, &STR_MV_VALUES)
}
