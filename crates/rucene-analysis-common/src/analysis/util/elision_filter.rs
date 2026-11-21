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

//! Removes elisions from a token stream, ported from
//! `org.apache.lucene.analysis.util.ElisionFilter`.

use rucene::analysis::{
    CharArraySet, CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// The body of [`ElisionFilter`]: what Lucene's `ElisionFilter` adds to
/// `TokenFilter`.
#[derive(Debug)]
pub struct ElisionFilterLogic {
    articles: CharArraySet,
    /// The UTF-16 units of the term's prefix, which `articles.contains`
    /// reads.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's term buffer is the `char[]`
    /// `articles.contains(termBuffer, 0, index)` reads directly; `rucene`'s
    /// `CharTermAttribute` holds `char`s while `CharArraySet` keys are
    /// UTF-16 units (as Java's are), so the prefix is encoded here first, as
    /// `rucene`'s `StopFilter` does.
    scratch: Vec<u16>,
}

impl TokenFilterLogic for ElisionFilterLogic {
    /// Increments the `TokenStream` with a `CharTermAttribute` without
    /// elisioned start.
    ///
    /// Equivalent to `ElisionFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let term_length = term_att.length();

            let mut index = None;
            for (i, &ch) in term_att.buffer()[..term_length].iter().enumerate() {
                if ch == '\'' || ch == '\u{2019}' {
                    index = Some(i);
                    break;
                }
            }

            // An apostrophe has been found. If the prefix is an article strip it off.
            if let Some(index) = index {
                self.scratch.clear();
                let mut units = [0u16; 2];
                for c in &term_att.buffer()[..index] {
                    self.scratch.extend_from_slice(c.encode_utf16(&mut units));
                }
                if self
                    .articles
                    .contains_chars(Some(&self.scratch), 0, self.scratch.len())
                {
                    let rest: Vec<char> = term_att.buffer()[index + 1..term_length].to_vec();
                    term_att.copy_buffer(&rest, 0, rest.len());
                }
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Removes elisions from a `TokenStream`. For example, "l'avion" (the plane)
/// will be tokenized as "avion" (plane).
///
/// Equivalent to `org.apache.lucene.analysis.util.ElisionFilter`.
pub type ElisionFilter = TokenFilter<ElisionFilterLogic>;

/// Constructs an elision filter with a Set of stop words: `input` is the
/// source `TokenStream`, `articles` a set of stopword articles.
///
/// Equivalent to `ElisionFilter(TokenStream, CharArraySet)`, which adds the
/// `CharTermAttribute`. Java's `Objects.requireNonNull(articles, "articles")`
/// has no counterpart: `articles` cannot be null here.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_elision_filter(
    input: Box<dyn TokenStream>,
    articles: CharArraySet,
) -> Result<ElisionFilter> {
    let mut filter = TokenFilter::new(
        input,
        ElisionFilterLogic {
            articles,
            scratch: Vec::new(),
        },
    );
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
