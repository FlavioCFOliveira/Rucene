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

//! The type declared in `HyphenationCompoundWordTokenFilter.java`, of the
//! Java package `org.apache.lucene.analysis.compound`.

use std::sync::Arc;

use rucene::analysis::{CharArraySet, TokenFilter, TokenStream};
use rucene::error::{LuceneError, Result};

use super::compound_word_token_filter_base::{
    new_compound_word_token_filter, CompoundTermView, CompoundToken, CompoundWordTokenFilterBase,
    CompoundWordTokenFilterBaseSubclass, CompoundWordTokenFilterLogic, DEFAULT_MAX_SUBWORD_SIZE,
    DEFAULT_MIN_SUBWORD_SIZE, DEFAULT_MIN_WORD_SIZE,
};
use super::hyphenation::hyphenation_tree::HyphenationTree;
use crate::jdk::sax::InputSource;

/// What `HyphenationCompoundWordTokenFilter` adds to
/// `CompoundWordTokenFilterBase`: the hyphenator, the three match flags and
/// `decompose()`.
#[derive(Debug, Clone)]
pub struct HyphenationCompoundWordTokenFilterSubclass {
    hyphenator: Arc<HyphenationTree>,
    no_sub_matches: bool,
    no_overlapping_matches: bool,
    calc_sub_matches: bool,
}

impl CompoundWordTokenFilterBaseSubclass for HyphenationCompoundWordTokenFilterSubclass {
    /// Equivalent to `HyphenationCompoundWordTokenFilter.decompose()`.
    fn decompose(&mut self, base: &mut CompoundWordTokenFilterBase, term: &CompoundTermView) {
        let length = term.length();
        // if the token is in the dictionary and we are not interested in subMatches
        // we can skip decomposing this token (see testNoSubAndTokenInDictionary unit test)
        // NOTE:
        // we check against token and the token that is one character
        // shorter to avoid problems with genitive 's characters and other binding characters
        if base.dictionary.is_some()
            && !self.calc_sub_matches
            && (base.dictionary_contains(&term.buffer, 0, length)
                || length > 1 && base.dictionary_contains(&term.buffer, 0, length - 1))
        {
            return; // the whole token is in the dictionary - do not decompose
        }
        // get the hyphenation points
        let hyphens = self.hyphenator.hyphenate(&term.buffer, 0, length, 1, 1);
        // No hyphen points found -> exit
        let Some(hyphens) = hyphens else {
            return;
        };
        let max_subword_size = base.max_subword_size.min(length as i32 - 1);
        let mut consumed: i32 = -1; // hyp of the longest token added (for noSub)
        let hyp = hyphens.get_hyphenation_points();
        let mut i: i32 = 0;
        while (i as usize) < hyp.len() {
            if self.no_overlapping_matches {
                // if we do not want overlapping subwords
                i = i.max(consumed); // skip over consumed hyp
            }
            let start = hyp[i as usize];
            let until = if self.no_sub_matches {
                consumed.max(i)
            } else {
                i
            };
            let mut j = hyp.len() as i32 - 1;
            while j > until {
                let part_length = hyp[j as usize] - start;
                // if the part is longer than maxSubwordSize we
                // are done with this round
                if part_length > max_subword_size {
                    j -= 1;
                    continue;
                }
                // we only put subwords to the token stream
                // that are longer than minPartSize
                if part_length < base.min_subword_size {
                    // BOGUS/BROKEN/FUNKY/WACKO: somehow we have negative 'parts' according to the
                    // calculation above, and we rely upon minSubwordSize being >=0 to filter them out...
                    break;
                }
                // check the dictionary (a negative length, which only
                // `partLength - 1` can be, is never contained: Java's
                // `CharArrayMap.getSlot` finds no key of that length)
                if base.dictionary.is_none()
                    || base.dictionary_contains(&term.buffer, start as usize, part_length as usize)
                {
                    base.tokens.push_back(CompoundToken::new(
                        term,
                        start as usize,
                        part_length as usize,
                    ));
                    consumed = j; // mark the current hyp as consumed
                    if !self.calc_sub_matches {
                        break; // do not search for shorter matches
                    }
                } else if part_length > 0
                    && base.dictionary_contains(
                        &term.buffer,
                        start as usize,
                        part_length as usize - 1,
                    )
                {
                    // check the dictionary again with a word that is one character
                    // shorter to avoid problems with genitive 's characters and
                    // other binding characters
                    base.tokens.push_back(CompoundToken::new(
                        term,
                        start as usize,
                        part_length as usize - 1,
                    ));
                    consumed = j; // mark the current hyp as consumed
                    if !self.calc_sub_matches {
                        break; // do not search for shorter matches
                    }
                } // else dictionary is present but does not contain the part
                j -= 1;
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
/// hyphenation grammar and a word dictionary to achieve this.
///
/// Equivalent to `org.apache.lucene.analysis.compound.HyphenationCompoundWordTokenFilter`;
/// its constructors are the `new_hyphenation_compound_word_token_filter*`
/// functions of this module, its static methods [`get_hyphenation_tree`] and
/// [`get_hyphenation_tree_from_source`]. The hyphenation tree, which Java
/// shares by reference, is shared through an [`Arc`].
pub type HyphenationCompoundWordTokenFilter =
    TokenFilter<CompoundWordTokenFilterLogic<HyphenationCompoundWordTokenFilterSubclass>>;

/// Creates a new `HyphenationCompoundWordTokenFilter` instance with every
/// parameter: `hyphenator` (Java's `requireNonNull` is checked after the
/// base's size checks, `NullPointerException` `hyphenator` for `None`), the
/// word `dictionary` to match against (`None` for none), `min_word_size`
/// (only words longer than this get processed), `min_subword_size` and
/// `max_subword_size` (only subwords longer, shorter than these get to the
/// output stream), `only_longest_match` (add only the longest matching
/// subword to the stream), `no_sub_matches` (excludes subwords that are
/// enclosed by an other token) and `no_overlapping_matches` (excludes
/// subwords that overlap with an other subword).
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_filter(
    input: Box<dyn TokenStream>,
    hyphenator: Option<Arc<HyphenationTree>>,
    dictionary: Option<CharArraySet>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
    no_sub_matches: bool,
    no_overlapping_matches: bool,
) -> Result<HyphenationCompoundWordTokenFilter> {
    // The base's checks run before `requireNonNull`; the subclass is only
    // built when the hyphenator is there.
    let base_check = CompoundWordTokenFilterBase::new(
        None,
        min_word_size,
        min_subword_size,
        max_subword_size,
        only_longest_match,
    );
    let hyphenator = match (base_check, hyphenator) {
        (Err(e), _) => return Err(e),
        (Ok(_), None) => {
            return Err(LuceneError::unchecked(
                "java.lang.NullPointerException",
                "hyphenator",
            ))
        }
        (Ok(_), Some(hyphenator)) => hyphenator,
    };
    new_compound_word_token_filter(
        input,
        dictionary,
        min_word_size,
        min_subword_size,
        max_subword_size,
        only_longest_match,
        HyphenationCompoundWordTokenFilterSubclass {
            hyphenator,
            no_sub_matches,
            no_overlapping_matches,
            calc_sub_matches: !only_longest_match && !no_sub_matches && !no_overlapping_matches,
        },
    )
}

/// Creates a new `HyphenationCompoundWordTokenFilter` instance, with the
/// default sizes and no match flag.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter(TokenStream,
/// HyphenationTree, CharArraySet)`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement.
pub fn new_hyphenation_compound_word_token_filter(
    input: Box<dyn TokenStream>,
    hyphenator: Arc<HyphenationTree>,
    dictionary: Option<CharArraySet>,
) -> Result<HyphenationCompoundWordTokenFilter> {
    new_filter(
        input,
        Some(hyphenator),
        dictionary,
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
        false,
        false,
    )
}

/// Creates a new `HyphenationCompoundWordTokenFilter` instance; see
/// [`new_hyphenation_compound_word_token_filter_with_flags`] for the
/// parameters.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter(TokenStream,
/// HyphenationTree, CharArraySet, int, int, int, boolean)`.
///
/// # Errors
///
/// `IllegalArgumentException` for a negative size, and what the stream's
/// attribute factory raises.
#[allow(clippy::too_many_arguments)]
pub fn new_hyphenation_compound_word_token_filter_with_sizes(
    input: Box<dyn TokenStream>,
    hyphenator: Arc<HyphenationTree>,
    dictionary: Option<CharArraySet>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
) -> Result<HyphenationCompoundWordTokenFilter> {
    new_filter(
        input,
        Some(hyphenator),
        dictionary,
        min_word_size,
        min_subword_size,
        max_subword_size,
        only_longest_match,
        false,
        false,
    )
}

/// Creates a new `HyphenationCompoundWordTokenFilter` instance.
///
/// `hyphenator`: the hyphenation pattern tree to use for hyphenation;
/// `dictionary`: the word dictionary to match against; `min_word_size`:
/// only words longer than this get processed; `min_subword_size`: only
/// subwords longer than this get to the output stream; `max_subword_size`:
/// only subwords shorter than this get to the output stream;
/// `only_longest_match`: add only the longest matching subword to the
/// stream; `no_sub_matches`: excludes subwords that are enclosed by an other
/// token; `no_overlapping_matches`: excludes subwords that overlap with an
/// other subword.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter(TokenStream,
/// HyphenationTree, CharArraySet, int, int, int, boolean, boolean, boolean)`.
///
/// # Errors
///
/// `IllegalArgumentException` for a negative size, and what the stream's
/// attribute factory raises.
#[allow(clippy::too_many_arguments)]
pub fn new_hyphenation_compound_word_token_filter_with_flags(
    input: Box<dyn TokenStream>,
    hyphenator: Arc<HyphenationTree>,
    dictionary: Option<CharArraySet>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
    only_longest_match: bool,
    no_sub_matches: bool,
    no_overlapping_matches: bool,
) -> Result<HyphenationCompoundWordTokenFilter> {
    new_filter(
        input,
        Some(hyphenator),
        dictionary,
        min_word_size,
        min_subword_size,
        max_subword_size,
        only_longest_match,
        no_sub_matches,
        no_overlapping_matches,
    )
}

/// Create a HyphenationCompoundWordTokenFilter with no dictionary.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter(TokenStream,
/// HyphenationTree, int, int, int)`.
///
/// # Errors
///
/// `IllegalArgumentException` for a negative size, and what the stream's
/// attribute factory raises.
pub fn new_hyphenation_compound_word_token_filter_without_dictionary(
    input: Box<dyn TokenStream>,
    hyphenator: Arc<HyphenationTree>,
    min_word_size: i32,
    min_subword_size: i32,
    max_subword_size: i32,
) -> Result<HyphenationCompoundWordTokenFilter> {
    new_hyphenation_compound_word_token_filter_with_sizes(
        input,
        hyphenator,
        None,
        min_word_size,
        min_subword_size,
        max_subword_size,
        false,
    )
}

/// Create a HyphenationCompoundWordTokenFilter with no dictionary and the
/// default sizes.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter(TokenStream, HyphenationTree)`.
///
/// # Errors
///
/// What the stream's attribute factory raises.
pub fn new_hyphenation_compound_word_token_filter_with_hyphenator(
    input: Box<dyn TokenStream>,
    hyphenator: Arc<HyphenationTree>,
) -> Result<HyphenationCompoundWordTokenFilter> {
    new_hyphenation_compound_word_token_filter_without_dictionary(
        input,
        hyphenator,
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
    )
}

/// Create a hyphenator tree from the XML grammar file `hyphenation_filename`.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter.getHyphenationTree(String)`.
///
/// # Errors
///
/// If there is a low-level I/O error.
pub fn get_hyphenation_tree(hyphenation_filename: &str) -> Result<HyphenationTree> {
    get_hyphenation_tree_from_source(InputSource::from_system_id(hyphenation_filename))
}

/// Create a hyphenator tree from the InputSource pointing to the XML grammar.
///
/// Equivalent to `HyphenationCompoundWordTokenFilter.getHyphenationTree(InputSource)`.
///
/// # Errors
///
/// If there is a low-level I/O error.
pub fn get_hyphenation_tree_from_source(
    hyphenation_source: InputSource,
) -> Result<HyphenationTree> {
    let mut tree = HyphenationTree::new();
    tree.load_patterns(hyphenation_source)?;
    Ok(tree)
}
