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

//! A stemmer for Brazilian Portuguese words, ported from
//! `org.apache.lucene.analysis.br.BrazilianStemmer`.

use std::sync::LazyLock;

use crate::jdk::character::is_letter;
use crate::jdk::locale::Locale;
use crate::jdk::string::{ends_with_str, to_lower_case, to_rust_string};

/// `BrazilianStemmer.locale`, `new Locale.Builder().setLanguageTag("pt-BR").build()`.
static LOCALE: LazyLock<Locale> = LazyLock::new(|| {
    Locale::builder_from_language_tag("pt-BR").expect("INVARIANT: pt-BR is a well-formed tag")
});

/// A stemmer for Brazilian Portuguese words.
///
/// Equivalent to `org.apache.lucene.analysis.br.BrazilianStemmer`. Its
/// `String` fields, which may be null, are `Option`s of UTF-16 units.
#[derive(Debug, Default, Clone)]
pub(crate) struct BrazilianStemmer {
    /// Changed term
    term: Option<Vec<u16>>,
    ct: Option<Vec<u16>>,
    r1: Option<Vec<u16>>,
    r2: Option<Vec<u16>>,
    rv: Option<Vec<u16>>,
}

impl BrazilianStemmer {
    /// Equivalent to `BrazilianStemmer()`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Stems the given term to an unique `discriminator`.
    ///
    /// * `term`: The term that should be stemmed.
    /// * returns: Discriminator for `term`
    ///
    /// Equivalent to `stem(String)`: `None` is Java's `null`, for a term
    /// that cannot be indexed.
    pub(crate) fn stem(&mut self, term: &[u16]) -> Option<Vec<u16>> {
        // altered the term
        let mut altered;

        // creates CT
        self.create_ct(term);

        let ct = self.ct.as_deref().expect("INVARIANT: createCT sets CT");
        if !self.is_indexable(ct) {
            return None;
        }
        if !self.is_stemmable(ct) {
            return self.ct.clone();
        }

        self.r1 = self.get_r1(self.ct.as_deref());
        self.r2 = self.get_r1(self.r1.as_deref());
        self.rv = self.get_rv(self.ct.as_deref());
        let mut changed = term.to_vec();
        changed.push(u16::from(b';'));
        changed.extend_from_slice(self.ct.as_deref().expect("INVARIANT: createCT sets CT"));
        self.term = Some(changed);

        altered = self.step1();
        if !altered {
            altered = self.step2();
        }

        if altered {
            self.step3();
        } else {
            self.step4();
        }

        self.step5();

        self.ct.clone()
    }

    /// Checks a term if it can be processed correctly: true if, and only
    /// if, the given term consists in letters.
    ///
    /// Equivalent to `isStemmable(String)`.
    fn is_stemmable(&self, term: &[u16]) -> bool {
        for &c in term {
            // Discard terms that contain non-letter characters.
            if !is_letter(i32::from(c)) {
                return false;
            }
        }
        true
    }

    /// Checks a term if it can be processed indexed: true if it can be
    /// indexed.
    ///
    /// Equivalent to `isIndexable(String)`.
    fn is_indexable(&self, term: &[u16]) -> bool {
        (term.len() < 30) && (term.len() > 2)
    }

    /// See if string is 'a','e','i','o','u': true if is vowel.
    ///
    /// Equivalent to `isVowel(char)`.
    fn is_vowel(&self, value: u16) -> bool {
        (value == u16::from(b'a'))
            || (value == u16::from(b'e'))
            || (value == u16::from(b'i'))
            || (value == u16::from(b'o'))
            || (value == u16::from(b'u'))
    }

    /// Gets R1
    ///
    /// R1 - is the region after the first non-vowel following a vowel, or is
    /// the null region at the end of the word if there is no such
    /// non-vowel.
    ///
    /// * returns: null or a string representing R1
    ///
    /// Equivalent to `getR1(String)`.
    fn get_r1(&self, value: Option<&[u16]>) -> Option<Vec<u16>> {
        // be-safe !!!
        let value = value?;

        // find 1st vowel
        let i = value.len() as i64 - 1;
        let mut j: i64 = 0;
        while j < i {
            if self.is_vowel(value[j as usize]) {
                break;
            }
            j += 1;
        }

        if !(j < i) {
            return None;
        }

        // find 1st non-vowel
        while j < i {
            if !(self.is_vowel(value[j as usize])) {
                break;
            }
            j += 1;
        }

        if !(j < i) {
            return None;
        }

        Some(value[(j + 1) as usize..].to_vec())
    }

    /// Gets RV
    ///
    /// RV - IF the second letter is a consonant, RV is the region after the
    /// next following vowel,
    ///
    /// OR if the first two letters are vowels, RV is the region after the
    /// next consonant,
    ///
    /// AND otherwise (consonant-vowel case) RV is the region after the third
    /// letter.
    ///
    /// BUT RV is the end of the word if this positions cannot be found.
    ///
    /// * returns: null or a string representing RV
    ///
    /// Equivalent to `getRV(String)`.
    fn get_rv(&self, value: Option<&[u16]>) -> Option<Vec<u16>> {
        // be-safe !!!
        let value = value?;

        let i = value.len() as i64 - 1;

        // RV - IF the second letter is a consonant, RV is the region after
        //      the next following vowel,
        if (i > 0) && !self.is_vowel(value[1]) {
            // find 1st vowel
            let mut j: i64 = 2;
            while j < i {
                if self.is_vowel(value[j as usize]) {
                    break;
                }
                j += 1;
            }

            if j < i {
                return Some(value[(j + 1) as usize..].to_vec());
            }
        }

        // RV - OR if the first two letters are vowels, RV is the region
        //      after the next consonant,
        if (i > 1) && self.is_vowel(value[0]) && self.is_vowel(value[1]) {
            // find 1st consoant
            let mut j: i64 = 2;
            while j < i {
                if !self.is_vowel(value[j as usize]) {
                    break;
                }
                j += 1;
            }

            if j < i {
                return Some(value[(j + 1) as usize..].to_vec());
            }
        }

        // RV - AND otherwise (consonant-vowel case) RV is the region after
        //      the third letter.
        if i > 2 {
            return Some(value[3..].to_vec());
        }

        None
    }

    /// 1) Turn to lowercase 2) Remove accents 3) ã -> a ; õ -> o 4) ç -> c
    ///
    /// * returns: null or a string transformed
    ///
    /// Equivalent to `changeTerm(String)`.
    fn change_term(&self, value: Option<&[u16]>) -> Option<Vec<u16>> {
        let mut r = Vec::new();

        // be-safe !!!
        let value = value?;

        let value = to_lower_case(value, &LOCALE);
        for &c in &value {
            match char::from_u32(u32::from(c)) {
                Some('á' | 'â' | 'ã') => r.push(u16::from(b'a')),
                Some('é' | 'ê') => r.push(u16::from(b'e')),
                Some('í') => r.push(u16::from(b'i')),
                Some('ó' | 'ô' | 'õ') => r.push(u16::from(b'o')),
                Some('ú' | 'ü') => r.push(u16::from(b'u')),
                Some('ç') => r.push(u16::from(b'c')),
                Some('ñ') => r.push(u16::from(b'n')),
                _ => r.push(c),
            }
        }

        Some(r)
    }

    /// Check if a string ends with a suffix: true if the string ends with
    /// the specified suffix.
    ///
    /// Equivalent to `suffix(String, String)`; the suffixes are literals,
    /// never null.
    fn suffix(value: Option<&[u16]>, suffix: &str) -> bool {
        // be-safe !!!
        let Some(value) = value else {
            return false;
        };

        if suffix.encode_utf16().count() > value.len() {
            return false;
        }

        ends_with_str(value, suffix)
    }

    /// Replace a string suffix by another: the replaced String.
    ///
    /// Equivalent to `replaceSuffix(String, String, String)`.
    fn replace_suffix(
        value: Option<Vec<u16>>,
        to_replace: &str,
        change_to: &str,
    ) -> Option<Vec<u16>> {
        // be-safe !!!
        let value = value?;

        let mut vvalue = Self::remove_suffix(Some(value.clone()), to_replace)
            .expect("INVARIANT: removeSuffix keeps a non-null value");

        if value == vvalue {
            Some(value)
        } else {
            vvalue.extend(change_to.encode_utf16());
            Some(vvalue)
        }
    }

    /// Remove a string suffix: the String without the suffix.
    ///
    /// Equivalent to `removeSuffix(String, String)`.
    fn remove_suffix(value: Option<Vec<u16>>, to_remove: &str) -> Option<Vec<u16>> {
        // be-safe !!!
        let mut value = value?;
        if !Self::suffix(Some(&value), to_remove) {
            return Some(value);
        }

        value.truncate(value.len() - to_remove.encode_utf16().count());
        Some(value)
    }

    /// See if a suffix is preceded by a String: true if the suffix is
    /// preceded.
    ///
    /// Equivalent to `suffixPreceded(String, String, String)`.
    fn suffix_preceded(value: Option<&[u16]>, suffix: &str, preceded: &str) -> bool {
        // be-safe !!!
        let Some(value) = value else {
            return false;
        };
        if !Self::suffix(Some(value), suffix) {
            return false;
        }

        Self::suffix(
            Self::remove_suffix(Some(value.to_vec()), suffix).as_deref(),
            preceded,
        )
    }

    /// The length of `CT` or `RV`, which the steps read only once they know
    /// it is not null.
    fn length(value: &Option<Vec<u16>>) -> usize {
        value
            .as_ref()
            .expect("INVARIANT: the steps read the length of a non-null string")
            .len()
    }

    /// Creates CT (changed term) , substituting * 'ã' and 'õ' for 'a~' and
    /// 'o~'.
    ///
    /// Equivalent to `createCT(String)`.
    fn create_ct(&mut self, term: &[u16]) {
        let mut ct = self
            .change_term(Some(term))
            .expect("INVARIANT: changeTerm of a non-null term is not null");

        if ct.len() < 2 {
            self.ct = Some(ct);
            return;
        }

        // if the first character is ... , remove it
        if matches!(
            char::from_u32(u32::from(ct[0])),
            Some('"' | '\'' | '-' | ',' | ';' | '.' | '?' | '!')
        ) {
            ct.remove(0);
        }

        if ct.len() < 2 {
            self.ct = Some(ct);
            return;
        }

        // if the last character is ... , remove it
        if matches!(
            char::from_u32(u32::from(ct[ct.len() - 1])),
            Some('-' | ',' | ';' | '.' | '?' | '!' | '\'' | '"')
        ) {
            ct.pop();
        }
        self.ct = Some(ct);
    }

    /// Standard suffix removal. Search for the longest among the following
    /// suffixes, and perform the following actions:
    ///
    /// * returns: false if no ending was removed
    ///
    /// Equivalent to `step1()`.
    fn step1(&mut self) -> bool {
        if self.ct.is_none() {
            return false;
        }

        // suffix length = 7
        if Self::suffix(self.ct.as_deref(), "uciones")
            && Self::suffix(self.r2.as_deref(), "uciones")
        {
            self.ct = Self::replace_suffix(self.ct.take(), "uciones", "u");
            return true;
        }

        // suffix length = 6
        if Self::length(&self.ct) >= 6 {
            if Self::suffix(self.ct.as_deref(), "imentos")
                && Self::suffix(self.r2.as_deref(), "imentos")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "imentos");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "amentos")
                && Self::suffix(self.r2.as_deref(), "amentos")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "amentos");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "adores")
                && Self::suffix(self.r2.as_deref(), "adores")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "adores");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "adoras")
                && Self::suffix(self.r2.as_deref(), "adoras")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "adoras");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "logias")
                && Self::suffix(self.r2.as_deref(), "logias")
            {
                // Java discards the result: CT keeps its value.
                let _ = Self::replace_suffix(self.ct.clone(), "logias", "log");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "encias")
                && Self::suffix(self.r2.as_deref(), "encias")
            {
                self.ct = Self::replace_suffix(self.ct.take(), "encias", "ente");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "amente")
                && Self::suffix(self.r1.as_deref(), "amente")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "amente");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "idades")
                && Self::suffix(self.r2.as_deref(), "idades")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "idades");
                return true;
            }
        }

        // suffix length = 5
        if Self::length(&self.ct) >= 5 {
            if Self::suffix(self.ct.as_deref(), "acoes")
                && Self::suffix(self.r2.as_deref(), "acoes")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "acoes");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "imento")
                && Self::suffix(self.r2.as_deref(), "imento")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "imento");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "amento")
                && Self::suffix(self.r2.as_deref(), "amento")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "amento");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "adora")
                && Self::suffix(self.r2.as_deref(), "adora")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "adora");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ismos")
                && Self::suffix(self.r2.as_deref(), "ismos")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ismos");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "istas")
                && Self::suffix(self.r2.as_deref(), "istas")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "istas");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "logia")
                && Self::suffix(self.r2.as_deref(), "logia")
            {
                self.ct = Self::replace_suffix(self.ct.take(), "logia", "log");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ucion")
                && Self::suffix(self.r2.as_deref(), "ucion")
            {
                self.ct = Self::replace_suffix(self.ct.take(), "ucion", "u");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "encia")
                && Self::suffix(self.r2.as_deref(), "encia")
            {
                self.ct = Self::replace_suffix(self.ct.take(), "encia", "ente");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "mente")
                && Self::suffix(self.r2.as_deref(), "mente")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "mente");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "idade")
                && Self::suffix(self.r2.as_deref(), "idade")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "idade");
                return true;
            }
        }

        // suffix length = 4
        if Self::length(&self.ct) >= 4 {
            if Self::suffix(self.ct.as_deref(), "acao") && Self::suffix(self.r2.as_deref(), "acao")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "acao");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ezas") && Self::suffix(self.r2.as_deref(), "ezas")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ezas");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "icos") && Self::suffix(self.r2.as_deref(), "icos")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "icos");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "icas") && Self::suffix(self.r2.as_deref(), "icas")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "icas");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ismo") && Self::suffix(self.r2.as_deref(), "ismo")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ismo");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "avel") && Self::suffix(self.r2.as_deref(), "avel")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "avel");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ivel") && Self::suffix(self.r2.as_deref(), "ivel")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ivel");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ista") && Self::suffix(self.r2.as_deref(), "ista")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ista");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "osos") && Self::suffix(self.r2.as_deref(), "osos")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "osos");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "osas") && Self::suffix(self.r2.as_deref(), "osas")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "osas");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ador") && Self::suffix(self.r2.as_deref(), "ador")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ador");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ivas") && Self::suffix(self.r2.as_deref(), "ivas")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ivas");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ivos") && Self::suffix(self.r2.as_deref(), "ivos")
            {
                self.ct = Self::remove_suffix(self.ct.take(), "ivos");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "iras")
                && Self::suffix(self.rv.as_deref(), "iras")
                && Self::suffix_preceded(self.ct.as_deref(), "iras", "e")
            {
                self.ct = Self::replace_suffix(self.ct.take(), "iras", "ir");
                return true;
            }
        }

        // suffix length = 3
        if Self::length(&self.ct) >= 3 {
            if Self::suffix(self.ct.as_deref(), "eza") && Self::suffix(self.r2.as_deref(), "eza") {
                self.ct = Self::remove_suffix(self.ct.take(), "eza");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ico") && Self::suffix(self.r2.as_deref(), "ico") {
                self.ct = Self::remove_suffix(self.ct.take(), "ico");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ica") && Self::suffix(self.r2.as_deref(), "ica") {
                self.ct = Self::remove_suffix(self.ct.take(), "ica");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "oso") && Self::suffix(self.r2.as_deref(), "oso") {
                self.ct = Self::remove_suffix(self.ct.take(), "oso");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "osa") && Self::suffix(self.r2.as_deref(), "osa") {
                self.ct = Self::remove_suffix(self.ct.take(), "osa");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "iva") && Self::suffix(self.r2.as_deref(), "iva") {
                self.ct = Self::remove_suffix(self.ct.take(), "iva");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ivo") && Self::suffix(self.r2.as_deref(), "ivo") {
                self.ct = Self::remove_suffix(self.ct.take(), "ivo");
                return true;
            }
            if Self::suffix(self.ct.as_deref(), "ira")
                && Self::suffix(self.rv.as_deref(), "ira")
                && Self::suffix_preceded(self.ct.as_deref(), "ira", "e")
            {
                self.ct = Self::replace_suffix(self.ct.take(), "ira", "ir");
                return true;
            }
        }

        // no ending was removed by step1
        false
    }

    /// Verb suffixes.
    ///
    /// Search for the longest among the following suffixes in RV, and if
    /// found, delete.
    ///
    /// * returns: false if no ending was removed
    ///
    /// Equivalent to `step2()`.
    fn step2(&mut self) -> bool {
        if self.rv.is_none() {
            return false;
        }

        // suffix lenght = 7
        if Self::length(&self.rv) >= 7 {
            if Self::suffix(self.rv.as_deref(), "issemos") {
                self.ct = Self::remove_suffix(self.ct.take(), "issemos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "essemos") {
                self.ct = Self::remove_suffix(self.ct.take(), "essemos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "assemos") {
                self.ct = Self::remove_suffix(self.ct.take(), "assemos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ariamos") {
                self.ct = Self::remove_suffix(self.ct.take(), "ariamos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eriamos") {
                self.ct = Self::remove_suffix(self.ct.take(), "eriamos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iriamos") {
                self.ct = Self::remove_suffix(self.ct.take(), "iriamos");
                return true;
            }
        }

        // suffix length = 6
        if Self::length(&self.rv) >= 6 {
            if Self::suffix(self.rv.as_deref(), "iremos") {
                self.ct = Self::remove_suffix(self.ct.take(), "iremos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eremos") {
                self.ct = Self::remove_suffix(self.ct.take(), "eremos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "aremos") {
                self.ct = Self::remove_suffix(self.ct.take(), "aremos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "avamos") {
                self.ct = Self::remove_suffix(self.ct.take(), "avamos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iramos") {
                self.ct = Self::remove_suffix(self.ct.take(), "iramos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eramos") {
                self.ct = Self::remove_suffix(self.ct.take(), "eramos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "aramos") {
                self.ct = Self::remove_suffix(self.ct.take(), "aramos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "asseis") {
                self.ct = Self::remove_suffix(self.ct.take(), "asseis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "esseis") {
                self.ct = Self::remove_suffix(self.ct.take(), "esseis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "isseis") {
                self.ct = Self::remove_suffix(self.ct.take(), "isseis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "arieis") {
                self.ct = Self::remove_suffix(self.ct.take(), "arieis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "erieis") {
                self.ct = Self::remove_suffix(self.ct.take(), "erieis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "irieis") {
                self.ct = Self::remove_suffix(self.ct.take(), "irieis");
                return true;
            }
        }

        // suffix length = 5
        if Self::length(&self.rv) >= 5 {
            if Self::suffix(self.rv.as_deref(), "irmos") {
                self.ct = Self::remove_suffix(self.ct.take(), "irmos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iamos") {
                self.ct = Self::remove_suffix(self.ct.take(), "iamos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "armos") {
                self.ct = Self::remove_suffix(self.ct.take(), "armos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ermos") {
                self.ct = Self::remove_suffix(self.ct.take(), "ermos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "areis") {
                self.ct = Self::remove_suffix(self.ct.take(), "areis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ereis") {
                self.ct = Self::remove_suffix(self.ct.take(), "ereis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ireis") {
                self.ct = Self::remove_suffix(self.ct.take(), "ireis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "asses") {
                self.ct = Self::remove_suffix(self.ct.take(), "asses");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "esses") {
                self.ct = Self::remove_suffix(self.ct.take(), "esses");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "isses") {
                self.ct = Self::remove_suffix(self.ct.take(), "isses");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "astes") {
                self.ct = Self::remove_suffix(self.ct.take(), "astes");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "assem") {
                self.ct = Self::remove_suffix(self.ct.take(), "assem");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "essem") {
                self.ct = Self::remove_suffix(self.ct.take(), "essem");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "issem") {
                self.ct = Self::remove_suffix(self.ct.take(), "issem");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ardes") {
                self.ct = Self::remove_suffix(self.ct.take(), "ardes");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "erdes") {
                self.ct = Self::remove_suffix(self.ct.take(), "erdes");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "irdes") {
                self.ct = Self::remove_suffix(self.ct.take(), "irdes");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ariam") {
                self.ct = Self::remove_suffix(self.ct.take(), "ariam");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eriam") {
                self.ct = Self::remove_suffix(self.ct.take(), "eriam");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iriam") {
                self.ct = Self::remove_suffix(self.ct.take(), "iriam");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "arias") {
                self.ct = Self::remove_suffix(self.ct.take(), "arias");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "erias") {
                self.ct = Self::remove_suffix(self.ct.take(), "erias");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "irias") {
                self.ct = Self::remove_suffix(self.ct.take(), "irias");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "estes") {
                self.ct = Self::remove_suffix(self.ct.take(), "estes");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "istes") {
                self.ct = Self::remove_suffix(self.ct.take(), "istes");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "areis") {
                self.ct = Self::remove_suffix(self.ct.take(), "areis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "aveis") {
                self.ct = Self::remove_suffix(self.ct.take(), "aveis");
                return true;
            }
        }

        // suffix length = 4
        if Self::length(&self.rv) >= 4 {
            if Self::suffix(self.rv.as_deref(), "aria") {
                self.ct = Self::remove_suffix(self.ct.take(), "aria");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eria") {
                self.ct = Self::remove_suffix(self.ct.take(), "eria");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iria") {
                self.ct = Self::remove_suffix(self.ct.take(), "iria");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "asse") {
                self.ct = Self::remove_suffix(self.ct.take(), "asse");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "esse") {
                self.ct = Self::remove_suffix(self.ct.take(), "esse");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "isse") {
                self.ct = Self::remove_suffix(self.ct.take(), "isse");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "aste") {
                self.ct = Self::remove_suffix(self.ct.take(), "aste");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "este") {
                self.ct = Self::remove_suffix(self.ct.take(), "este");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iste") {
                self.ct = Self::remove_suffix(self.ct.take(), "iste");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "arei") {
                self.ct = Self::remove_suffix(self.ct.take(), "arei");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "erei") {
                self.ct = Self::remove_suffix(self.ct.take(), "erei");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "irei") {
                self.ct = Self::remove_suffix(self.ct.take(), "irei");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "aram") {
                self.ct = Self::remove_suffix(self.ct.take(), "aram");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eram") {
                self.ct = Self::remove_suffix(self.ct.take(), "eram");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iram") {
                self.ct = Self::remove_suffix(self.ct.take(), "iram");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "avam") {
                self.ct = Self::remove_suffix(self.ct.take(), "avam");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "arem") {
                self.ct = Self::remove_suffix(self.ct.take(), "arem");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "erem") {
                self.ct = Self::remove_suffix(self.ct.take(), "erem");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "irem") {
                self.ct = Self::remove_suffix(self.ct.take(), "irem");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ando") {
                self.ct = Self::remove_suffix(self.ct.take(), "ando");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "endo") {
                self.ct = Self::remove_suffix(self.ct.take(), "endo");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "indo") {
                self.ct = Self::remove_suffix(self.ct.take(), "indo");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "arao") {
                self.ct = Self::remove_suffix(self.ct.take(), "arao");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "erao") {
                self.ct = Self::remove_suffix(self.ct.take(), "erao");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "irao") {
                self.ct = Self::remove_suffix(self.ct.take(), "irao");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "adas") {
                self.ct = Self::remove_suffix(self.ct.take(), "adas");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "idas") {
                self.ct = Self::remove_suffix(self.ct.take(), "idas");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "aras") {
                self.ct = Self::remove_suffix(self.ct.take(), "aras");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eras") {
                self.ct = Self::remove_suffix(self.ct.take(), "eras");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iras") {
                self.ct = Self::remove_suffix(self.ct.take(), "iras");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "avas") {
                self.ct = Self::remove_suffix(self.ct.take(), "avas");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ares") {
                self.ct = Self::remove_suffix(self.ct.take(), "ares");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eres") {
                self.ct = Self::remove_suffix(self.ct.take(), "eres");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ires") {
                self.ct = Self::remove_suffix(self.ct.take(), "ires");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ados") {
                self.ct = Self::remove_suffix(self.ct.take(), "ados");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "idos") {
                self.ct = Self::remove_suffix(self.ct.take(), "idos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "amos") {
                self.ct = Self::remove_suffix(self.ct.take(), "amos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "emos") {
                self.ct = Self::remove_suffix(self.ct.take(), "emos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "imos") {
                self.ct = Self::remove_suffix(self.ct.take(), "imos");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iras") {
                self.ct = Self::remove_suffix(self.ct.take(), "iras");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ieis") {
                self.ct = Self::remove_suffix(self.ct.take(), "ieis");
                return true;
            }
        }

        // suffix length = 3
        if Self::length(&self.rv) >= 3 {
            if Self::suffix(self.rv.as_deref(), "ada") {
                self.ct = Self::remove_suffix(self.ct.take(), "ada");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ida") {
                self.ct = Self::remove_suffix(self.ct.take(), "ida");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ara") {
                self.ct = Self::remove_suffix(self.ct.take(), "ara");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "era") {
                self.ct = Self::remove_suffix(self.ct.take(), "era");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ira") {
                self.ct = Self::remove_suffix(self.ct.take(), "ava");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iam") {
                self.ct = Self::remove_suffix(self.ct.take(), "iam");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ado") {
                self.ct = Self::remove_suffix(self.ct.take(), "ado");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ido") {
                self.ct = Self::remove_suffix(self.ct.take(), "ido");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ias") {
                self.ct = Self::remove_suffix(self.ct.take(), "ias");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ais") {
                self.ct = Self::remove_suffix(self.ct.take(), "ais");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eis") {
                self.ct = Self::remove_suffix(self.ct.take(), "eis");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ira") {
                self.ct = Self::remove_suffix(self.ct.take(), "ira");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ear") {
                self.ct = Self::remove_suffix(self.ct.take(), "ear");
                return true;
            }
        }

        // suffix length = 2
        if Self::length(&self.rv) >= 2 {
            if Self::suffix(self.rv.as_deref(), "ia") {
                self.ct = Self::remove_suffix(self.ct.take(), "ia");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ei") {
                self.ct = Self::remove_suffix(self.ct.take(), "ei");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "am") {
                self.ct = Self::remove_suffix(self.ct.take(), "am");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "em") {
                self.ct = Self::remove_suffix(self.ct.take(), "em");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ar") {
                self.ct = Self::remove_suffix(self.ct.take(), "ar");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "er") {
                self.ct = Self::remove_suffix(self.ct.take(), "er");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ir") {
                self.ct = Self::remove_suffix(self.ct.take(), "ir");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "as") {
                self.ct = Self::remove_suffix(self.ct.take(), "as");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "es") {
                self.ct = Self::remove_suffix(self.ct.take(), "es");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "is") {
                self.ct = Self::remove_suffix(self.ct.take(), "is");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "eu") {
                self.ct = Self::remove_suffix(self.ct.take(), "eu");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iu") {
                self.ct = Self::remove_suffix(self.ct.take(), "iu");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "iu") {
                self.ct = Self::remove_suffix(self.ct.take(), "iu");
                return true;
            }
            if Self::suffix(self.rv.as_deref(), "ou") {
                self.ct = Self::remove_suffix(self.ct.take(), "ou");
                return true;
            }
        }

        // no ending was removed by step2
        false
    }

    /// Delete suffix 'i' if in RV and preceded by 'c'
    ///
    /// Equivalent to `step3()`.
    fn step3(&mut self) {
        if self.rv.is_none() {
            return;
        }

        if Self::suffix(self.rv.as_deref(), "i")
            && Self::suffix_preceded(self.rv.as_deref(), "i", "c")
        {
            self.ct = Self::remove_suffix(self.ct.take(), "i");
        }
    }

    /// Residual suffix
    ///
    /// If the word ends with one of the suffixes (os a i o á í ó) in RV,
    /// delete it
    ///
    /// Equivalent to `step4()`.
    #[allow(clippy::needless_return)]
    fn step4(&mut self) {
        if self.rv.is_none() {
            return;
        }

        if Self::suffix(self.rv.as_deref(), "os") {
            self.ct = Self::remove_suffix(self.ct.take(), "os");
            return;
        }
        if Self::suffix(self.rv.as_deref(), "a") {
            self.ct = Self::remove_suffix(self.ct.take(), "a");
            return;
        }
        if Self::suffix(self.rv.as_deref(), "i") {
            self.ct = Self::remove_suffix(self.ct.take(), "i");
            return;
        }
        if Self::suffix(self.rv.as_deref(), "o") {
            self.ct = Self::remove_suffix(self.ct.take(), "o");
            return;
        }
    }

    /// If the word ends with one of ( e é ê) in RV,delete it, and if
    /// preceded by 'gu' (or 'ci') with the 'u' (or 'i') in RV, delete the
    /// 'u' (or 'i')
    ///
    /// Or if the word ends ç remove the cedilha
    ///
    /// Equivalent to `step5()`.
    #[allow(clippy::needless_return)]
    fn step5(&mut self) {
        if self.rv.is_none() {
            return;
        }

        if Self::suffix(self.rv.as_deref(), "e") {
            if Self::suffix_preceded(self.rv.as_deref(), "e", "gu") {
                self.ct = Self::remove_suffix(self.ct.take(), "e");
                self.ct = Self::remove_suffix(self.ct.take(), "u");
                return;
            }

            if Self::suffix_preceded(self.rv.as_deref(), "e", "ci") {
                self.ct = Self::remove_suffix(self.ct.take(), "e");
                self.ct = Self::remove_suffix(self.ct.take(), "i");
                return;
            }

            self.ct = Self::remove_suffix(self.ct.take(), "e");
            return;
        }
    }

    /// For log and debug purpose
    ///
    /// * returns: TERM, CT, RV, R1 and R2
    ///
    /// Equivalent to `log()`; a null field prints `null`, as Java's string
    /// concatenation does.
    pub(crate) fn log(&self) -> String {
        let text = |value: &Option<Vec<u16>>| match value {
            Some(value) => to_rust_string(value),
            None => "null".to_string(),
        };
        format!(
            " (TERM = {}) (CT = {}) (RV = {}) (R1 = {}) (R2 = {})",
            text(&self.term),
            text(&self.ct),
            text(&self.rv),
            text(&self.r1),
            text(&self.r2)
        )
    }
}
