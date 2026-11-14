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

//! The Porter stemming algorithm, ported from
//! `org.apache.lucene.analysis.en.PorterStemmer`.
//!
//! Porter stemmer in Java. The original paper is in
//!
//! > Porter, 1980, An algorithm for suffix stripping, Program, Vol. 14,
//! > no. 3, pp 130-137,
//!
//! See also <https://snowballstem.org/algorithms/porter/stemmer.html>
//!
//! Bug 1 (reported by Gonzalo Parra 16/10/99) fixed as marked below.
//! Tthe words 'aed', 'eed', 'oed' leave k at 'a' for step 3, and b\[k-1\]
//! is then out outside the bounds of b.
//!
//! Similarly,
//!
//! Bug 2 (reported by Steve Dyrdahl 22/2/00) fixed as marked below.
//! 'ion' by itself leaves j = -1 in the test for 'ion' in step 5, and
//! b\[j\] is then outside the bounds of b.
//!
//! Release 3.
//!
//! \[ This version is derived from Release 3, modified by Brian Goetz to
//! optimize for fewer object creations. \]

use rucene::util::ArrayUtil;

/// Equivalent to `PorterStemmer.INITIAL_SIZE`.
const INITIAL_SIZE: usize = 50;

/// Stemmer, implementing the Porter Stemming Algorithm
///
/// The Stemmer class transforms a word into its root form. The input word
/// can be provided a character at time (by calling add()), or at once by
/// calling one of the various stem(something) methods.
///
/// Equivalent to `org.apache.lucene.analysis.en.PorterStemmer`. Its buffer
/// holds UTF-16 units, Java's `char`s.
#[derive(Debug, Clone)]
pub(crate) struct PorterStemmer {
    b: Vec<u16>,
    /// offset into b
    i: i32,
    j: i32,
    k: i32,
    k0: i32,
    dirty: bool,
}

impl Default for PorterStemmer {
    fn default() -> Self {
        Self::new()
    }
}

/// The UTF-16 unit of an ASCII character literal.
const fn unit(c: char) -> u16 {
    c as u16
}

impl PorterStemmer {
    /// Equivalent to `PorterStemmer()`.
    pub(crate) fn new() -> Self {
        Self {
            b: vec![0; INITIAL_SIZE],
            i: 0,
            j: 0,
            k: 0,
            k0: 0,
            dirty: false,
        }
    }

    /// reset() resets the stemmer so it can stem another word. If you invoke
    /// the stemmer by calling add(char) and then stem(), you must call
    /// reset() before starting another word.
    ///
    /// Equivalent to `reset()`.
    pub(crate) fn reset(&mut self) {
        self.i = 0;
        self.dirty = false;
    }

    /// Add a character to the word being stemmed. When you are finished
    /// adding characters, you can call stem(void) to process the word.
    ///
    /// Equivalent to `add(char)`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn add(&mut self, ch: u16) {
        if self.b.len() <= self.i as usize {
            self.b = ArrayUtil::grow_generic(&self.b, self.i as usize + 1);
        }
        self.b[self.i as usize] = ch;
        self.i += 1;
    }

    /// After a word has been stemmed, it can be retrieved by toString(), or a
    /// reference to the internal buffer can be retrieved by getResultBuffer
    /// and getResultLength (which is generally more efficient.)
    ///
    /// Equivalent to `toString()`, as UTF-16 units.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn to_units(&self) -> Vec<u16> {
        self.b[..self.i as usize].to_vec()
    }

    /// Returns the length of the word resulting from the stemming process.
    ///
    /// Equivalent to `getResultLength()`.
    pub(crate) fn get_result_length(&self) -> usize {
        self.i as usize
    }

    /// Returns a reference to a character buffer containing the results of
    /// the stemming process. You also need to consult getResultLength() to
    /// determine the length of the result.
    ///
    /// Equivalent to `getResultBuffer()`.
    pub(crate) fn get_result_buffer(&self) -> &[u16] {
        &self.b
    }

    /// `b[index]`; a negative index fails as Java's
    /// `ArrayIndexOutOfBoundsException` does.
    fn at(&self, index: i32) -> u16 {
        self.b[index as usize]
    }

    /// cons(i) is true <=> b\[i\] is a consonant.
    ///
    /// Equivalent to `cons(int)`.
    fn cons(&self, i: i32) -> bool {
        match char::from_u32(u32::from(self.at(i))) {
            Some('a' | 'e' | 'i' | 'o' | 'u') => false,
            Some('y') => {
                if i == self.k0 {
                    true
                } else {
                    !self.cons(i - 1)
                }
            }
            _ => true,
        }
    }

    /// m() measures the number of consonant sequences between k0 and j. if c
    /// is a consonant sequence and v a vowel sequence, and <..> indicates
    /// arbitrary presence,
    ///
    /// ```text
    ///      <c><v>       gives 0
    ///      <c>vc<v>     gives 1
    ///      <c>vcvc<v>   gives 2
    ///      <c>vcvcvc<v> gives 3
    ///      ....
    /// ```
    ///
    /// Equivalent to `m()`.
    fn m(&self) -> i32 {
        let mut n = 0;
        let mut i = self.k0;
        loop {
            if i > self.j {
                return n;
            }
            if !self.cons(i) {
                break;
            }
            i += 1;
        }
        i += 1;
        loop {
            loop {
                if i > self.j {
                    return n;
                }
                if self.cons(i) {
                    break;
                }
                i += 1;
            }
            i += 1;
            n += 1;
            loop {
                if i > self.j {
                    return n;
                }
                if !self.cons(i) {
                    break;
                }
                i += 1;
            }
            i += 1;
        }
    }

    /// vowelinstem() is true <=> k0,...j contains a vowel
    ///
    /// Equivalent to `vowelinstem()`.
    fn vowelinstem(&self) -> bool {
        let mut i = self.k0;
        while i <= self.j {
            if !self.cons(i) {
                return true;
            }
            i += 1;
        }
        false
    }

    /// doublec(j) is true <=> j,(j-1) contain a double consonant.
    ///
    /// Equivalent to `doublec(int)`.
    fn doublec(&self, j: i32) -> bool {
        if j < self.k0 + 1 {
            return false;
        }
        if self.at(j) != self.at(j - 1) {
            return false;
        }
        self.cons(j)
    }

    /// cvc(i) is true <=> i-2,i-1,i has the form consonant - vowel -
    /// consonant and also if the second c is not w,x or y. this is used when
    /// trying to restore an e at the end of a short word. e.g.
    ///
    /// ```text
    ///      cav(e), lov(e), hop(e), crim(e), but
    ///      snow, box, tray.
    /// ```
    ///
    /// Equivalent to `cvc(int)`.
    fn cvc(&self, i: i32) -> bool {
        if i < self.k0 + 2 || !self.cons(i) || self.cons(i - 1) || !self.cons(i - 2) {
            return false;
        } else {
            let ch = self.at(i);
            if ch == unit('w') || ch == unit('x') || ch == unit('y') {
                return false;
            }
        }
        true
    }

    /// Equivalent to `ends(String)`.
    fn ends(&mut self, s: &str) -> bool {
        let l = s.len() as i32;
        let o = self.k - l + 1;
        if o < self.k0 {
            return false;
        }
        for (i, c) in s.bytes().enumerate() {
            if self.at(o + i as i32) != u16::from(c) {
                return false;
            }
        }
        self.j = self.k - l;
        true
    }

    /// setto(s) sets (j+1),...k to the characters in the string s,
    /// readjusting k.
    ///
    /// Equivalent to `setto(String)`.
    fn setto(&mut self, s: &str) {
        let l = s.len() as i32;
        let o = self.j + 1;
        for (i, c) in s.bytes().enumerate() {
            self.b[(o + i as i32) as usize] = u16::from(c);
        }
        self.k = self.j + l;
        self.dirty = true;
    }

    /// r(s) is used further down.
    ///
    /// Equivalent to `r(String)`.
    fn r(&mut self, s: &str) {
        if self.m() > 0 {
            self.setto(s);
        }
    }

    /// step1() gets rid of plurals and -ed or -ing. e.g.
    ///
    /// ```text
    ///      caresses  ->  caress
    ///      ponies    ->  poni
    ///      ties      ->  ti
    ///      caress    ->  caress
    ///      cats      ->  cat
    ///
    ///      feed      ->  feed
    ///      agreed    ->  agree
    ///      disabled  ->  disable
    ///
    ///      matting   ->  mat
    ///      mating    ->  mate
    ///      meeting   ->  meet
    ///      milling   ->  mill
    ///      messing   ->  mess
    ///
    ///      meetings  ->  meet
    /// ```
    ///
    /// Equivalent to `step1()`.
    fn step1(&mut self) {
        if self.at(self.k) == unit('s') {
            if self.ends("sses") {
                self.k -= 2;
            } else if self.ends("ies") {
                self.setto("i");
            } else if self.at(self.k - 1) != unit('s') {
                self.k -= 1;
            }
        }
        if self.ends("eed") {
            if self.m() > 0 {
                self.k -= 1;
            }
        } else if (self.ends("ed") || self.ends("ing")) && self.vowelinstem() {
            self.k = self.j;
            if self.ends("at") {
                self.setto("ate");
            } else if self.ends("bl") {
                self.setto("ble");
            } else if self.ends("iz") {
                self.setto("ize");
            } else if self.doublec(self.k) {
                let ch = self.at(self.k);
                self.k -= 1;
                if ch == unit('l') || ch == unit('s') || ch == unit('z') {
                    self.k += 1;
                }
            } else if self.m() == 1 && self.cvc(self.k) {
                self.setto("e");
            }
        }
    }

    /// step2() turns terminal y to i when there is another vowel in the stem.
    ///
    /// Equivalent to `step2()`.
    fn step2(&mut self) {
        if self.ends("y") && self.vowelinstem() {
            let k = self.k as usize;
            self.b[k] = unit('i');
            self.dirty = true;
        }
    }

    /// step3() maps double suffices to single ones. so -ization ( = -ize plus
    /// -ation) maps to -ize etc. note that the string before the suffix must
    /// give m() > 0.
    ///
    /// Equivalent to `step3()`: each `case` of Java's `switch`, whose matches
    /// `break`, is an arm that stops at its first matching suffix.
    fn step3(&mut self) {
        if self.k == self.k0 {
            return; // For Bug 1
        }
        let rules: &[(&str, &str)] = match char::from_u32(u32::from(self.at(self.k - 1))) {
            Some('a') => &[("ational", "ate"), ("tional", "tion")],
            Some('c') => &[("enci", "ence"), ("anci", "ance")],
            Some('e') => &[("izer", "ize")],
            Some('l') => &[
                ("bli", "ble"),
                ("alli", "al"),
                ("entli", "ent"),
                ("eli", "e"),
                ("ousli", "ous"),
            ],
            Some('o') => &[("ization", "ize"), ("ation", "ate"), ("ator", "ate")],
            Some('s') => &[
                ("alism", "al"),
                ("iveness", "ive"),
                ("fulness", "ful"),
                ("ousness", "ous"),
            ],
            Some('t') => &[("aliti", "al"), ("iviti", "ive"), ("biliti", "ble")],
            Some('g') => &[("logi", "log")],
            _ => &[],
        };
        self.apply_first(rules);
    }

    /// The body of a `case` of `step3()` or `step4()`: `if (ends(suffix)) {
    /// r(replacement); break; }` for each rule, in order.
    fn apply_first(&mut self, rules: &[(&str, &str)]) {
        for &(suffix, replacement) in rules {
            if self.ends(suffix) {
                self.r(replacement);
                break;
            }
        }
    }

    /// step4() deals with -ic-, -full, -ness etc. similar strategy to step3.
    ///
    /// Equivalent to `step4()`.
    fn step4(&mut self) {
        let rules: &[(&str, &str)] = match char::from_u32(u32::from(self.at(self.k))) {
            Some('e') => &[("icate", "ic"), ("ative", ""), ("alize", "al")],
            Some('i') => &[("iciti", "ic")],
            Some('l') => &[("ical", "ic"), ("ful", "")],
            Some('s') => &[("ness", "")],
            _ => &[],
        };
        self.apply_first(rules);
    }

    /// step5() takes off -ant, -ence etc., in context <c>vcvc<v>.
    ///
    /// Equivalent to `step5()`.
    fn step5(&mut self) {
        if self.k == self.k0 {
            return; // for Bug 1
        }
        let found = match char::from_u32(u32::from(self.at(self.k - 1))) {
            Some('a') => self.ends("al"),
            Some('c') => self.ends("ance") || self.ends("ence"),
            Some('e') => self.ends("er"),
            Some('i') => self.ends("ic"),
            Some('l') => self.ends("able") || self.ends("ible"),
            Some('n') => {
                self.ends("ant")
                    || self.ends("ement")
                    || self.ends("ment")
                    // element etc. not stripped before the m
                    || self.ends("ent")
            }
            Some('o') => {
                // j >= 0 fixes Bug 2
                (self.ends("ion")
                    && self.j >= 0
                    && (self.at(self.j) == unit('s') || self.at(self.j) == unit('t')))
                    || self.ends("ou")
            }
            // takes care of -ous
            Some('s') => self.ends("ism"),
            Some('t') => self.ends("ate") || self.ends("iti"),
            Some('u') => self.ends("ous"),
            Some('v') => self.ends("ive"),
            Some('z') => self.ends("ize"),
            _ => false,
        };
        if !found {
            return;
        }
        if self.m() > 1 {
            self.k = self.j;
        }
    }

    /// step6() removes a final -e if m() > 1.
    ///
    /// Equivalent to `step6()`.
    fn step6(&mut self) {
        self.j = self.k;
        if self.at(self.k) == unit('e') {
            let a = self.m();
            if a > 1 || a == 1 && !self.cvc(self.k - 1) {
                self.k -= 1;
            }
        }
        if self.at(self.k) == unit('l') && self.doublec(self.k) && self.m() > 1 {
            self.k -= 1;
        }
    }

    /// Stem a word provided as a String. Returns the result as a String.
    ///
    /// Equivalent to `stem(String)`, over the UTF-16 units of the string.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn stem_string(&mut self, s: &[u16]) -> Vec<u16> {
        if self.stem_word(s, s.len()) {
            self.to_units()
        } else {
            s.to_vec()
        }
    }

    /// Stem a word contained in a char\[\]. Returns true if the stemming
    /// process resulted in a word different from the input. You can retrieve
    /// the result with getResultLength()/getResultBuffer() or toString().
    ///
    /// Equivalent to `stem(char[])`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn stem_array(&mut self, word: &[u16]) -> bool {
        self.stem_word(word, word.len())
    }

    /// Stem a word contained in a portion of a char\[\] array. Returns true
    /// if the stemming process resulted in a word different from the input.
    /// You can retrieve the result with getResultLength()/getResultBuffer()
    /// or toString().
    ///
    /// Equivalent to `stem(char[], int, int)`.
    pub(crate) fn stem_range(
        &mut self,
        word_buffer: &[u16],
        offset: usize,
        word_len: usize,
    ) -> bool {
        self.reset();
        if self.b.len() < word_len {
            self.b = vec![0; ArrayUtil::oversize(word_len, std::mem::size_of::<u16>())];
        }
        self.b[..word_len].copy_from_slice(&word_buffer[offset..offset + word_len]);
        self.i = word_len as i32;
        self.stem_from(0)
    }

    /// Stem a word contained in a leading portion of a char\[\] array.
    /// Returns true if the stemming process resulted in a word different
    /// from the input. You can retrieve the result with
    /// getResultLength()/getResultBuffer() or toString().
    ///
    /// Equivalent to `stem(char[], int)`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn stem_word(&mut self, word: &[u16], word_len: usize) -> bool {
        self.stem_range(word, 0, word_len)
    }

    /// Stem the word placed into the Stemmer buffer through calls to add().
    /// Returns true if the stemming process resulted in a word different
    /// from the input. You can retrieve the result with
    /// getResultLength()/getResultBuffer() or toString().
    ///
    /// Equivalent to `stem()`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn stem(&mut self) -> bool {
        self.stem_from(0)
    }

    /// Equivalent to `stem(int)`.
    pub(crate) fn stem_from(&mut self, i0: i32) -> bool {
        self.k = self.i - 1;
        self.k0 = i0;
        if self.k > self.k0 + 1 {
            self.step1();
            self.step2();
            self.step3();
            self.step4();
            self.step5();
            self.step6();
        }
        // Also, a word is considered dirty if we lopped off letters
        // Thanks to Ifigenia Vairelles for pointing this out.
        if self.i != self.k + 1 {
            self.dirty = true;
        }
        self.i = self.k + 1;
        self.dirty
    }
}
