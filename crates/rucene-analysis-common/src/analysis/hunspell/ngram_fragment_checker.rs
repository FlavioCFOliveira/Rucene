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

//! A fragment checker over the n-grams of a language, ported from
//! `org.apache.lucene.analysis.hunspell.NGramFragmentChecker`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};

use super::affixed_word::AffixedWord;
use super::dictionary::Dictionary;
use super::fragment_checker::FragmentChecker;
use super::hunspell::CheckCanceled;
use super::word_case::WordCase;
use super::word_form_generator::{WordFormGenerator, WordFormGeneratorSubclass};
use crate::jdk::character::{to_upper_case_root, to_upper_case_unit};

/// `java.util.BitSet`, as the checker uses it: a set of bits whose `size()`
/// is the bits its words hold, 64 per word.
#[derive(Debug, Clone)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    /// Equivalent to `new BitSet(int nbits)`.
    fn new(nbits: i32) -> Result<Self> {
        if nbits < 0 {
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                format!("nbits < 0: {nbits}"),
            ));
        }
        Ok(Self {
            words: vec![0; (nbits as usize).div_ceil(64)],
        })
    }

    /// Equivalent to `size()`.
    fn size(&self) -> i32 {
        (self.words.len() * 64) as i32
    }

    /// Equivalent to `get(int)` of an index below `size()`.
    fn get(&self, index: usize) -> bool {
        self.words[index / 64] & (1u64 << (index % 64)) != 0
    }

    /// Equivalent to `set(int)` of an index below `size()`.
    fn set(&mut self, index: usize) {
        self.words[index / 64] |= 1u64 << (index % 64);
    }

    /// Equivalent to `cardinality()`.
    fn cardinality(&self) -> i32 {
        self.words.iter().map(|w| w.count_ones() as i32).sum()
    }

    /// `Math.abs(hash % size())`, the slot of a hash.
    ///
    /// # Panics
    ///
    /// Where Java throws `ArithmeticException("/ by zero")`: a set of size
    /// 0.
    fn slot(&self, hash: i32) -> usize {
        let size = self.size();
        assert!(size != 0, "ArithmeticException: / by zero");
        (hash % size).unsigned_abs() as usize
    }
}

/// A [`FragmentChecker`] based on all character n-grams possible in a
/// certain language, keeping them in a relatively memory-efficient, but
/// probabilistic data structure. The n-gram length should be 2, 3 or 4.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.NGramFragmentChecker`;
/// see [`from_all_simple_words`](Self::from_all_simple_words) for enumerating
/// the whole dictionary automatically and [`from_words`](Self::from_words)
/// for creating an instance from a precomputed set of all word forms or
/// n-grams.
#[derive(Debug, Clone)]
pub struct NGramFragmentChecker {
    n: usize,
    hashes: BitSet,
}

impl NGramFragmentChecker {
    /// Equivalent to `NGramFragmentChecker(int, BitSet)`.
    fn new(n: i32, hashes: BitSet) -> Result<Self> {
        if !(2..=4).contains(&n) {
            return Err(LuceneError::illegal_argument(format!(
                "N should be between 2 and 4: {n}"
            )));
        }
        if hashes.cardinality() > hashes.size() * 2 / 3 {
            return Err(LuceneError::illegal_argument(
                "Too many collisions, please report this to dev@lucene.apache.org",
            ));
        }
        Ok(Self {
            n: n as usize,
            hashes,
        })
    }

    /// Equivalent to `hashCount()`.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub fn hash_count(&self) -> i32 {
        self.hashes.cardinality()
    }

    /// Iterate the whole dictionary, derive all word forms (using
    /// [`WordFormGenerator`]), vary the case to get all words acceptable by
    /// the spellchecker, and create a fragment checker based on their
    /// `n`-grams. Note that this enumerates only words derivable by suffixes
    /// and prefixes. If the language has compounds, some n-grams possible via
    /// those compounds can be missed. In the latter case, consider using
    /// [`from_words`](Self::from_words).
    ///
    /// Equivalent to `fromAllSimpleWords(int, Dictionary, Runnable)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an `n` outside `2..=4` or too many
    /// collisions, and what `check_canceled` raises.
    pub fn from_all_simple_words(
        n: i32,
        dictionary: Arc<Dictionary>,
        check_canceled: &CheckCanceled,
    ) -> Result<NGramFragmentChecker> {
        // some empirical numbers
        let mut hashes = BitSet::new(1i32.wrapping_shl((7 + n.wrapping_mul(3)) as u32))?;
        let mut consumer = CollectHashes {
            hashes: &mut hashes,
        };
        Self::process_n_grams(n, dictionary, check_canceled, &mut consumer)?;
        NGramFragmentChecker::new(n, hashes)
    }

    /// Create a fragment checker for n-grams found in the given words. The
    /// words can be n-grams themselves or full words of the language. The
    /// words are case-sensitive, so be sure to include upper-case and
    /// title-case variants if they're accepted by the spellchecker.
    ///
    /// Equivalent to `fromWords(int, Collection<? extends CharSequence>)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an `n` outside `2..=4` or too many
    /// collisions.
    pub fn from_words(n: i32, words: &[&str]) -> Result<NGramFragmentChecker> {
        let mut hashes = BitSet::new(java_highest_one_bit(words.len() as i32).wrapping_mul(4))?;
        {
            let mut consumer = CollectHashes {
                hashes: &mut hashes,
            };
            for word in words {
                let units: Vec<u16> = word.encode_utf16().collect();
                consumer.process_n_grams(n, &units);
            }
        }
        NGramFragmentChecker::new(n, hashes)
    }

    /// Traverse the whole dictionary, generate all word forms of its entries,
    /// and process all n-grams in these word forms. No duplication removal is
    /// done, so the `consumer` should be prepared to duplicate n-grams. The
    /// traversal order is undefined.
    ///
    /// Equivalent to `processNGrams(int, Dictionary, Runnable,
    /// NGramConsumer)`.
    ///
    /// # Errors
    ///
    /// What `check_canceled` raises.
    pub fn process_n_grams(
        n: i32,
        dictionary: Arc<Dictionary>,
        check_canceled: &CheckCanceled,
        consumer: &mut dyn NGramConsumer,
    ) -> Result<()> {
        let gen = WordFormGenerator::with_subclass(dictionary, Arc::new(OvergeneratingSubclass))?;
        let mut last_entry_stem: Option<Vec<u16>> = None;
        let mut last_entry_case: Option<WordCase> = None;
        gen.generate_all_simple_words(
            &mut |aw: AffixedWord| {
                let word = aw.word();
                consumer.process_n_grams(n, word);
                // `shouldVaryCase(aw.getDictEntry())`; Java compares the
                // entry objects, which differ when the stems do
                let stem = aw.get_dict_entry().stem();
                if last_entry_stem.as_deref() != Some(stem) {
                    last_entry_stem = Some(stem.to_vec());
                    last_entry_case = Some(WordCase::case_of(stem));
                }
                let should_vary_case = last_entry_case != Some(WordCase::Mixed)
                    && last_entry_case != Some(WordCase::Neutral);
                if should_vary_case {
                    consumer.process_n_grams(n, &to_upper_case_root(word));
                    if word.len() > 1 {
                        // `word.substring(1, Math.min(n, word.length()))`
                        let end = i64::from(n).min(word.len() as i64);
                        if end < 1 {
                            return Err(LuceneError::from(
                                LuceneErrorKind::StringIndexOutOfBounds(format!(
                                    "begin 1, end {end}, length {}",
                                    word.len()
                                )),
                            ));
                        }
                        let mut capitalized = vec![to_upper_case_unit(word[0])];
                        capitalized.extend_from_slice(&word[1..end as usize]);
                        consumer.process_n_grams(n, &capitalized);
                    }
                }
                Ok(())
            },
            check_canceled,
        )
    }

    /// Equivalent to `lowCollisionHash(CharSequence, int, int)`.
    fn low_collision_hash(chars: &[u16], offset: usize, end: usize) -> i32 {
        chars[offset..end].iter().fold(0i32, |result, &c| {
            result.wrapping_mul(239).wrapping_add(i32::from(c))
        })
    }
}

impl FragmentChecker for NGramFragmentChecker {
    fn has_impossible_fragment_around(&self, word: &[u16], start: usize, end: usize) -> bool {
        let n = self.n as i64;
        if (word.len() as i64) < n {
            return false;
        }
        let first_intersecting_start = (start as i64 - n + 1).max(0);
        let last_intersecting_start = (end as i64 - 1).min(word.len() as i64 - n);
        let mut i = first_intersecting_start;
        while i <= last_intersecting_start {
            let iu = i as usize;
            let hash = Self::low_collision_hash(word, iu, iu + self.n);
            if !self.hashes.get(self.hashes.slot(hash)) {
                return true;
            }
            i += 1;
        }
        false
    }
}

/// The `WordFormGenerator` subclass of `processNGrams`: overgenerate a bit,
/// but avoid very expensive checks.
struct OvergeneratingSubclass;

impl WordFormGeneratorSubclass for OvergeneratingSubclass {
    fn can_stem_to_original(
        &self,
        _generator: &WordFormGenerator,
        _derived: &AffixedWord,
    ) -> Result<bool> {
        Ok(true)
    }
}

/// The consumer of `collectHashes(BitSet)`.
struct CollectHashes<'a> {
    hashes: &'a mut BitSet,
}

impl NGramConsumer for CollectHashes<'_> {
    fn process_n_gram(&mut self, word: &[u16], start: usize, end: usize) {
        let hash = NGramFragmentChecker::low_collision_hash(word, start, end);
        let slot = self.hashes.slot(hash);
        self.hashes.set(slot);
    }
}

/// `Integer.highestOneBit(int)`.
fn java_highest_one_bit(i: i32) -> i32 {
    if i == 0 {
        0
    } else {
        (i32::MIN as u32 >> (i as u32).leading_zeros()) as i32
    }
}

/// A callback for n-gram ranges in words.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.NGramFragmentChecker$NGramConsumer`;
/// the word, a Java `CharSequence`, is given as its UTF-16 units.
pub trait NGramConsumer {
    /// Equivalent to `processNGram(CharSequence, int, int)`.
    fn process_n_gram(&mut self, word: &[u16], start: usize, end: usize);

    /// Call [`process_n_gram`](Self::process_n_gram) for each fragment of
    /// the length `n` in the given word.
    ///
    /// Equivalent to `processNGrams(int, CharSequence)`.
    ///
    /// **Divergence.** For a negative `n`, Java calls `processNGram` with an
    /// `end` before `start`, which `usize` ranges cannot express; no n-gram
    /// is processed instead. The checker's constructor rejects such an `n`
    /// either way.
    fn process_n_grams(&mut self, n: i32, word: &[u16]) {
        if n < 0 {
            return;
        }
        let n = n as usize;
        if word.len() >= n {
            for i in 0..=word.len() - n {
                self.process_n_gram(word, i, i + n);
            }
        }
    }
}
