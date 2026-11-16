//! The European Portuguese analysis chain: search insensitive to case,
//! accents and other diacritics (cedilla, tilde, acute, circumflex).
//!
//! [`PortugueseFoldingAnalyzer`] is built only from ported Lucene 10.5.0
//! components and equals this `CustomAnalyzer`:
//!
//! ```java
//! Analyzer analyzer = CustomAnalyzer.builder()
//!     .withTokenizer("standard")
//!     .addTokenFilter("lowercase")
//!     .addTokenFilter("asciiFolding")                          // preserveOriginal = false
//!     .addTokenFilter("stop", "words", "portuguese_stop_folded.txt")
//!     .addTokenFilter("patternReplace", "pattern", "oes$", "replacement", "ões", "replace", "first")
//!     .addTokenFilter("patternReplace", "pattern", "aes$", "replacement", "ães", "replace", "first")
//!     .addTokenFilter("patternReplace", "pattern", "ois$", "replacement", "óis", "replace", "first")
//!     .addTokenFilter("portugueseLightStem")
//!     .addTokenFilter("asciiFolding")
//!     .build();
//! ```
//!
//! where `portuguese_stop_folded.txt` holds `ASCIIFoldingFilter.foldToASCII`
//! of every word of Lucene's `snowball/portuguese_stop.txt` (the stop set of
//! `PortugueseAnalyzer`). The filters are `StandardTokenizer`,
//! `LowerCaseFilter`, `ASCIIFoldingFilter`, `StopFilter`,
//! `PatternReplaceFilter` (three times), `PortugueseLightStemFilter` and
//! `ASCIIFoldingFilter`; `normalize` applies the multi-term-aware ones,
//! `LowerCaseFilter` and `ASCIIFoldingFilter`, as `CustomAnalyzer.normalize`
//! does. Compared with `PortugueseAnalyzer.createComponents`, the chain folds
//! before the stop filter and the stemmer, folds the stop words to match, and
//! restores three plural endings before stemming.
//!
//! # Why fold first
//!
//! Folding before everything else makes every spelling that differs only in
//! diacritics the *same string* before the stop filter and the stemmer see
//! it, so `preço`/`preco`, `música`/`musica`, `português`/`portugues` and
//! `ação`/`acao` cannot diverge, at index or at query time.
//!
//! # Why three plural endings are restored
//!
//! `PortugueseLightStemmer.removeSuffix` keys three plural rules on accented
//! letters (`PortugueseLightStemmer.java`): `-ões` and `-ães` become `-ão`
//! (`ações` → `ação`, `pães` → `pão`), and `-óis` becomes `-ol`
//! (`anzóis` → `anzol`). On folded input those rules never fire: `acoes`
//! falls through to the plain `-s` rule (`acoe`) and `anzois` to the `-is`
//! rule (`anzoil`), so a plural would no longer meet its singular. The
//! `PatternReplaceFilter`s put back exactly the accents those three rules
//! test, on the token's last three letters only, so the stemmer applies the
//! rule Lucene applies to the accented spelling. Every other stemmer rule
//! (`-eis`/`-éis`, `-ais`, `-es`, `-ns`, `-mente`, the feminine and vowel
//! rules) tests unaccented letters or both forms and behaves identically on
//! folded input. The final `ASCIIFoldingFilter` folds the accents the stemmer
//! writes (`ã`, `ê`) on stems it does not fold itself.
//!
//! Resulting stems: `ação`, `acao`, `AÇÃO`, `ações`, `acoes`, `AÇÕES` → `acao`;
//! `acção`, `acções`, `accao`, `accoes` → `acca`; `pão`, `pães`, `paes` →
//! `pao`; `limão`, `limões`, `limoes` → `lima`; `papéis`, `papeis`, `papel` →
//! `papel`; `anzóis`, `anzois`, `anzol` → `anzol`.
//!
//! **Cost:** an unaccented word ending in `-oes`, `-aes` or `-ois` that is
//! not such a plural (a foreign word such as `shoes`) gets the plural rule
//! too, exactly as its accented spelling would in Lucene's own analyzer.
//!
//! # Why the stop words are folded
//!
//! The stop filter runs after folding, so it must hold the folded forms
//! (`não` → `nao`, `você` → `voce`, `está` → `esta`); the accented spellings
//! and the unaccented ones are then removed alike.

use std::sync::{Arc, LazyLock};

use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::{
    new_lower_case_filter, new_standard_tokenizer, new_stop_filter, Analyzer, AnalyzerState,
    CharArraySet, CharReader, GlobalReuseStrategy, SharedTokenStream, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result as LuceneResult;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene_analysis_common::analysis::miscellaneous::ascii_folding_filter::{
    fold_to_ascii, new_ascii_folding_filter,
};
use rucene_analysis_common::analysis::pattern::pattern_replace_filter::new_pattern_replace_filter;
use rucene_analysis_common::analysis::pt::portuguese_analyzer::PortugueseAnalyzer;
use rucene_analysis_common::analysis::pt::portuguese_light_stem_filter::new_portuguese_light_stem_filter;

/// Folds `word` to ASCII exactly as `ASCIIFoldingFilter.foldToASCII` does.
#[must_use]
pub fn fold(word: &str) -> String {
    let input: Vec<char> = word.chars().collect();
    // `ASCIIFoldingFilter` sizes its output at 4 chars per input char, the
    // longest expansion of its table.
    let mut output = vec!['\0'; input.len() * 4];
    let length = fold_to_ascii(&input, 0, &mut output, 0, input.len());
    output[..length].iter().collect()
}

static STOP_SET: LazyLock<CharArraySet> = LazyLock::new(|| {
    let default = PortugueseAnalyzer::get_default_stop_set();
    let words: Vec<String> = default.iter().map(String::from_utf16_lossy).collect();
    let mut set = CharArraySet::new(words.len(), false);
    for word in &words {
        set.add(fold(word));
    }
    set
});

/// The stop set of [`PortugueseFoldingAnalyzer`]: the ASCII-folded form of
/// every Portuguese snowball stop word (`PortugueseAnalyzer.getDefaultStopSet()`),
/// since the stop filter runs after folding.
#[must_use]
pub fn stop_set() -> &'static CharArraySet {
    &STOP_SET
}

/// The `PatternReplaceFilter` rules that restore, on folded tokens, the
/// accents `PortugueseLightStemmer` tests in its `-ões`, `-ães` and `-óis`
/// plural rules: `(pattern, replacement)`, replacing the first match.
pub const PLURAL_RESTORATIONS: &[(&str, &str)] =
    &[("oes$", "ões"), ("aes$", "ães"), ("ois$", "óis")];

/// The pt-PT analyzer of every text field: `StandardTokenizer`,
/// `LowerCaseFilter`, `ASCIIFoldingFilter`, `StopFilter`, three
/// `PatternReplaceFilter`s, `PortugueseLightStemFilter`, `ASCIIFoldingFilter`
/// (see the [module documentation](self) for the Java `CustomAnalyzer`
/// equivalent and the reasons for the order).
#[derive(Debug)]
pub struct PortugueseFoldingAnalyzer {
    state: AnalyzerState,
}

impl Default for PortugueseFoldingAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl PortugueseFoldingAnalyzer {
    /// A new analyzer with Lucene's default (global) reuse strategy.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for PortugueseFoldingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "rucene_cli_base.analysis.PortugueseFoldingAnalyzer"
    }

    fn create_components(&self, _field_name: Option<&str>) -> LuceneResult<TokenStreamComponents> {
        let source = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let source_for_reader = Arc::clone(&source);
        let set_reader = Box::new(move |reader: Box<dyn CharReader>| {
            source_for_reader.borrow_mut().set_reader(reader)
        });
        let mut result: Box<dyn TokenStream> = Box::new(new_lower_case_filter(Box::new(
            SharedTokenStream::new(source),
        ))?);
        result = Box::new(new_ascii_folding_filter(result)?);
        result = Box::new(new_stop_filter(result, stop_set().clone())?);
        for (pattern, replacement) in PLURAL_RESTORATIONS {
            let pattern = JavaPattern::compile(pattern, 0)
                .expect("INVARIANT: the plural restoration patterns are valid regular expressions");
            result = Box::new(new_pattern_replace_filter(
                result,
                &pattern,
                Some(replacement),
                false,
            )?);
        }
        result = Box::new(new_portuguese_light_stem_filter(result)?);
        let result: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(new_ascii_folding_filter(result)?));
        Ok(TokenStreamComponents::new(set_reader, result))
    }

    /// `LowerCaseFilter` then `ASCIIFoldingFilter`: the filters of the chain
    /// that are normalizing (multi-term aware), as `CustomAnalyzer.normalize`
    /// selects them.
    fn normalize_token_stream(
        &self,
        _field_name: Option<&str>,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let lower = new_lower_case_filter(input)
            .expect("INVARIANT: the normalization input holds the CharTermAttribute");
        Box::new(
            new_ascii_folding_filter(Box::new(lower))
                .expect("INVARIANT: the normalization input holds the CharTermAttribute"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucene::analysis::CharTermAttribute;

    fn analyze(text: &str) -> Vec<String> {
        let analyzer = PortugueseFoldingAnalyzer::new();
        let stream = analyzer.token_stream_from_str("f", text).unwrap();
        let mut stream = stream.borrow_mut();
        stream.reset().unwrap();
        let mut terms = Vec::new();
        while stream.increment_token().unwrap() {
            let term = stream
                .attribute_source()
                .get_attribute_interface::<dyn CharTermAttribute>()
                .unwrap();
            // `CharTermAttribute.toString()`: the term text.
            terms.push(term.buffer()[..term.length()].iter().collect());
        }
        stream.end().unwrap();
        stream.close().unwrap();
        terms
    }

    /// Spelling groups whose forms must share an analysed token, with the
    /// stem the chain gives them (traced from `PortugueseLightStemmer`).
    const GROUPS: &[(&[&str], &str)] = &[
        (&["ação", "acao", "AÇÃO", "ações", "acoes", "AÇÕES"], "acao"),
        (&["acção", "acções", "accao", "accoes"], "acca"),
        (&["pão", "pães", "paes"], "pao"),
        (&["limão", "limões", "limoes"], "lima"),
        (&["papéis", "papeis", "papel"], "papel"),
        (&["anzóis", "anzois", "anzol"], "anzol"),
        (&["preço", "preco"], "prec"),
        (&["música", "musica"], "music"),
        (&["português", "portugues"], "portugue"),
    ];

    #[test]
    fn every_pair_of_a_spelling_group_shares_a_token() {
        for (group, stem) in GROUPS {
            let analysed: Vec<(&str, Vec<String>)> =
                group.iter().map(|form| (*form, analyze(form))).collect();
            for (form, tokens) in &analysed {
                assert_eq!(tokens, &vec![(*stem).to_owned()], "{form}");
            }
            for (a, tokens_a) in &analysed {
                for (b, tokens_b) in &analysed {
                    assert!(
                        tokens_a.iter().any(|token| tokens_b.contains(token)),
                        "{a} {tokens_a:?} and {b} {tokens_b:?} share no token"
                    );
                }
            }
        }
    }

    #[test]
    fn short_tokens_are_folded() {
        assert_eq!(analyze("pá fé PÓ"), vec!["pa", "fe", "po"]);
        assert_eq!(analyze("España"), vec!["espan"]);
    }

    #[test]
    fn accented_and_unaccented_stop_words_are_removed() {
        assert_eq!(analyze("não nao NÃO você voce contrato"), vec!["contrat"]);
        assert!(stop_set().contains(Some("nao")));
        assert!(stop_set().contains(Some("voce")));
        assert!(stop_set().contains(Some("esta")));
    }

    #[test]
    fn normalize_lowercases_and_folds() {
        let analyzer = PortugueseFoldingAnalyzer::new();
        let normalized = analyzer.normalize("f", "AÇÃO").unwrap();
        assert_eq!(
            &normalized.bytes[normalized.offset..normalized.offset + normalized.length],
            b"acao"
        );
    }

    #[test]
    fn fold_matches_the_ascii_folding_table() {
        assert_eq!(fold("Ação çÇ ñ Æ ß"), "Acao cC n AE ss");
    }
}
