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

//! A stemmer for Greek words, ported from
//! `org.apache.lucene.analysis.el.GreekStemmer`.

use std::sync::LazyLock;

use rucene::analysis::CharArraySet;

/// `GreekStemmer.exc4`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC4: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(["θ", "δ", "ελ", "γαλ", "ν", "π", "ιδ", "παρ"], false)
});

/// `GreekStemmer.exc6`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC6: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "αλ",
            "αδ",
            "ενδ",
            "αμαν",
            "αμμοχαλ",
            "ηθ",
            "ανηθ",
            "αντιδ",
            "φυσ",
            "βρωμ",
            "γερ",
            "εξωδ",
            "καλπ",
            "καλλιν",
            "καταδ",
            "μουλ",
            "μπαν",
            "μπαγιατ",
            "μπολ",
            "μποσ",
            "νιτ",
            "ξικ",
            "συνομηλ",
            "πετσ",
            "πιτσ",
            "πικαντ",
            "πλιατσ",
            "ποστελν",
            "πρωτοδ",
            "σερτ",
            "συναδ",
            "τσαμ",
            "υποδ",
            "φιλον",
            "φυλοδ",
            "χασ",
        ],
        false,
    )
});

/// `GreekStemmer.exc7`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC7: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "αναπ",
            "αποθ",
            "αποκ",
            "αποστ",
            "βουβ",
            "ξεθ",
            "ουλ",
            "πεθ",
            "πικρ",
            "ποτ",
            "σιχ",
            "χ",
        ],
        false,
    )
});

/// `GreekStemmer.exc8a`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC8A: LazyLock<CharArraySet> =
    LazyLock::new(|| CharArraySet::from_iter(["τρ", "τσ"], false));

/// `GreekStemmer.exc8b`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC8B: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "βετερ",
            "βουλκ",
            "βραχμ",
            "γ",
            "δραδουμ",
            "θ",
            "καλπουζ",
            "καστελ",
            "κορμορ",
            "λαοπλ",
            "μωαμεθ",
            "μ",
            "μουσουλμ",
            "ν",
            "ουλ",
            "π",
            "πελεκ",
            "πλ",
            "πολισ",
            "πορτολ",
            "σαρακατσ",
            "σουλτ",
            "τσαρλατ",
            "ορφ",
            "τσιγγ",
            "τσοπ",
            "φωτοστεφ",
            "χ",
            "ψυχοπλ",
            "αγ",
            "ορφ",
            "γαλ",
            "γερ",
            "δεκ",
            "διπλ",
            "αμερικαν",
            "ουρ",
            "πιθ",
            "πουριτ",
            "σ",
            "ζωντ",
            "ικ",
            "καστ",
            "κοπ",
            "λιχ",
            "λουθηρ",
            "μαιντ",
            "μελ",
            "σιγ",
            "σπ",
            "στεγ",
            "τραγ",
            "τσαγ",
            "φ",
            "ερ",
            "αδαπ",
            "αθιγγ",
            "αμηχ",
            "ανικ",
            "ανοργ",
            "απηγ",
            "απιθ",
            "ατσιγγ",
            "βασ",
            "βασκ",
            "βαθυγαλ",
            "βιομηχ",
            "βραχυκ",
            "διατ",
            "διαφ",
            "ενοργ",
            "θυσ",
            "καπνοβιομηχ",
            "καταγαλ",
            "κλιβ",
            "κοιλαρφ",
            "λιβ",
            "μεγλοβιομηχ",
            "μικροβιομηχ",
            "νταβ",
            "ξηροκλιβ",
            "ολιγοδαμ",
            "ολογαλ",
            "πενταρφ",
            "περηφ",
            "περιτρ",
            "πλατ",
            "πολυδαπ",
            "πολυμηχ",
            "στεφ",
            "ταβ",
            "τετ",
            "υπερηφ",
            "υποκοπ",
            "χαμηλοδαπ",
            "ψηλοταβ",
        ],
        false,
    )
});

/// `GreekStemmer.exc9`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC9: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "αβαρ",
            "βεν",
            "εναρ",
            "αβρ",
            "αδ",
            "αθ",
            "αν",
            "απλ",
            "βαρον",
            "ντρ",
            "σκ",
            "κοπ",
            "μπορ",
            "νιφ",
            "παγ",
            "παρακαλ",
            "σερπ",
            "σκελ",
            "συρφ",
            "τοκ",
            "υ",
            "δ",
            "εμ",
            "θαρρ",
            "θ",
        ],
        false,
    )
});

/// `GreekStemmer.exc12a`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC12A: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(["π", "απ", "συμπ", "ασυμπ", "ακαταπ", "αμεταμφ"], false)
});

/// `GreekStemmer.exc12b`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC12B: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "αλ",
            "αρ",
            "εκτελ",
            "ζ",
            "μ",
            "ξ",
            "παρακαλ",
            "αρ",
            "προ",
            "νισ",
        ],
        false,
    )
});

/// `GreekStemmer.exc13`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC13: LazyLock<CharArraySet> =
    LazyLock::new(|| CharArraySet::from_iter(["διαθ", "θ", "παρακαταθ", "προσθ", "συνθ"], false));

/// `GreekStemmer.exc14`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC14: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "φαρμακ",
            "χαδ",
            "αγκ",
            "αναρρ",
            "βρομ",
            "εκλιπ",
            "λαμπιδ",
            "λεχ",
            "μ",
            "πατ",
            "ρ",
            "λ",
            "μεδ",
            "μεσαζ",
            "υποτειν",
            "αμ",
            "αιθ",
            "ανηκ",
            "δεσποζ",
            "ενδιαφερ",
            "δε",
            "δευτερευ",
            "καθαρευ",
            "πλε",
            "τσα",
        ],
        false,
    )
});

/// `GreekStemmer.exc15a`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC15A: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "αβαστ",
            "πολυφ",
            "αδηφ",
            "παμφ",
            "ρ",
            "ασπ",
            "αφ",
            "αμαλ",
            "αμαλλι",
            "ανυστ",
            "απερ",
            "ασπαρ",
            "αχαρ",
            "δερβεν",
            "δροσοπ",
            "ξεφ",
            "νεοπ",
            "νομοτ",
            "ολοπ",
            "ομοτ",
            "προστ",
            "προσωποπ",
            "συμπ",
            "συντ",
            "τ",
            "υποτ",
            "χαρ",
            "αειπ",
            "αιμοστ",
            "ανυπ",
            "αποτ",
            "αρτιπ",
            "διατ",
            "εν",
            "επιτ",
            "κροκαλοπ",
            "σιδηροπ",
            "λ",
            "ναυ",
            "ουλαμ",
            "ουρ",
            "π",
            "τρ",
            "μ",
        ],
        false,
    )
});

/// `GreekStemmer.exc15b`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC15B: LazyLock<CharArraySet> =
    LazyLock::new(|| CharArraySet::from_iter(["ψοφ", "ναυλοχ"], false));

/// `GreekStemmer.exc16`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC16: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        ["ν", "χερσον", "δωδεκαν", "ερημον", "μεγαλον", "επταν"],
        false,
    )
});

/// `GreekStemmer.exc17`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC17: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        [
            "ασβ",
            "σβ",
            "αχρ",
            "χρ",
            "απλ",
            "αειμν",
            "δυσχρ",
            "ευχρ",
            "κοινοχρ",
            "παλιμψ",
        ],
        false,
    )
});

/// `GreekStemmer.exc18`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC18: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(["ν", "ρ", "σπι", "στραβομουτσ", "κακομουτσ", "εξων"], false)
});

/// `GreekStemmer.exc19`, `new CharArraySet(Arrays.asList(...), false)`.
static EXC19: LazyLock<CharArraySet> = LazyLock::new(|| {
    CharArraySet::from_iter(
        ["παρασουσ", "φ", "χ", "ωριοπλ", "αζ", "αλλοσουσ", "ασουσ"],
        false,
    )
});
/// A stemmer for Greek words, according to: *Development of a Stemmer for
/// the Greek Language.* Georgios Ntais
///
/// NOTE: Input is expected to be casefolded for Greek (including folding of
/// final sigma to sigma), and with diacritics removed. This can be achieved
/// with either `GreekLowerCaseFilter` or ICUFoldingFilter.
///
/// Equivalent to `org.apache.lucene.analysis.el.GreekStemmer`
/// (`@lucene.experimental`). The term it stems is the `char[]` of Java,
/// UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GreekStemmer;

impl GreekStemmer {
    /// Equivalent to the implicit `GreekStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems a word contained in a leading portion of a char[] array. The
    /// word is passed through a number of rules that modify its length.
    ///
    /// * `s`: A char[] array that contains the word to be stemmed.
    /// * `len`: The length of the char[] array.
    /// * returns: The new length of the stemmed word.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], mut len: usize) -> usize {
        if len < 4 {
            // too short
            return len;
        }
        let orig_len = len;
        // "short rules": if it hits one of these, it skips the "long list"
        len = self.rule0(s, len);
        len = self.rule1(s, len);
        len = self.rule2(s, len);
        len = self.rule3(s, len);
        len = self.rule4(s, len);
        len = self.rule5(s, len);
        len = self.rule6(s, len);
        len = self.rule7(s, len);
        len = self.rule8(s, len);
        len = self.rule9(s, len);
        len = self.rule10(s, len);
        len = self.rule11(s, len);
        len = self.rule12(s, len);
        len = self.rule13(s, len);
        len = self.rule14(s, len);
        len = self.rule15(s, len);
        len = self.rule16(s, len);
        len = self.rule17(s, len);
        len = self.rule18(s, len);
        len = self.rule19(s, len);
        len = self.rule20(s, len);
        // "long list"
        if len == orig_len {
            len = self.rule21(s, len);
        }
        self.rule22(s, len)
    }

    /// Equivalent to `rule0(char[], int)`.
    fn rule0(&self, s: &mut [u16], len: usize) -> usize {
        if len > 9 && (ends_with(s, len, "καθεστωτοσ") || ends_with(s, len, "καθεστωτων"))
        {
            return len - 4;
        }
        if len > 8 && (ends_with(s, len, "γεγονοτοσ") || ends_with(s, len, "γεγονοτων"))
        {
            return len - 4;
        }
        if len > 8 && ends_with(s, len, "καθεστωτα") {
            return len - 3;
        }
        if len > 7 && (ends_with(s, len, "τατογιου") || ends_with(s, len, "τατογιων"))
        {
            return len - 4;
        }
        if len > 7 && ends_with(s, len, "γεγονοτα") {
            return len - 3;
        }
        if len > 7 && ends_with(s, len, "καθεστωσ") {
            return len - 2;
        }
        if len > 6 && (ends_with(s, len, "σκαγιου"))
            || ends_with(s, len, "σκαγιων")
            || ends_with(s, len, "ολογιου")
            || ends_with(s, len, "ολογιων")
            || ends_with(s, len, "κρεατοσ")
            || ends_with(s, len, "κρεατων")
            || ends_with(s, len, "περατοσ")
            || ends_with(s, len, "περατων")
            || ends_with(s, len, "τερατοσ")
            || ends_with(s, len, "τερατων")
        {
            return len - 4;
        }
        if len > 6 && ends_with(s, len, "τατογια") {
            return len - 3;
        }
        if len > 6 && ends_with(s, len, "γεγονοσ") {
            return len - 2;
        }
        if len > 5
            && (ends_with(s, len, "φαγιου")
                || ends_with(s, len, "φαγιων")
                || ends_with(s, len, "σογιου")
                || ends_with(s, len, "σογιων"))
        {
            return len - 4;
        }
        if len > 5
            && (ends_with(s, len, "σκαγια")
                || ends_with(s, len, "ολογια")
                || ends_with(s, len, "κρεατα")
                || ends_with(s, len, "περατα")
                || ends_with(s, len, "τερατα"))
        {
            return len - 3;
        }
        if len > 4
            && (ends_with(s, len, "φαγια")
                || ends_with(s, len, "σογια")
                || ends_with(s, len, "φωτοσ")
                || ends_with(s, len, "φωτων"))
        {
            return len - 3;
        }
        if len > 4
            && (ends_with(s, len, "κρεασ")
                || ends_with(s, len, "περασ")
                || ends_with(s, len, "τερασ"))
        {
            return len - 2;
        }
        if len > 3 && ends_with(s, len, "φωτα") {
            return len - 2;
        }
        if len > 2 && ends_with(s, len, "φωσ") {
            return len - 1;
        }
        len
    }

    /// Equivalent to `rule1(char[], int)`.
    fn rule1(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 4 && (ends_with(s, len, "αδεσ") || ends_with(s, len, "αδων")) {
            len -= 4;
            if !(ends_with(s, len, "οκ")
                || ends_with(s, len, "μαμ")
                || ends_with(s, len, "μαν")
                || ends_with(s, len, "μπαμπ")
                || ends_with(s, len, "πατερ")
                || ends_with(s, len, "γιαγι")
                || ends_with(s, len, "νταντ")
                || ends_with(s, len, "κυρ")
                || ends_with(s, len, "θει")
                || ends_with(s, len, "πεθερ"))
            {
                len += 2; // add back -αδ
            }
        }
        len
    }

    /// Equivalent to `rule2(char[], int)`.
    fn rule2(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 4 && (ends_with(s, len, "εδεσ") || ends_with(s, len, "εδων")) {
            len -= 4;
            if ends_with(s, len, "οπ")
                || ends_with(s, len, "ιπ")
                || ends_with(s, len, "εμπ")
                || ends_with(s, len, "υπ")
                || ends_with(s, len, "γηπ")
                || ends_with(s, len, "δαπ")
                || ends_with(s, len, "κρασπ")
                || ends_with(s, len, "μιλ")
            {
                len += 2; // add back -εδ
            }
        }
        len
    }

    /// Equivalent to `rule3(char[], int)`.
    fn rule3(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 5 && (ends_with(s, len, "ουδεσ") || ends_with(s, len, "ουδων")) {
            len -= 5;
            if ends_with(s, len, "αρκ")
                || ends_with(s, len, "καλιακ")
                || ends_with(s, len, "πεταλ")
                || ends_with(s, len, "λιχ")
                || ends_with(s, len, "πλεξ")
                || ends_with(s, len, "σκ")
                || ends_with(s, len, "σ")
                || ends_with(s, len, "φλ")
                || ends_with(s, len, "φρ")
                || ends_with(s, len, "βελ")
                || ends_with(s, len, "λουλ")
                || ends_with(s, len, "χν")
                || ends_with(s, len, "σπ")
                || ends_with(s, len, "τραγ")
                || ends_with(s, len, "φε")
            {
                len += 3; // add back -ουδ
            }
        }
        len
    }

    /// Equivalent to `rule4(char[], int)`.
    fn rule4(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 3 && (ends_with(s, len, "εωσ") || ends_with(s, len, "εων")) {
            len -= 3;
            if EXC4.contains_chars(Some(s), 0, len) {
                len += 1; // add back -ε
            }
        }
        len
    }

    /// Equivalent to `rule5(char[], int)`.
    fn rule5(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 2 && ends_with(s, len, "ια") {
            len -= 2;
            if ends_with_vowel(s, len) {
                len += 1; // add back -ι
            }
        } else if len > 3 && (ends_with(s, len, "ιου") || ends_with(s, len, "ιων")) {
            len -= 3;
            if ends_with_vowel(s, len) {
                len += 1; // add back -ι
            }
        }
        len
    }

    /// Equivalent to `rule6(char[], int)`.
    fn rule6(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 3 && (ends_with(s, len, "ικα") || ends_with(s, len, "ικο")) {
            len -= 3;
            removed = true;
        } else if len > 4 && (ends_with(s, len, "ικου") || ends_with(s, len, "ικων")) {
            len -= 4;
            removed = true;
        }
        if removed && (ends_with_vowel(s, len) || EXC6.contains_chars(Some(s), 0, len)) {
            len += 2; // add back -ικ
        }
        len
    }

    /// Equivalent to `rule7(char[], int)`.
    fn rule7(&self, s: &mut [u16], mut len: usize) -> usize {
        if len == 5 && ends_with(s, len, "αγαμε") {
            return len - 1;
        }
        if len > 7 && ends_with(s, len, "ηθηκαμε") {
            len -= 7;
        } else if len > 6 && ends_with(s, len, "ουσαμε") {
            len -= 6;
        } else if len > 5
            && (ends_with(s, len, "αγαμε")
                || ends_with(s, len, "ησαμε")
                || ends_with(s, len, "ηκαμε"))
        {
            len -= 5;
        }
        if len > 3 && ends_with(s, len, "αμε") {
            len -= 3;
            if EXC7.contains_chars(Some(s), 0, len) {
                len += 2; // add back -αμ
            }
        }
        len
    }

    /// Equivalent to `rule8(char[], int)`.
    fn rule8(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 8 && ends_with(s, len, "ιουντανε") {
            len -= 8;
            removed = true;
        } else if len > 7 && ends_with(s, len, "ιοντανε")
            || ends_with(s, len, "ουντανε")
            || ends_with(s, len, "ηθηκανε")
        {
            len -= 7;
            removed = true;
        } else if len > 6 && ends_with(s, len, "ιοτανε")
            || ends_with(s, len, "οντανε")
            || ends_with(s, len, "ουσανε")
        {
            len -= 6;
            removed = true;
        } else if len > 5 && ends_with(s, len, "αγανε")
            || ends_with(s, len, "ησανε")
            || ends_with(s, len, "οτανε")
            || ends_with(s, len, "ηκανε")
        {
            len -= 5;
            removed = true;
        }
        if removed && EXC8A.contains_chars(Some(s), 0, len) {
            // add -αγαν (we removed > 4 chars so it's safe)
            len += 4;
            s[len - 4] = 'α' as u16;
            s[len - 3] = 'γ' as u16;
            s[len - 2] = 'α' as u16;
            s[len - 1] = 'ν' as u16;
        }
        if len > 3 && ends_with(s, len, "ανε") {
            len -= 3;
            if ends_with_vowel_no_y(s, len) || EXC8B.contains_chars(Some(s), 0, len) {
                len += 2; // add back -αν
            }
        }
        len
    }

    /// Equivalent to `rule9(char[], int)`.
    fn rule9(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 5 && ends_with(s, len, "ησετε") {
            len -= 5;
        }
        if len > 3 && ends_with(s, len, "ετε") {
            len -= 3;
            if EXC9.contains_chars(Some(s), 0, len)
                || ends_with_vowel_no_y(s, len)
                || ends_with(s, len, "οδ")
                || ends_with(s, len, "αιρ")
                || ends_with(s, len, "φορ")
                || ends_with(s, len, "ταθ")
                || ends_with(s, len, "διαθ")
                || ends_with(s, len, "σχ")
                || ends_with(s, len, "ενδ")
                || ends_with(s, len, "ευρ")
                || ends_with(s, len, "τιθ")
                || ends_with(s, len, "υπερθ")
                || ends_with(s, len, "ραθ")
                || ends_with(s, len, "ενθ")
                || ends_with(s, len, "ροθ")
                || ends_with(s, len, "σθ")
                || ends_with(s, len, "πυρ")
                || ends_with(s, len, "αιν")
                || ends_with(s, len, "συνδ")
                || ends_with(s, len, "συν")
                || ends_with(s, len, "συνθ")
                || ends_with(s, len, "χωρ")
                || ends_with(s, len, "πον")
                || ends_with(s, len, "βρ")
                || ends_with(s, len, "καθ")
                || ends_with(s, len, "ευθ")
                || ends_with(s, len, "εκθ")
                || ends_with(s, len, "νετ")
                || ends_with(s, len, "ρον")
                || ends_with(s, len, "αρκ")
                || ends_with(s, len, "βαρ")
                || ends_with(s, len, "βολ")
                || ends_with(s, len, "ωφελ")
            {
                len += 2; // add back -ετ
            }
        }
        len
    }

    /// Equivalent to `rule10(char[], int)`.
    fn rule10(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 5 && (ends_with(s, len, "οντασ") || ends_with(s, len, "ωντασ")) {
            len -= 5;
            if len == 3 && ends_with(s, len, "αρχ") {
                len += 3; // add back *ντ
                s[len - 3] = 'ο' as u16;
            }
            if ends_with(s, len, "κρε") {
                len += 3; // add back *ντ
                s[len - 3] = 'ω' as u16;
            }
        }
        len
    }

    /// Equivalent to `rule11(char[], int)`.
    fn rule11(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 6 && ends_with(s, len, "ομαστε") {
            len -= 6;
            if len == 2 && ends_with(s, len, "ον") {
                len += 5; // add back -ομαστ
            }
        } else if len > 7 && ends_with(s, len, "ιομαστε") {
            len -= 7;
            if len == 2 && ends_with(s, len, "ον") {
                len += 5;
                s[len - 5] = 'ο' as u16;
                s[len - 4] = 'μ' as u16;
                s[len - 3] = 'α' as u16;
                s[len - 2] = 'σ' as u16;
                s[len - 1] = 'τ' as u16;
            }
        }
        len
    }

    /// Equivalent to `rule12(char[], int)`.
    fn rule12(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 5 && ends_with(s, len, "ιεστε") {
            len -= 5;
            if EXC12A.contains_chars(Some(s), 0, len) {
                len += 4; // add back -ιεστ
            }
        }
        if len > 4 && ends_with(s, len, "εστε") {
            len -= 4;
            if EXC12B.contains_chars(Some(s), 0, len) {
                len += 3; // add back -εστ
            }
        }
        len
    }

    /// Equivalent to `rule13(char[], int)`.
    fn rule13(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 6 && ends_with(s, len, "ηθηκεσ") {
            len -= 6;
        } else if len > 5 && (ends_with(s, len, "ηθηκα") || ends_with(s, len, "ηθηκε")) {
            len -= 5;
        }
        let mut removed = false;
        if len > 4 && ends_with(s, len, "ηκεσ") {
            len -= 4;
            removed = true;
        } else if len > 3 && (ends_with(s, len, "ηκα") || ends_with(s, len, "ηκε")) {
            len -= 3;
            removed = true;
        }
        if removed
            && (EXC13.contains_chars(Some(s), 0, len)
                || ends_with(s, len, "σκωλ")
                || ends_with(s, len, "σκουλ")
                || ends_with(s, len, "ναρθ")
                || ends_with(s, len, "σφ")
                || ends_with(s, len, "οθ")
                || ends_with(s, len, "πιθ"))
        {
            len += 2; // add back the -ηκ
        }
        len
    }

    /// Equivalent to `rule14(char[], int)`.
    fn rule14(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 5 && ends_with(s, len, "ουσεσ") {
            len -= 5;
            removed = true;
        } else if len > 4 && (ends_with(s, len, "ουσα") || ends_with(s, len, "ουσε")) {
            len -= 4;
            removed = true;
        }
        if removed
            && (EXC14.contains_chars(Some(s), 0, len)
                || ends_with_vowel(s, len)
                || ends_with(s, len, "ποδαρ")
                || ends_with(s, len, "βλεπ")
                || ends_with(s, len, "πανταχ")
                || ends_with(s, len, "φρυδ")
                || ends_with(s, len, "μαντιλ")
                || ends_with(s, len, "μαλλ")
                || ends_with(s, len, "κυματ")
                || ends_with(s, len, "λαχ")
                || ends_with(s, len, "ληγ")
                || ends_with(s, len, "φαγ")
                || ends_with(s, len, "ομ")
                || ends_with(s, len, "πρωτ"))
        {
            len += 3; // add back -ουσ
        }
        len
    }

    /// Equivalent to `rule15(char[], int)`.
    fn rule15(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 4 && ends_with(s, len, "αγεσ") {
            len -= 4;
            removed = true;
        } else if len > 3 && (ends_with(s, len, "αγα") || ends_with(s, len, "αγε")) {
            len -= 3;
            removed = true;
        }
        if removed {
            let cond1 = EXC15A.contains_chars(Some(s), 0, len)
                || ends_with(s, len, "οφ")
                || ends_with(s, len, "πελ")
                || ends_with(s, len, "χορτ")
                || ends_with(s, len, "λλ")
                || ends_with(s, len, "σφ")
                || ends_with(s, len, "ρπ")
                || ends_with(s, len, "φρ")
                || ends_with(s, len, "πρ")
                || ends_with(s, len, "λοχ")
                || ends_with(s, len, "σμην");
            let cond2 = EXC15B.contains_chars(Some(s), 0, len) || ends_with(s, len, "κολλ");
            if cond1 && !cond2 {
                len += 2; // add back -αγ
            }
        }
        len
    }

    /// Equivalent to `rule16(char[], int)`.
    fn rule16(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 4 && ends_with(s, len, "ησου") {
            len -= 4;
            removed = true;
        } else if len > 3 && (ends_with(s, len, "ησε") || ends_with(s, len, "ησα")) {
            len -= 3;
            removed = true;
        }
        if removed && EXC16.contains_chars(Some(s), 0, len) {
            len += 2; // add back -ησ
        }
        len
    }

    /// Equivalent to `rule17(char[], int)`.
    fn rule17(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 4 && ends_with(s, len, "ηστε") {
            len -= 4;
            if EXC17.contains_chars(Some(s), 0, len) {
                len += 3; // add back the -ηστ
            }
        }
        len
    }

    /// Equivalent to `rule18(char[], int)`.
    fn rule18(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 6 && (ends_with(s, len, "ησουνε") || ends_with(s, len, "ηθουνε")) {
            len -= 6;
            removed = true;
        } else if len > 4 && ends_with(s, len, "ουνε") {
            len -= 4;
            removed = true;
        }
        if removed && EXC18.contains_chars(Some(s), 0, len) {
            len += 3;
            s[len - 3] = 'ο' as u16;
            s[len - 2] = 'υ' as u16;
            s[len - 1] = 'ν' as u16;
        }
        len
    }

    /// Equivalent to `rule19(char[], int)`.
    fn rule19(&self, s: &mut [u16], mut len: usize) -> usize {
        let mut removed = false;
        if len > 6 && (ends_with(s, len, "ησουμε") || ends_with(s, len, "ηθουμε")) {
            len -= 6;
            removed = true;
        } else if len > 4 && ends_with(s, len, "ουμε") {
            len -= 4;
            removed = true;
        }
        if removed && EXC19.contains_chars(Some(s), 0, len) {
            len += 3;
            s[len - 3] = 'ο' as u16;
            s[len - 2] = 'υ' as u16;
            s[len - 1] = 'μ' as u16;
        }
        len
    }

    /// Equivalent to `rule20(char[], int)`.
    fn rule20(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 5 && (ends_with(s, len, "ματων") || ends_with(s, len, "ματοσ")) {
            len -= 3;
        } else if len > 4 && ends_with(s, len, "ματα") {
            len -= 2;
        }
        len
    }

    /// Equivalent to `rule21(char[], int)`.
    fn rule21(&self, s: &mut [u16], len: usize) -> usize {
        if len > 9 && ends_with(s, len, "ιοντουσαν") {
            return len - 9;
        }
        if len > 8
            && (ends_with(s, len, "ιομασταν")
                || ends_with(s, len, "ιοσασταν")
                || ends_with(s, len, "ιουμαστε")
                || ends_with(s, len, "οντουσαν"))
        {
            return len - 8;
        }
        if len > 7
            && (ends_with(s, len, "ιεμαστε")
                || ends_with(s, len, "ιεσαστε")
                || ends_with(s, len, "ιομουνα")
                || ends_with(s, len, "ιοσαστε")
                || ends_with(s, len, "ιοσουνα")
                || ends_with(s, len, "ιουνται")
                || ends_with(s, len, "ιουνταν")
                || ends_with(s, len, "ηθηκατε")
                || ends_with(s, len, "ομασταν")
                || ends_with(s, len, "οσασταν")
                || ends_with(s, len, "ουμαστε"))
        {
            return len - 7;
        }
        if len > 6
            && (ends_with(s, len, "ιομουν")
                || ends_with(s, len, "ιονταν")
                || ends_with(s, len, "ιοσουν")
                || ends_with(s, len, "ηθειτε")
                || ends_with(s, len, "ηθηκαν")
                || ends_with(s, len, "ομουνα")
                || ends_with(s, len, "οσαστε")
                || ends_with(s, len, "οσουνα")
                || ends_with(s, len, "ουνται")
                || ends_with(s, len, "ουνταν")
                || ends_with(s, len, "ουσατε"))
        {
            return len - 6;
        }
        if len > 5
            && (ends_with(s, len, "αγατε")
                || ends_with(s, len, "ιεμαι")
                || ends_with(s, len, "ιεται")
                || ends_with(s, len, "ιεσαι")
                || ends_with(s, len, "ιοταν")
                || ends_with(s, len, "ιουμα")
                || ends_with(s, len, "ηθεισ")
                || ends_with(s, len, "ηθουν")
                || ends_with(s, len, "ηκατε")
                || ends_with(s, len, "ησατε")
                || ends_with(s, len, "ησουν")
                || ends_with(s, len, "ομουν")
                || ends_with(s, len, "ονται")
                || ends_with(s, len, "ονταν")
                || ends_with(s, len, "οσουν")
                || ends_with(s, len, "ουμαι")
                || ends_with(s, len, "ουσαν"))
        {
            return len - 5;
        }
        if len > 4
            && (ends_with(s, len, "αγαν")
                || ends_with(s, len, "αμαι")
                || ends_with(s, len, "ασαι")
                || ends_with(s, len, "αται")
                || ends_with(s, len, "ειτε")
                || ends_with(s, len, "εσαι")
                || ends_with(s, len, "εται")
                || ends_with(s, len, "ηδεσ")
                || ends_with(s, len, "ηδων")
                || ends_with(s, len, "ηθει")
                || ends_with(s, len, "ηκαν")
                || ends_with(s, len, "ησαν")
                || ends_with(s, len, "ησει")
                || ends_with(s, len, "ησεσ")
                || ends_with(s, len, "ομαι")
                || ends_with(s, len, "οταν"))
        {
            return len - 4;
        }
        if len > 3
            && (ends_with(s, len, "αει")
                || ends_with(s, len, "εισ")
                || ends_with(s, len, "ηθω")
                || ends_with(s, len, "ησω")
                || ends_with(s, len, "ουν")
                || ends_with(s, len, "ουσ"))
        {
            return len - 3;
        }
        if len > 2
            && (ends_with(s, len, "αν")
                || ends_with(s, len, "ασ")
                || ends_with(s, len, "αω")
                || ends_with(s, len, "ει")
                || ends_with(s, len, "εσ")
                || ends_with(s, len, "ησ")
                || ends_with(s, len, "οι")
                || ends_with(s, len, "οσ")
                || ends_with(s, len, "ου")
                || ends_with(s, len, "υσ")
                || ends_with(s, len, "ων"))
        {
            return len - 2;
        }
        if len > 1 && ends_with_vowel(s, len) {
            return len - 1;
        }
        len
    }

    /// Equivalent to `rule22(char[], int)`.
    fn rule22(&self, s: &mut [u16], len: usize) -> usize {
        if ends_with(s, len, "εστερ") || ends_with(s, len, "εστατ") {
            return len - 5;
        }
        if ends_with(s, len, "οτερ")
            || ends_with(s, len, "οτατ")
            || ends_with(s, len, "υτερ")
            || ends_with(s, len, "υτατ")
            || ends_with(s, len, "ωτερ")
            || ends_with(s, len, "ωτατ")
        {
            return len - 4;
        }
        len
    }
}

/// Checks if the word contained in the leading portion of char[] array ,
/// ends with the suffix given as parameter.
///
/// * `s`: A char[] array that represents a word.
/// * `len`: The length of the char[] array.
/// * `suffix`: A `String` object to check if the word given ends with these
///   characters.
/// * returns: True if the word ends with the suffix given , false otherwise.
///
/// Equivalent to the private `GreekStemmer.endsWith(char[], int, String)`.
fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
    let suffix_len = suffix.encode_utf16().count();
    if suffix_len > len {
        return false;
    }
    // Java compares from the last unit backwards; the comparisons have no
    // side effect, so comparing forwards answers the same.
    let start = len - suffix_len;
    suffix
        .encode_utf16()
        .enumerate()
        .all(|(i, c)| s[start + i] == c)
}

/// Checks if the word contained in the leading portion of char[] array ,
/// ends with a Greek vowel.
///
/// * `s`: A char[] array that represents a word.
/// * `len`: The length of the char[] array.
/// * returns: True if the word contained in the leading portion of char[]
///   array , ends with a vowel , false otherwise.
///
/// Equivalent to the private `GreekStemmer.endsWithVowel(char[], int)`.
fn ends_with_vowel(s: &[u16], len: usize) -> bool {
    if len == 0 {
        return false;
    }
    matches!(
        char::from_u32(u32::from(s[len - 1])),
        Some('α' | 'ε' | 'η' | 'ι' | 'ο' | 'υ' | 'ω')
    )
}

/// Checks if the word contained in the leading portion of char[] array ,
/// ends with a Greek vowel.
///
/// * `s`: A char[] array that represents a word.
/// * `len`: The length of the char[] array.
/// * returns: True if the word contained in the leading portion of char[]
///   array , ends with a vowel , false otherwise.
///
/// Equivalent to the private `GreekStemmer.endsWithVowelNoY(char[], int)`.
fn ends_with_vowel_no_y(s: &[u16], len: usize) -> bool {
    if len == 0 {
        return false;
    }
    matches!(
        char::from_u32(u32::from(s[len - 1])),
        Some('α' | 'ε' | 'η' | 'ι' | 'ο' | 'ω')
    )
}
