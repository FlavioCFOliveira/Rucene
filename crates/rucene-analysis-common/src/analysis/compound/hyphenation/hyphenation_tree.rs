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

//! The type declared in `HyphenationTree.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

use std::collections::HashMap;
use std::io::Write;
use std::ops::{Deref, DerefMut};

use rucene::error::Result;

use super::byte_vector::ByteVector;
use super::hyphenation::Hyphenation;
use super::pattern_consumer::{HyphenatedWordPart, PatternConsumer};
use super::pattern_parser::PatternParser;
use super::ternary_tree::TernaryTree;
use crate::jdk::sax::InputSource;

/// This tree structure stores the hyphenation patterns in an efficient way
/// for fast lookup. It provides the provides the method to hyphenate a word.
///
/// This class has been taken from the Apache FOP project
/// (<http://xmlgraphics.apache.org/fop/>). They have been slightly modified.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.HyphenationTree`,
/// which extends `TernaryTree`: the tree it extends is reached through
/// [`Deref`] (its inherited public methods) and [`base`](Self::base).
#[derive(Debug, Clone)]
pub struct HyphenationTree {
    tree: TernaryTree,
    /// value space: stores the interletter values
    pub(crate) vspace: ByteVector,
    /// This map stores hyphenation exceptions
    pub(crate) stoplist: HashMap<Vec<u16>, Vec<HyphenatedWordPart>>,
    /// This map stores the character classes
    pub(crate) classmap: TernaryTree,
    /// Temporary map to store interletter values on pattern loading.
    ivalues: Option<TernaryTree>,
}

impl Default for HyphenationTree {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for HyphenationTree {
    type Target = TernaryTree;

    fn deref(&self) -> &TernaryTree {
        &self.tree
    }
}

impl DerefMut for HyphenationTree {
    fn deref_mut(&mut self) -> &mut TernaryTree {
        &mut self.tree
    }
}

impl HyphenationTree {
    /// Equivalent to `HyphenationTree()`.
    pub fn new() -> Self {
        let mut vspace = ByteVector::new();
        vspace.alloc(1); // this reserves index 0, which we don't use
        Self {
            tree: TernaryTree::new(),
            stoplist: HashMap::with_capacity(23), // usually a small table
            classmap: TernaryTree::new(),
            vspace,
            ivalues: None,
        }
    }

    /// The `TernaryTree` this tree extends.
    pub fn base(&self) -> &TernaryTree {
        &self.tree
    }

    /// Packs the values by storing them in 4 bits, two values into a byte
    /// Values range is from 0 to 9. We use zero as terminator, so we'll add 1
    /// to the value.
    ///
    /// `values` is a string of digits from '0' to '9' representing the
    /// interletter values; returns the index into the vspace array where the
    /// packed values are stored.
    ///
    /// Equivalent to the protected `HyphenationTree.packValues(String)`.
    pub(crate) fn pack_values(&mut self, values: &[u16]) -> usize {
        let n = values.len();
        let m = if (n & 1) == 1 {
            (n >> 1) + 2
        } else {
            (n >> 1) + 1
        };
        let offset = self.vspace.alloc(m);
        let va = self.vspace.get_array_mut();
        for (i, &value) in values.iter().enumerate() {
            let j = i >> 1;
            let v = ((value as i32 - '0' as i32 + 1) & 0x0f) as i8;
            if (i & 1) == 1 {
                va[j + offset] |= v;
            } else {
                va[j + offset] = ((v as i32) << 4) as i8; // big endian
            }
        }
        va[m - 1 + offset] = 0; // terminator
        offset
    }

    /// Equivalent to the protected `HyphenationTree.unpackValues(int)`.
    pub(crate) fn unpack_values(&self, mut k: usize) -> Vec<u16> {
        let mut buf = Vec::new();
        let mut v = self.vspace.get(k);
        k += 1;
        while v != 0 {
            let c = ((((v as i32) & 0xf0) >> 4) - 1 + '0' as i32) as u16;
            buf.push(c);
            let c = ((v as i32) & 0x0f) as u16;
            if c == 0 {
                break;
            }
            let c = c.wrapping_sub(1).wrapping_add('0' as u16);
            buf.push(c);
            v = self.vspace.get(k);
            k += 1;
        }
        buf
    }

    /// Read hyphenation patterns from an XML file.
    ///
    /// Equivalent to `HyphenationTree.loadPatterns(InputSource)`.
    ///
    /// # Errors
    ///
    /// The `IOException` `PatternParser.parse` throws when the parsing fails.
    pub fn load_patterns(&mut self, source: InputSource) -> Result<()> {
        // Java creates the parser, then `ivalues`; creating the parser has no
        // effect on the tree.
        self.ivalues = Some(TernaryTree::new());
        let mut pp = PatternParser::with_consumer(self);

        pp.parse(source)?;
        drop(pp);

        // patterns/values should be now in the tree
        // let's optimize a bit
        self.tree.trim_to_size();
        self.vspace.trim_to_size();
        self.classmap.trim_to_size();

        // get rid of the auxiliary map
        self.ivalues = None;
        Ok(())
    }

    /// Equivalent to `HyphenationTree.findPattern(String)`.
    pub fn find_pattern(&self, pat: &[u16]) -> Vec<u16> {
        let k = self.tree.find(pat);
        if k >= 0 {
            return self.unpack_values(k as usize);
        }
        Vec::new()
    }

    /// String compare, returns 0 if equal or t is a substring of s
    ///
    /// Equivalent to the protected `HyphenationTree.hstrcmp(char[], int, char[], int)`.
    pub(crate) fn hstrcmp(s: &[u16], mut si: usize, t: &[u16], mut ti: usize) -> i32 {
        while s[si] == t[ti] {
            if s[si] == 0 {
                return 0;
            }
            si += 1;
            ti += 1;
        }
        if t[ti] == 0 {
            return 0;
        }
        s[si] as i32 - t[ti] as i32
    }

    /// Equivalent to the protected `HyphenationTree.getValues(int)`.
    pub(crate) fn get_values(&self, mut k: usize) -> Vec<i8> {
        let mut buf: Vec<u16> = Vec::new();
        let mut v = self.vspace.get(k);
        k += 1;
        while v != 0 {
            let c = ((((v as i32) & 0xf0) >> 4) - 1) as u16;
            buf.push(c);
            let c = ((v as i32) & 0x0f) as u16;
            if c == 0 {
                break;
            }
            let c = c.wrapping_sub(1);
            buf.push(c);
            v = self.vspace.get(k);
            k += 1;
        }
        buf.iter().map(|&c| c as i8).collect()
    }

    /// Search for all possible partial matches of word starting at index an
    /// update interletter values. In other words, it does something like:
    /// `for(i=0; i<patterns.length; i++) { if ( word.substring(index).startsWidth(patterns[i]) ) update_interletter_values(patterns[i]); }`
    ///
    /// But it is done in an efficient way since the patterns are stored in a
    /// ternary tree. In fact, this is the whole purpose of having the tree:
    /// doing this search without having to test every single pattern. The
    /// number of patterns for languages such as English range from 4000 to
    /// 10000. Thus, doing thousands of string comparisons for each word to
    /// hyphenate would be really slow without the tree. The tradeoff is
    /// memory, but using a ternary tree instead of a trie, almost halves the
    /// memory used by Lout or TeX. It's also faster than using a hash table
    ///
    /// `word` is the null terminated word to match, `index` the start index
    /// from word, `il` the interletter values array to update.
    ///
    /// Equivalent to the protected `HyphenationTree.searchPatterns(char[], int, byte[])`.
    pub(crate) fn search_patterns(&self, word: &[u16], index: usize, il: &mut [i8]) {
        let t = &self.tree;
        let mut i = index;
        let mut sp = word[i];
        let mut p = t.root;

        while p > 0 && (p as usize) < t.sc.len() {
            let pi = p as usize;
            if t.sc[pi] == 0xFFFF {
                if Self::hstrcmp(word, i, t.kv.get_array(), t.lo[pi] as usize) == 0 {
                    let values = self.get_values(t.eq[pi] as usize); // data pointer is in eq[]
                    for (j, &value) in (index..).zip(values.iter()) {
                        if j < il.len() && value > il[j] {
                            il[j] = value;
                        }
                    }
                }
                return;
            }
            let d = sp as i32 - t.sc[pi] as i32;
            if d == 0 {
                if sp == 0 {
                    break;
                }
                i += 1;
                sp = word[i];
                p = t.eq[pi];
                let mut q = p;

                // look for a pattern ending at this position by searching for
                // the null char ( splitchar == 0 )
                while q > 0 && (q as usize) < t.sc.len() {
                    let qi = q as usize;
                    if t.sc[qi] == 0xFFFF {
                        // stop at compressed branch
                        break;
                    }
                    if t.sc[qi] == 0 {
                        let values = self.get_values(t.eq[qi] as usize);
                        for (j, &value) in (index..).zip(values.iter()) {
                            if j < il.len() && value > il[j] {
                                il[j] = value;
                            }
                        }
                        break;
                    } else {
                        q = t.lo[qi];

                        // actually the code should be: q = sc[q] < 0 ? hi[q] :
                        // lo[q]; but java chars are unsigned
                    }
                }
            } else {
                p = if d < 0 { t.lo[pi] } else { t.hi[pi] };
            }
        }
    }

    /// Hyphenate word and return a Hyphenation object.
    ///
    /// `remain_char_count` is the minimum number of characters allowed before
    /// the hyphenation point, `push_char_count` the minimum number of
    /// characters allowed after it. Returns `None` (Java's `null`) if the word
    /// is not hyphenated.
    ///
    /// Equivalent to `HyphenationTree.hyphenate(String, int, int)`.
    pub fn hyphenate_word(
        &self,
        word: &[u16],
        remain_char_count: i32,
        push_char_count: i32,
    ) -> Option<Hyphenation> {
        self.hyphenate(word, 0, word.len(), remain_char_count, push_char_count)
    }

    /// Hyphenate word and return an array of hyphenation points.
    ///
    /// w = "****nnllllllnnn*****", where n is a non-letter, l is a letter,
    /// all n may be absent, the first n is at offset, the first l is at
    /// offset + iIgnoreAtBeginning; word = ".llllll.'\0'***", where all l in
    /// w are copied into word. In the first part of the routine len =
    /// w.length, in the second part of the routine len = word.length. Three
    /// indices are used: index(w), the index in w, index(word), the index in
    /// word, letterindex(word), the index in the letter part of word. The
    /// following relations exist: index(w) = offset + i - 1 index(word) = i -
    /// iIgnoreAtBeginning letterindex(word) = index(word) - 1 (see first
    /// loop). It follows that: index(w) - index(word) = offset - 1 +
    /// iIgnoreAtBeginning index(w) = letterindex(word) + offset +
    /// iIgnoreAtBeginning
    ///
    /// `w` is the char array that contains the word, `offset` the offset to
    /// its first character, `len` its length. Returns `None` (Java's `null`)
    /// if the word is not hyphenated.
    ///
    /// Equivalent to `HyphenationTree.hyphenate(char[], int, int, int, int)`.
    pub fn hyphenate(
        &self,
        w: &[u16],
        offset: usize,
        mut len: usize,
        remain_char_count: i32,
        push_char_count: i32,
    ) -> Option<Hyphenation> {
        let mut word = vec![0u16; len + 3];

        // normalize word
        let mut c = [0u16; 2];
        let mut i_ignore_at_beginning = 0usize;
        let mut i_length = len;
        let mut b_end_of_letters = false;
        for i in 1..=len {
            c[0] = w[offset + i - 1];
            let nc = self.classmap.find_chars(&c, 0);
            if nc < 0 {
                // found a non-letter character ...
                if i == 1 + i_ignore_at_beginning {
                    // ... before any letter character
                    i_ignore_at_beginning += 1;
                } else {
                    // ... after a letter character
                    b_end_of_letters = true;
                }
                i_length -= 1;
            } else if !b_end_of_letters {
                word[i - i_ignore_at_beginning] = nc as u16;
            } else {
                return None;
            }
        }
        len = i_length;
        if (len as i32) < remain_char_count.wrapping_add(push_char_count) {
            // word is too short to be hyphenated
            return None;
        }
        let len_i = len as i32;
        let mut result = vec![0i32; len + 1];
        let mut k = 0usize;

        // check exception list first
        let sw = word[1..1 + len].to_vec();
        if let Some(hw) = self.stoplist.get(&sw) {
            // assume only simple hyphens (Hyphen.pre="-", Hyphen.post = Hyphen.no =
            // null)
            let mut j: i32 = 0;
            for o in hw {
                // j = index(sw) = letterindex(word)?
                // result[k] = corresponding index(w)
                if let HyphenatedWordPart::String(s) = o {
                    j = j.wrapping_add(s.len() as i32);
                    if j >= remain_char_count && j < len_i.wrapping_sub(push_char_count) {
                        result[k] = j.wrapping_add(i_ignore_at_beginning as i32);
                        k += 1;
                    }
                }
            }
        } else {
            // use algorithm to get hyphenation points
            word[0] = '.' as u16; // word start marker
            word[len + 1] = '.' as u16; // word end marker
            word[len + 2] = 0; // null terminated
            let mut il = vec![0i8; len + 3]; // initialized to zero
            for i in 0..len + 1 {
                self.search_patterns(&word, i, &mut il);
            }

            // hyphenation points are located where interletter value is odd
            // i is letterindex(word),
            // i + 1 is index(word),
            // result[k] = corresponding index(w)
            for i in 0..len_i {
                if (il[i as usize + 1] & 1) == 1
                    && i >= remain_char_count
                    && i <= len_i.wrapping_sub(push_char_count)
                {
                    result[k] = i + i_ignore_at_beginning as i32;
                    k += 1;
                }
            }
        }

        if k > 0 {
            // trim result array
            let mut res = vec![0i32; k + 2];
            res[1..1 + k].copy_from_slice(&result[..k]);
            // We add the synthetical hyphenation points
            // at the beginning and end of the word
            res[0] = 0;
            res[k + 1] = len_i;
            Some(Hyphenation::new(res))
        } else {
            None
        }
    }

    /// Equivalent to `HyphenationTree.printStats(PrintStream)`; like a
    /// `PrintStream`, it ignores the errors of `out`.
    pub fn print_stats(&self, out: &mut dyn Write) {
        let _ = writeln!(out, "Value space size = {}", self.vspace.length());
        self.tree.print_stats(out);
    }
}

impl PatternConsumer for HyphenationTree {
    /// Add a character class to the tree. It is used by [`PatternParser`] as
    /// callback to add character classes. Character classes define the valid
    /// word characters for hyphenation. If a word contains a character not
    /// defined in any of the classes, it is not hyphenated. It also defines a
    /// way to normalize the characters in order to compare them with the
    /// stored patterns. Usually pattern files use only lower case characters,
    /// in this case a class for letter 'a', for example, should be defined as
    /// "aA", the first character being the normalization char.
    ///
    /// Equivalent to `HyphenationTree.addClass(String)`.
    fn add_class(&mut self, chargroup: &[u16]) {
        if !chargroup.is_empty() {
            let equiv_char = chargroup[0];
            let mut key = [0u16; 2];
            key[1] = 0;
            for &ch in chargroup {
                key[0] = ch;
                self.classmap.insert_chars(&key, 0, equiv_char);
            }
        }
    }

    /// Add an exception to the tree. It is used by [`PatternParser`] class as
    /// callback to store the hyphenation exceptions.
    ///
    /// `word` is the normalized word, `hyphenatedword` a vector of
    /// alternating strings and [`Hyphen`](super::hyphen::Hyphen) objects.
    ///
    /// Equivalent to `HyphenationTree.addException(String, ArrayList<Object>)`.
    fn add_exception(&mut self, word: &[u16], hyphenatedword: Vec<HyphenatedWordPart>) {
        self.stoplist.insert(word.to_vec(), hyphenatedword);
    }

    /// Add a pattern to the tree. Mainly, to be used by [`PatternParser`]
    /// class as callback to add a pattern to the tree.
    ///
    /// `pattern` is the hyphenation pattern, `ivalue` the interletter weight
    /// values indicating the desirability and priority of hyphenating at a
    /// given point within the pattern. It should contain only digit
    /// characters. (i.e. '0' to '9').
    ///
    /// Equivalent to `HyphenationTree.addPattern(String, String)`; outside
    /// `loadPatterns` it panics where Java's `ivalues.find` throws
    /// `NullPointerException`.
    fn add_pattern(&mut self, pattern: &[u16], ivalue: &[u16]) {
        let mut k = self
            .ivalues
            .as_ref()
            .expect(
                "INVARIANT: Java's ivalues.find throws NullPointerException outside loadPatterns",
            )
            .find(ivalue);
        if k <= 0 {
            k = self.pack_values(ivalue) as i32;
            self.ivalues
                .as_mut()
                .expect("INVARIANT: checked above")
                .insert(ivalue, k as u16);
        }
        self.tree.insert(pattern, k as u16);
    }
}
