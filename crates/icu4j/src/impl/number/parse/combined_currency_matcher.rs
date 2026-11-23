// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.CombinedCurrencyMatcher`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::{null_pointer, units};
use crate::r#impl::number::parse::number_parse_matcher::NumberParseMatcher;
use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::string_segment::StringSegment;
use crate::r#impl::text_trie_map::{Output, TextTrieMap};
use crate::text::decimal_format_symbols::{DecimalFormatSymbols, CURRENCY_SPC_INSERT};
use crate::util::currency::{self, Currency, CurrencyStringInfo};

/// This matcher combines both parts of currency matching (the ISO code and
/// the long names) in one matcher.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.CombinedCurrencyMatcher`.
#[derive(Debug)]
pub struct CombinedCurrencyMatcher {
    iso_code: String,
    currency1: Vec<u16>,
    currency2: Vec<u16>,
    local_long_names: Option<Vec<Vec<u16>>>,
    after_prefix_insert: Vec<u16>,
    before_suffix_insert: Vec<u16>,
    long_name_trie: Option<Arc<TextTrieMap<CurrencyStringInfo>>>,
    symbol_trie: Option<Arc<TextTrieMap<CurrencyStringInfo>>>,
}

impl CombinedCurrencyMatcher {
    /// Equivalent to `getInstance(Currency, DecimalFormatSymbols, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the currency name lookups (`NullPointerException` for a
    /// `null` name the constructor dereferences).
    pub fn get_instance(currency: &Currency, dfs: &DecimalFormatSymbols, parse_flags: i32) -> Result<Arc<CombinedCurrencyMatcher>> {
        // TODO: Cache these instances. They are somewhat expensive.
        Ok(Arc::new(Self::new(currency, dfs, parse_flags)?))
    }

    fn new(currency: &Currency, dfs: &DecimalFormatSymbols, parse_flags: i32) -> Result<Self> {
        let iso_code = currency.get_subtype().ok_or_else(|| null_pointer("isoCode"))?.to_string();
        let currency1 = units(
            &currency
                .get_symbol_ulocale(&dfs.get_u_locale())?
                .ok_or_else(|| null_pointer("currency1"))?,
        );
        let currency2 = units(&currency.get_currency_code());

        let after_prefix_insert = dfs
            .get_pattern_for_currency_spacing(CURRENCY_SPC_INSERT, false)?
            .ok_or_else(|| null_pointer("afterPrefixInsert"))?
            .to_vec();
        let before_suffix_insert = dfs
            .get_pattern_for_currency_spacing(CURRENCY_SPC_INSERT, true)?
            .ok_or_else(|| null_pointer("beforeSuffixInsert"))?
            .to_vec();

        let (long_name_trie, symbol_trie, local_long_names);
        if 0 == (parse_flags & ParsingUtils::PARSE_FLAG_NO_FOREIGN_CURRENCIES) {
            // TODO: Currency trie does not currently have an option for case folding. It defaults to use
            // case folding on long-names but not symbols.
            long_name_trie = Some(Currency::get_parsing_trie(&dfs.get_u_locale(), currency::LONG_NAME)?);
            symbol_trie = Some(Currency::get_parsing_trie(&dfs.get_u_locale(), currency::SYMBOL_NAME)?);
            local_long_names = None;
        } else {
            long_name_trie = None;
            symbol_trie = None;
            let mut names = Vec::with_capacity(StandardPlural::COUNT as usize);
            for plural in StandardPlural::VALUES {
                let plural_keyword = plural.get_keyword();
                let name = currency
                    .get_name_plural_locale(dfs.get_locale(), currency::PLURAL_LONG_NAME, plural_keyword, None)?
                    .ok_or_else(|| null_pointer("localLongNames"))?;
                names.push(units(&name));
            }
            local_long_names = Some(names);
        }

        Ok(CombinedCurrencyMatcher {
            iso_code,
            currency1,
            currency2,
            local_long_names,
            after_prefix_insert,
            before_suffix_insert,
            long_name_trie,
            symbol_trie,
        })
    }

    /// Matches the currency string without concern for currency spacing.
    fn match_currency(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        let mut maybe_more = false;

        let overlap1 = if !self.currency1.is_empty() {
            segment.get_case_sensitive_prefix_length(&self.currency1)
        } else {
            -1
        };
        maybe_more = maybe_more || overlap1 == segment.length();
        if overlap1 == self.currency1.len() as i32 {
            result.currency_code = Some(self.iso_code.clone());
            segment.adjust_offset(overlap1);
            result.set_chars_consumed(segment);
            return Ok(maybe_more);
        }

        let overlap2 = if !self.currency2.is_empty() {
            segment.get_common_prefix_length(&self.currency2)
        } else {
            -1
        };
        maybe_more = maybe_more || overlap2 == segment.length();
        if overlap2 == self.currency2.len() as i32 {
            result.currency_code = Some(self.iso_code.clone());
            segment.adjust_offset(overlap2);
            result.set_chars_consumed(segment);
            return Ok(maybe_more);
        }

        if let Some(long_name_trie) = self.long_name_trie.as_ref() {
            // Use the full currency data.
            // NOTE: This call site should be improved with #13584.
            let text = segment.as_string();
            let mut trie_output = Output::default();
            let mut values = long_name_trie.get_from(&text, 0, Some(&mut trie_output))?;
            maybe_more = maybe_more || trie_output.partial_match;
            if values.is_none() {
                let symbol_trie = self.symbol_trie.as_ref().ok_or_else(|| null_pointer("symbolTrie"))?;
                values = symbol_trie.get_from(&text, 0, Some(&mut trie_output))?;
                maybe_more = maybe_more || trie_output.partial_match;
            }
            if let Some(values) = values.filter(|v| !v.is_empty()) {
                result.currency_code = Some(values[0].get_iso_code().to_string());
                segment.adjust_offset(trie_output.match_length);
                result.set_chars_consumed(segment);
                return Ok(maybe_more);
            }
        } else {
            // Use the locale long names.
            let mut longest_full_match = 0;
            let names = self.local_long_names.as_ref().ok_or_else(|| null_pointer("localLongNames"))?;
            for name in names {
                if name.is_empty() {
                    // This plural case has no data
                    continue;
                }
                let overlap = segment.get_common_prefix_length(name);
                if overlap == name.len() as i32 && name.len() as i32 > longest_full_match {
                    longest_full_match = name.len() as i32;
                }
                maybe_more = maybe_more || overlap > 0;
            }
            if longest_full_match > 0 {
                result.currency_code = Some(self.iso_code.clone());
                segment.adjust_offset(longest_full_match);
                result.set_chars_consumed(segment);
                return Ok(maybe_more);
            }
        }

        // No match found.
        Ok(maybe_more)
    }
}

impl NumberParseMatcher for CombinedCurrencyMatcher {
    fn match_(&self, segment: &mut StringSegment, result: &mut ParsedNumber) -> Result<bool> {
        if result.currency_code.is_some() {
            return Ok(false);
        }

        // Try to match a currency spacing separator.
        let initial_offset = segment.get_offset();
        let mut maybe_more = false;
        if result.seen_number() && !self.before_suffix_insert.is_empty() {
            let overlap = segment.get_common_prefix_length(&self.before_suffix_insert);
            if overlap == self.before_suffix_insert.len() as i32 {
                segment.adjust_offset(overlap);
                // Note: let currency spacing be a weak match. Don't update chars consumed.
            }
            maybe_more = maybe_more || overlap == segment.length();
        }

        // Match the currency string, and reset if we didn't find one.
        maybe_more = maybe_more || self.match_currency(segment, result)?;
        if result.currency_code.is_none() {
            segment.set_offset(initial_offset);
            return Ok(maybe_more);
        }

        // Try to match a currency spacing separator.
        if !result.seen_number() && !self.after_prefix_insert.is_empty() {
            let overlap = segment.get_common_prefix_length(&self.after_prefix_insert);
            if overlap == self.after_prefix_insert.len() as i32 {
                segment.adjust_offset(overlap);
                // Note: let currency spacing be a weak match. Don't update chars consumed.
            }
            maybe_more = maybe_more || overlap == segment.length();
        }

        Ok(maybe_more)
    }

    fn smoke_test(&self, _segment: &StringSegment) -> bool {
        // TODO: See constructor
        true
        // return segment.startsWith(uniSet);
    }

    fn post_process(&self, _result: &mut ParsedNumber) -> Result<()> {
        // No-op
        Ok(())
    }

    fn to_java_string(&self) -> String {
        format!("<CombinedCurrencyMatcher {}>", self.iso_code)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
