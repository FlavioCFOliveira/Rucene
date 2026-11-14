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

//! Java oracle of the attribute view a `ConditionalTokenFilter` adds to
//! after its wrapped filters were built (rmp #451): `ProtectedTermFilter`
//! and `CustomAnalyzer.Builder.when_term` add the `CharTermAttribute` after
//! `super(input, inputFactory)` built the `OneTimeWrapper` and the wrapped
//! chain over the input's attributes. Over a tokenizer that has no
//! `CharTermAttribute` (offsets only), each scenario of
//! `ConditionalAttributeOracle.java` (Lucene 10.5.0) and its Rust
//! counterpart here print every token's term, offsets and position
//! increment, the end state and the attribute interfaces the stream holds,
//! and the two dumps must be equal.

mod analysis_oracle;
mod support;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute,
};
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, CharArraySet, CharReader, StringCharReader,
    TokenStream, Tokenizer, TokenizerLogic,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeFactory, AttributeSource};
use rucene_analysis_common::analysis::custom::custom_analyzer::CustomAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::drop_if_flagged_filter::new_drop_if_flagged_filter;
use rucene_analysis_common::analysis::miscellaneous::protected_term_filter::new_protected_term_filter;
use rucene_analysis_common::analysis::miscellaneous::type_as_synonym_filter::new_type_as_synonym_filter;

/// `ConditionalAttributeOracle.TEXT`.
const TEXT: &str = "a bb ccc dddd";

/// `NoTermTokenizer`: one token per run of non-space characters, with
/// offsets only.
#[derive(Debug, Default)]
struct NoTermLogic {
    text: Option<Vec<u16>>,
    pos: usize,
}

impl TokenizerLogic for NoTermLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.text.is_none() {
            let mut text = Vec::new();
            let mut buf = [0u16; 64];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                text.extend_from_slice(&buf[..n]);
            }
            self.text = Some(text);
        }
        let text = self.text.as_ref().expect("read above");
        while self.pos < text.len() && text[self.pos] == b' ' as u16 {
            self.pos += 1;
        }
        if self.pos == text.len() {
            return Ok(false);
        }
        let start = self.pos;
        while self.pos < text.len() && text[self.pos] != b' ' as u16 {
            self.pos += 1;
        }
        let (start, end) = (
            Tokenizer::<NoTermLogic>::correct_offset_of(Some(reader), start as i32),
            Tokenizer::<NoTermLogic>::correct_offset_of(Some(reader), self.pos as i32),
        );
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("offsetAtt")
            .set_offset(start, end);
        Ok(true)
    }

    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let length = self.text.as_ref().map_or(0, |t| t.len()) as i32;
        let final_offset = Tokenizer::<NoTermLogic>::correct_offset_of(input, length);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("offsetAtt")
            .set_offset(final_offset, final_offset);
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.text = None;
        self.pos = 0;
        Ok(())
    }
}

/// `new NoTermTokenizer(factory)`.
fn no_term_tokenizer<L: TokenizerLogic + 'static>(
    factory: Arc<dyn AttributeFactory>,
    logic: L,
) -> Result<Tokenizer<L>> {
    let mut tokenizer = Tokenizer::with_factory(factory, logic);
    tokenizer
        .attribute_source_mut()
        .add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(tokenizer)
}

/// `NoTermTokenizerFactory`.
#[derive(Debug)]
struct NoTermTokenizerFactory;

impl TokenizerFactory for NoTermTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.rucene.codec.ConditionalAttributeOracle$NoTermTokenizerFactory"
    }

    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        no_term_tokenizer(
            factory,
            Box::new(NoTermLogic::default()) as Box<dyn TokenizerLogic>,
        )
    }
}

/// `NoTermTokenizerFactory(Map<String, String>)`.
fn new_no_term_tokenizer_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenizerFactory>> {
    AbstractAnalysisFactory::new(args)?;
    if !args.is_empty() {
        return Err(LuceneError::illegal_argument(format!(
            "Unknown parameters: {args:?}"
        )));
    }
    Ok(Arc::new(NoTermTokenizerFactory))
}

/// The class `NoTermTokenizerFactory.class`.
const NO_TERM_TOKENIZER_FACTORY: AnalysisFactoryClass<dyn TokenizerFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.rucene.codec.ConditionalAttributeOracle$NoTermTokenizerFactory",
        SPINameField::Missing,
        Some(new_no_term_tokenizer_factory),
    );

/// `dump(String name, TokenStream ts, PrintWriter out)`.
fn dump(name: &str, ts: &mut dyn TokenStream, out: &mut String) -> Result<()> {
    writeln!(out, "scenario {name}").unwrap();
    {
        let src = ts.attribute_source_mut();
        src.add_attribute_interface::<dyn CharTermAttribute>()?;
        src.add_attribute_interface::<dyn OffsetAttribute>()?;
        src.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    }
    let state = |ts: &dyn TokenStream| -> (String, i32, i32, i32) {
        let src = ts.attribute_source();
        let term = src
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        let offset = src
            .get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap();
        let pos_inc = src
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .unwrap();
        (
            term.buffer()[..term.length()].iter().collect(),
            offset.start_offset(),
            offset.end_offset(),
            pos_inc.get_position_increment(),
        )
    };
    ts.reset()?;
    while ts.increment_token()? {
        let (term, start, end, pos_inc) = state(ts);
        writeln!(out, "tok term={term} offset={start}-{end} posInc={pos_inc}").unwrap();
    }
    ts.end()?;
    let (_, start, end, pos_inc) = state(ts);
    writeln!(out, "end offset={start}-{end} posInc={pos_inc}").unwrap();
    let known = analysis_oracle::known_attributes();
    let mut names: Vec<&str> = ts
        .attribute_source()
        .attribute_classes()
        .map(|id| {
            known
                .iter()
                .find(|(k, _)| *k == id)
                .map_or("?", |(_, n)| *n)
        })
        .collect();
    names.sort_unstable();
    writeln!(out, "attrs {}", names.join(",")).unwrap();
    ts.close()
}

/// A `NoTermTokenizer` over `TEXT`.
fn no_term() -> Result<Box<dyn TokenStream>> {
    let mut t = no_term_tokenizer(default_token_attribute_factory(), NoTermLogic::default())?;
    t.set_reader(Box::new(StringCharReader::new(TEXT)))?;
    Ok(Box::new(t))
}

/// `new CharArraySet(List.of(words), false)`.
fn set(words: &[&str]) -> CharArraySet {
    CharArraySet::from_iter(words.iter().copied(), false)
}

/// Rucene's dump of every scenario of `ConditionalAttributeOracle.main`.
fn rust_dump() -> Result<String> {
    support::ensure_spi();
    let mut out = String::new();
    dump(
        "protected-drop",
        &mut new_protected_term_filter(Some(set(&["zz"])), no_term()?, |input| {
            Ok(Box::new(new_drop_if_flagged_filter(input, 0)?) as Box<dyn TokenStream>)
        })?,
        &mut out,
    )?;
    dump(
        "protected-empty-term",
        &mut new_protected_term_filter(Some(set(&[""])), no_term()?, |input| {
            Ok(Box::new(new_drop_if_flagged_filter(input, 0)?) as Box<dyn TokenStream>)
        })?,
        &mut out,
    )?;
    dump(
        "protected-type-as-synonym",
        &mut new_protected_term_filter(Some(set(&["word"])), no_term()?, |input| {
            Ok(Box::new(new_type_as_synonym_filter(input)?) as Box<dyn TokenStream>)
        })?,
        &mut out,
    )?;
    for empty in [true, false] {
        let mut builder = CustomAnalyzer::builder();
        builder.with_tokenizer_class(&NO_TERM_TOKENIZER_FACTORY, &[])?;
        let mut condition = builder.when_term(Arc::new(move |t: &str| t.is_empty() == empty));
        condition.add_token_filter("dropIfFlagged", &["dropFlags", "0"])?;
        condition.endwhen()?;
        let a = builder.build()?;
        let ts = a.token_stream("f", Box::new(StringCharReader::new(TEXT)))?;
        dump(
            &format!("when-term-{empty}"),
            &mut *ts.borrow_mut(),
            &mut out,
        )?;
        a.close();
    }
    Ok(out)
}

#[test]
fn conditional_filters_see_the_term_attribute_they_add() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         ConditionalAttributeOracle.java",
    );
    let (ok, java, stderr) = analysis_oracle::run_java_file(&source, &[]);
    assert!(ok, "ConditionalAttributeOracle.java failed:\n{stderr}");
    assert_eq!(java.matches("scenario ").count(), 5, "{java}");
    let rust = rust_dump().expect("the Rust scenarios run");
    assert_eq!(rust, java);
}
