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

//! Plural stemmer for Spanish, ported from
//! `org.apache.lucene.analysis.es.SpanishPluralStemmer`.

use std::sync::LazyLock;

use rucene::analysis::CharArraySet;

/// `SpanishPluralStemmer.invariantsList`.
static INVARIANTS_LIST: &[&str] = &[
    "abrebotellas",
    "abrecartas",
    "abrelatas",
    "afueras",
    "albatros",
    "albricias",
    "aledaños",
    "alexis",
    "alicates",
    "analisis",
    "andurriales",
    "antitesis",
    "añicos",
    "apendicitis",
    "apocalipsis",
    "arcoiris",
    "aries",
    "bilis",
    "boletus",
    "boris",
    "brindis",
    "cactus",
    "canutas",
    "caries",
    "cascanueces",
    "cascarrabias",
    "ciempies",
    "cifosis",
    "cortaplumas",
    "corpus",
    "cosmos",
    "cosquillas",
    "creces",
    "crisis",
    "cuatrocientas",
    "cuatrocientos",
    "cuelgacapas",
    "cuentacuentos",
    "cuentapasos",
    "cumpleaños",
    "doscientas",
    "doscientos",
    "dosis",
    "enseres",
    "entonces",
    "esponsales",
    "estatus",
    "exequias",
    "fauces",
    "forceps",
    "fotosintesis",
    "gafas",
    "gafotas",
    "gargaras",
    "gris",
    "honorarios",
    "ictus",
    "jueves",
    "lapsus",
    "lavacoches",
    "lavaplatos",
    "limpiabotas",
    "lunes",
    "maitines",
    "martes",
    "mondadientes",
    "novecientas",
    "novecientos",
    "nupcias",
    "ochocientas",
    "ochocientos",
    "pais",
    "paris",
    "parabrisas",
    "paracaidas",
    "parachoques",
    "paraguas",
    "pararrayos",
    "pisapapeles",
    "piscis",
    "portaaviones",
    "portamaletas",
    "portamantas",
    "quinientas",
    "quinientos",
    "quitamanchas",
    "recogepelotas",
    "rictus",
    "rompeolas",
    "sacacorchos",
    "sacapuntas",
    "saltamontes",
    "salvavidas",
    "seis",
    "seiscientas",
    "seiscientos",
    "setecientas",
    "setecientos",
    "sintesis",
    "tenis",
    "tifus",
    "trabalenguas",
    "vacaciones",
    "venus",
    "versus",
    "viacrucis",
    "virus",
    "viveres",
    "volandas",
];

/// `SpanishPluralStemmer.invariants`, built by the static initializer:
/// `CharArraySet.unmodifiableSet(new CharArraySet(invariantsList, true))`,
/// with Java's `assert invariants.size() == invariantsList.size()` as a
/// debug assertion.
static INVARIANTS: LazyLock<CharArraySet> = LazyLock::new(|| {
    let invariant_set = CharArraySet::from_iter(INVARIANTS_LIST.iter(), true);
    let invariants = CharArraySet::unmodifiable_set(Some(invariant_set));
    debug_assert_eq!(invariants.size(), INVARIANTS_LIST.len());
    invariants
});

/// `SpanishPluralStemmer.specialCases`, built by the static initializer:
/// `CharArraySet.unmodifiableSet(new CharArraySet(specialCasesList, true))`.
static SPECIAL_CASES: LazyLock<CharArraySet> = LazyLock::new(|| {
    let special_cases_list = [
        "yoes",
        "noes",
        "sies",
        "clubes",
        "faralaes",
        "albalaes",
        "itemes",
        "albumes",
        "sandwiches",
        "relojes",
        "bojes",
        "contrarreloj",
        "carcajes",
    ];
    let sepecial_set = CharArraySet::from_iter(special_cases_list, true);
    CharArraySet::unmodifiable_set(Some(sepecial_set))
});

/// Plural Stemmer for Spanish
///
/// This stemmer implements the rules described in:
/// *http://www.wikilengua.org/index.php/Plural_(formación)*
///
/// Equivalent to `org.apache.lucene.analysis.es.SpanishPluralStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SpanishPluralStemmer;

impl SpanishPluralStemmer {
    /// Equivalent to the implicit `SpanishPluralStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        if len < 4 {
            return len; // plural have at least 4 letters (ases,eses,etc.)
        }
        self.remove_accents(s, len);
        if self.invariant(s, len) {
            return len;
        }
        if self.special(s, len) {
            return len - 2;
        }
        if s[len - 1] == 's' as u16 {
            if !self.is_vowel(s[len - 2]) {
                // no vocals, singular words ending with consonant
                return len - 1;
            }
            if s[len - 4] == 'q' as u16
                || (s[len - 4] == 'g' as u16)
                    && s[len - 3] == 'u' as u16
                    && (s[len - 2] == 'i' as u16 || s[len - 2] == 'e' as u16)
            {
                // maniquis,caquis, parques
                return len - 1;
            }
            if self.is_vowel(s[len - 4]) && (s[len - 3] == 'r' as u16) && s[len - 2] == 'e' as u16 {
                // escaneres, alfileres, amores, cables
                return len - 2;
            }
            if self.is_vowel(s[len - 4])
                && (s[len - 3] == 'd' as u16
                    || s[len - 3] == 'l' as u16
                    || s[len - 3] == 'n' as u16
                    || s[len - 3] == 'x' as u16)
                && s[len - 2] == 'e' as u16
            {
                // abades, comerciales, faxes,  relojes,
                return len - 2;
            }
            if (s[len - 3] == 'y' as u16 || s[len - 3] == 'u' as u16) && s[len - 2] == 'e' as u16 {
                // bambues,leyes
                return len - 2;
            }
            if (s[len - 4] == 'u' as u16
                || s[len - 4] == 'l' as u16
                || s[len - 4] == 'r' as u16
                || s[len - 4] == 't' as u16
                || s[len - 4] == 'n' as u16)
                && (s[len - 3] == 'i' as u16)
                && s[len - 2] == 'e' as u16
            {
                // jabalies,israelies, maniquies
                return len - 2;
            }
            if s[len - 3] == 's' as u16 && s[len - 2] == 'e' as u16 {
                // reses
                return len - 2;
            }
            if self.is_vowel(s[len - 3]) && s[len - 2] == 'i' as u16 {
                // jerseis
                s[len - 2] = 'y' as u16;
                return len - 1;
            }
            if s[len - 3] == 'd' as u16 && s[len - 2] == 'i' as u16 {
                // brandis
                s[len - 2] = 'y' as u16;
                return len - 1;
            }
            if s[len - 2] == 'e' as u16 && s[len - 3] == 'c' as u16 {
                // voces-->voz
                s[len - 3] = 'z' as u16;
                return len - 2;
            }
            if self.is_vowel(s[len - 2]) {
                // remove last 's': jabalís, casas, coches, etc.
                return len - 1;
            }
        }
        len
    }

    /// Equivalent to `isVowel(char)`.
    fn is_vowel(&self, c: u16) -> bool {
        let mut res = false;
        if c == 'a' as u16
            || c == 'e' as u16
            || c == 'i' as u16
            || c == 'o' as u16
            || c == 'u' as u16
        {
            res = true;
        }
        res
    }

    /// Equivalent to `invariant(char[], int)`.
    fn invariant(&self, s: &[u16], len: usize) -> bool {
        INVARIANTS.contains_chars(Some(s), 0, len)
    }

    /// Equivalent to `special(char[], int)`.
    fn special(&self, s: &[u16], len: usize) -> bool {
        SPECIAL_CASES.contains_chars(Some(s), 0, len)
    }

    /// Equivalent to `removeAccents(char[], int)`.
    fn remove_accents(&self, s: &mut [u16], len: usize) {
        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('à' | 'á' | 'â' | 'ä') => *c = 'a' as u16,
                Some('ò' | 'ó' | 'ô' | 'ö') => *c = 'o' as u16,
                Some('è' | 'é' | 'ê' | 'ë') => *c = 'e' as u16,
                Some('ù' | 'ú' | 'û' | 'ü') => *c = 'u' as u16,
                Some('ì' | 'í' | 'î' | 'ï') => *c = 'i' as u16,
                _ => {}
            }
        }
    }
}
