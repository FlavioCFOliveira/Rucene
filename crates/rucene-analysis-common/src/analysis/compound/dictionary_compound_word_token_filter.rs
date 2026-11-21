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

//! The type declared in `DictionaryCompoundWordTokenFilter.java`, of the Java
//! package `org.apache.lucene.analysis.compound`.

use rucene::analysis::{CharArraySet, TokenFilter, TokenStream};
use rucene::error::{LuceneError, Result};

use super::compound_word_token_filter_base::{
    new_compound_word_token_filter, CompoundTermView, CompoundToken, CompoundWordTokenFilterBase,
    CompoundWordTokenFilterBaseSubclass, CompoundWordTokenFilterLogic, DEFAULT_MAX_SUBWORD_SIZE,
    DEFAULT_MIN_SUBWORD_SIZE, DEFAULT_MIN_WORD_SIZE,
};

/// What `DictionaryCompoundWordTokenFilter` adds to
/// `CompoundWordTokenFilterBase`: `onlyLongestMatchNoSubwords` and
/// `decompose()`.
#[derive(Debug, Clone, Default)]
pub struct DictionaryCompoundWordTokenFilterSubclass {
    only_longest_match_no_subwords: bool,
}

impl CompoundWordTokenFilterBaseSubclass for DictionaryCompoundWordTokenFilterSubclass {
    /// Equivalent to `DictionaryCompoundWordTokenFilter.decompose()`.
    fn decompose(&mut self, base: &mut CompoundWordTokenFilterBase, term: &CompoundTermView) {
        let only_longest_match = base.only_longest_match || self.only_longest_match_no_subwords;
        let len = term.length() as i64;
        let min_subword_size = base.min_subword_size as i64;
        let max_subword_size = base.max_subword_size as i64;
        let mut i: i64 = 0;
        while i <= len - min_subword_size {
            let mut longest_match_token: Option<CompoundToken> = None;
            let mut j = min_subword_size;
            while j <= max_subword_size {
                if i + j > len {
                    break;
                }
                if base.dictionary_contains(&term.buffer, i as usize, j as usize) {
                    if only_longest_match {
                        match &longest_match_token {
                            Some(longest) => {
                                if (longest.txt.len() as i64) < j {
                                    longest_match_token =
                                        Some(CompoundToken::new(term, i as usize, j as usize));
                                }
                            }
                            None => {
                                longest_match_token =
                                    Some(CompoundToken::new(term, i as usize, j as usize));
                            }
                        }
                    } else {
                        base.tokens
                            .push_back(CompoundToken::new(term, i as usize, j as usize));
                    }
                }
                j += 1;
            }

            if let Some(longest) = longest_match_token {
                let longest_len = longest.txt.len() as i64;
                base.tokens.push_back(longest);
                if self.only_longest_match_no_subwords {
                    i += longest_len - 1;
                }
            }
            i += 1;
        }
    }
}

/// A `TokenFilter` that decomposes compound words found in many Germanic
/// languages.
///
/// "Donaudampfschiff" becomes Donau, dampf, schiff so that you can find
/// "Donaudampfschiff" even when you only enter "schiff". It uses a
/// brute-force algorithm to achieve this.
///
/// Equivalent to `org.apache.lucene.analysis.compound.DictionaryCompoundWordTokenFilter`;
/// its constructors are [`new_dictionary_compound_word_token_filter`],
/// [`new_dictionary_compound_word_token_filter_with_sizes`] and the
/// deprecated [`new_dictionary_compound_word_token_filter_with_only_longest_match`].
pub type DictionaryCompoundWordTokenFilter =
    TokenFilter<CompoundWordTokenFilterLogic<DictionaryCompoundWordTokenFilterSubclass>>;

fn dictionary_must_not_be_null() -> LuceneError {
    LuceneError::illegal_argument("dictionary must not be null")
}

/// Creates a new `DictionaryCompoundWordTokenFilter` over `input`, matching
/// against `dictionary` (`None` for Java's `null`).
///
/// Equivalent to `DictionaryCompoundWordTokenFilter(TokenStream, CharArraySet)`.
///
/// # Errors
///
/// What the base constructor returns, then `IllegalArgumentException`
/// (`dictionary must not be null`) for a missing dictionary.
pub fn new_dictionary_compound_word_token_filter(
    input: Box<dyn TokenStream>,
    dictionary: Option<CharArraySet>,
) -> Result<DictionaryCompoundWordTokenFilter> {
    let missing = dictionary.is_none();
    let filter = new_compound_word_token_filter(
        input,
        dictionary,
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
        DictionaryCompoundWordTokenFilterSubclass::default(),
    )?;
    if missing {
        return Err(dictionary_must_not_be_null());
    }
    Ok(filter)
}

/// Creates a new `DictionaryCompoundWordTokenFilter`.
///
/// `min_word_size`: only words longer than this get processed;
/// `min_subword_size`: only subwords longer than this get to the output
/// stream; `max_subword_size`: only subwords shorter than this get to the
/// output stream; `only_longest_match`: deprecated, use parameter
/// `only_longest_match_ignore_subwords` instead;
/// `only_longest_match_ignore_subwords`: subwords are igored, e.g. if a word
/// contains 'schwein', only the longer word 'schwein' will be extracted, the
/// subword 'wein' will be ignored. Supersede parameter onlyLongestMatch.
///
/// Equivalent to the deprecated `DictionaryCompoundWordTokenFilter(TokenStream,
/// CharArraySet, int, int, int, boolean, boolean)`.
///
/// # Errors
///
/// As [`new_dictionary_compound_word_token_filter`].
#[allow(clippy::too_many_arguments)]
pub fn new_dictionary_compound_word_token_filter_with_only_longest_match(
    input: Box<dyn TokenStream>,
    dictionary: Option<CharArraySet>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
    only_longest_match_ignore_subwords: bool,
) -> Result<DictionaryCompoundWordTokenFilter> {
    let missing = dictionary.is_none();
    let filter = new_compound_word_token_filter(
        input,
        dictionary,
        min_word_size,
        min_subword_size,
        max_subword_size,
        only_longest_match,
        DictionaryCompoundWordTokenFilterSubclass {
            only_longest_match_no_subwords: only_longest_match_ignore_subwords,
        },
    )?;
    if missing {
        return Err(dictionary_must_not_be_null());
    }
    Ok(filter)
}

/// Creates a new `DictionaryCompoundWordTokenFilter`, with the parameters
/// of [`new_dictionary_compound_word_token_filter_with_only_longest_match`]
/// and `onlyLongestMatch` false.
///
/// Equivalent to `DictionaryCompoundWordTokenFilter(TokenStream,
/// CharArraySet, int, int, int, boolean)`.
///
/// # Errors
///
/// As [`new_dictionary_compound_word_token_filter`].
pub fn new_dictionary_compound_word_token_filter_with_sizes(
    input: Box<dyn TokenStream>,
    dictionary: Option<CharArraySet>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match_ignore_subwords: bool,
) -> Result<DictionaryCompoundWordTokenFilter> {
    new_dictionary_compound_word_token_filter_with_only_longest_match(
        input,
        dictionary,
        min_word_size,
        min_subword_size,
        max_subword_size,
        false,
        only_longest_match_ignore_subwords,
    )
}
