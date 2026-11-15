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

//! Port of `TestMultiPhraseQueryParsing.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, CharReader, Tokenizer, TokenizerLogic};
use rucene::error::Result;
use rucene::search::multi_phrase_query::{Builder as MultiPhraseQueryBuilder, MultiPhraseQuery};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base::QueryParserBaseOps;
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::tokenizer_components;

use crate::qp_support::core_support::class;
use crate::qp_support::query_parser_test_base::FnAnalyzer;
use crate::qp_support::search_support::term;
use crate::qp_support::{assert_query_eq, is, jdk_default_hooks};

/// `TestMultiPhraseQueryParsing`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.push(crate::qp_support::core_support::test(
        "testMultiPhraseQueryParsing",
        test_multi_phrase_query_parsing,
    ));
    class(
        "org.apache.lucene.queryparser.classic.TestMultiPhraseQueryParsing",
        methods,
    )
}

/// `private record TokenAndPos(String token, int pos)`.
#[derive(Debug, Clone)]
struct TokenAndPos {
    token: &'static str,
    pos: i32,
}

/// `new CannedAnalyzer(TokenAndPos[] tokens)`.
fn canned_analyzer(tokens: Vec<TokenAndPos>) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.classic.TestMultiPhraseQueryParsing$CannedAnalyzer",
        move |_| {
            let mut source = AttributeSource::new_with_factory(
                rucene::analysis::default_token_attribute_factory(),
            );
            let term_att = Att::add(&mut source, char_term_id())?;
            let pos_incr_att = Att::add(&mut source, pos_inc_id())?;
            let t = Tokenizer::with_source(
                source,
                CannedTokenizer {
                    tokens: tokens.clone(),
                    upto: 0,
                    last_pos: 0,
                    term_att,
                    pos_incr_att,
                },
            );
            Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
        },
    )
    .arc()
}

/// `TestMultiPhraseQueryParsing.CannedTokenizer`.
#[derive(Debug)]
struct CannedTokenizer {
    tokens: Vec<TokenAndPos>,
    upto: usize,
    last_pos: i32,
    term_att: Att,
    pos_incr_att: Att,
}

impl TokenizerLogic for CannedTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.upto < self.tokens.len() {
            let token = self.tokens[self.upto].clone();
            self.upto += 1;
            self.term_att.char_term_mut().set_empty();
            self.term_att
                .char_term_mut()
                .append_string(Some(token.token));
            self.pos_incr_att
                .pos_inc_mut()
                .set_position_increment(token.pos - self.last_pos);
            self.last_pos = token.pos;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.upto = 0;
        self.last_pos = 0;
        Ok(())
    }
}

fn test_multi_phrase_query_parsing() -> Result<()> {
    let incr_0_query_tokens_and = vec![
        TokenAndPos { token: "a", pos: 0 },
        TokenAndPos { token: "1", pos: 0 },
        TokenAndPos { token: "b", pos: 1 },
        TokenAndPos { token: "1", pos: 1 },
        TokenAndPos { token: "c", pos: 2 },
    ];

    let mut qp = QueryParser::new(
        Some("field"),
        Some(canned_analyzer(incr_0_query_tokens_and)),
    )?;
    let q = qp.parse("\"this text is acually ignored\"")?;
    Assert::assert_true_msg("wrong query type!", is::<MultiPhraseQuery>(&*q));

    let mut multi_phrase_query_builder = MultiPhraseQueryBuilder::new();
    multi_phrase_query_builder.add_at(vec![term("field", "a"), term("field", "1")], -1)?;
    multi_phrase_query_builder.add_at(vec![term("field", "b"), term("field", "1")], 0)?;
    multi_phrase_query_builder.add_at(vec![term("field", "c")], 1)?;

    assert_query_eq(&multi_phrase_query_builder.build(), &*q);
    Ok(())
}
