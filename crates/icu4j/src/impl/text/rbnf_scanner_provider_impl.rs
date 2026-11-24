// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.text.RbnfScannerProviderImpl`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;

use crate::r#impl::icu_debug::ICUDebug;
use crate::text::collation_element_iterator::{CollationElementIterator, NULLORDER};
use crate::text::collator::{self, Collator, CANONICAL_DECOMPOSITION};
use crate::text::number_format::lock;
use crate::text::rbnf_lenient_scanner::RbnfLenientScanner;
use crate::text::rbnf_lenient_scanner_provider::RbnfLenientScannerProvider;
use crate::text::rule_based_collator::RuleBasedCollator;
use crate::text::rule_based_number_format::substring_from;
use crate::util::u_locale::ULocale;

/// `DEBUG` (private static): `ICUDebug.enabled("rbnf")`.
static DEBUG: LazyLock<bool> = LazyLock::new(|| ICUDebug::enabled_arg("rbnf"));

/// Returns RbnfLenientScanners that use the old RuleBasedNumberFormat
/// implementation behind setLenientParseMode, which is based on Collator.
///
/// Equivalent to the deprecated internal class
/// `com.ibm.icu.impl.text.RbnfScannerProviderImpl` (an
/// `RbnfLenientScannerProvider`).
#[derive(Debug, Default)]
pub struct RbnfScannerProviderImpl {
    /// `cache`: the scanners by `locale + "/" + extras`.
    cache: Mutex<HashMap<String, Arc<dyn RbnfLenientScanner>>>,
}

impl RbnfScannerProviderImpl {
    /// Equivalent to the deprecated `RbnfScannerProviderImpl()`.
    pub fn new() -> Self {
        RbnfScannerProviderImpl {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Creates a collation-based scanner: the locale's collator, its rules
    /// extended with `extras` when given, with canonical decomposition; a
    /// scanner without collator when that fails. Equivalent to the
    /// deprecated protected `createScanner(ULocale locale, String extras)`.
    pub fn create_scanner(
        &self,
        locale: &ULocale,
        extras: Option<&[u16]>,
    ) -> Arc<dyn RbnfLenientScanner> {
        // create a default collator based on the locale,
        // then pull out that collator's rules, append any additional
        // rules specified in the description, and create a _new_
        // collator based on the combination of those rules
        let create = || -> Result<Option<Box<dyn Collator>>> {
            let mut collator = collator::get_instance_locale(Some(&locale.to_locale()))?;
            // (RuleBasedCollator)Collator.getInstance(...): a ClassCastException
            // when the collator is not rule-based.
            if collator.as_rule_based().is_none() {
                return Ok(None);
            }
            if let Some(extras) = extras {
                let mut rules = collator
                    .as_rule_based()
                    .expect("checked above")
                    .get_rules()?;
                rules.extend_from_slice(extras);
                collator = Box::new(RuleBasedCollator::new(Some(&rules))?);
            }
            collator.set_decomposition(CANONICAL_DECOMPOSITION)?;
            Ok(Some(collator))
        };
        let collator = match create() {
            Ok(Some(c)) => Some(c),
            Ok(None) => {
                // the ClassCastException, caught as any Exception
                if *DEBUG {
                    // debug hook
                    eprintln!("java.lang.ClassCastException");
                    println!("++++");
                }
                None
            }
            Err(e) => {
                // If we get here, it means we have a malformed set of
                // collation rules, which hopefully won't happen
                //CLOVER:OFF
                if *DEBUG {
                    // debug hook
                    eprintln!("{e}");
                    println!("++++");
                }
                //CLOVER:ON
                None
            }
        };

        Arc::new(RbnfLenientScannerImpl { collator })
    }
}

impl RbnfLenientScannerProvider for RbnfScannerProviderImpl {
    /// Returns a collation-based scanner, cached per locale and extras.
    ///
    /// Only primary differences are treated as significant. This means that
    /// case differences, accent differences, alternate spellings of the
    /// same letter (e.g., ae and a-umlaut in German), ignorable characters,
    /// etc. are ignored in matching the text. In many cases, numerals will
    /// be accepted in place of words or phrases as well.
    ///
    /// Equivalent to the deprecated `get(ULocale locale, String extras)`.
    fn get(&self, locale: &ULocale, extras: Option<&[u16]>) -> Result<Arc<dyn RbnfLenientScanner>> {
        let key = format!(
            "{}/{}",
            locale,
            extras.map_or_else(|| "null".to_string(), String::from_utf16_lossy)
        );
        {
            let cache = lock(&self.cache);
            if let Some(result) = cache.get(&key) {
                return Ok(result.clone());
            }
        }
        let result = self.create_scanner(locale, extras);
        lock(&self.cache).insert(key, result.clone());
        Ok(result)
    }
}

/// `RbnfLenientScannerImpl` (private static): the scanner over a
/// rule-based collator.
///
/// The collator is `None` when its creation failed, as Java's `null`; the
/// methods then throw `NullPointerException`, a panic here (the trait's
/// signatures are Java's).
#[derive(Debug)]
struct RbnfLenientScannerImpl {
    collator: Option<Box<dyn Collator>>,
}

impl RbnfLenientScannerImpl {
    /// The rule-based collator (Java's dereference of `collator`).
    fn collator(&self) -> &RuleBasedCollator {
        match self.collator.as_ref().and_then(|c| c.as_rule_based()) {
            Some(c) => c,
            None => panic!("java.lang.NullPointerException"),
        }
    }

    /// The following method contains the same signature as findText and
    /// has never been used by anything once. Equivalent to
    /// `findText2(String str, String key, int startingAt)`.
    #[allow(dead_code)]
    fn find_text2(&self, str: &[u16], key: &[u16], starting_at: i32) -> [i32; 2] {
        //CLOVER:OFF
        let collator = self.collator();
        let mut str_iter = collator.get_collation_element_iterator(str);
        let mut key_iter = collator.get_collation_element_iterator(key);

        let mut key_start = -1;

        str_iter.set_offset(starting_at);

        let mut o_str = str_iter.next();
        let mut o_key = key_iter.next();
        while o_key != NULLORDER {
            while o_str != NULLORDER && CollationElementIterator::primary_order(o_str) == 0 {
                o_str = str_iter.next();
            }

            while o_key != NULLORDER && CollationElementIterator::primary_order(o_key) == 0 {
                o_key = key_iter.next();
            }

            if o_str == NULLORDER {
                return [-1, 0];
            }

            if o_key == NULLORDER {
                break;
            }

            if CollationElementIterator::primary_order(o_str)
                == CollationElementIterator::primary_order(o_key)
            {
                key_start = str_iter.get_offset();
                o_str = str_iter.next();
                o_key = key_iter.next();
            } else if key_start != -1 {
                key_start = -1;
                key_iter.reset();
            } else {
                o_str = str_iter.next();
            }
        }

        [key_start, str_iter.get_offset() - key_start]
        //CLOVER:ON
    }
}

impl RbnfLenientScanner for RbnfLenientScannerImpl {
    fn all_ignorable(&self, s: &[u16]) -> bool {
        let mut iter = self.collator().get_collation_element_iterator(s);

        let mut o = iter.next();
        while o != NULLORDER && CollationElementIterator::primary_order(o) == 0 {
            o = iter.next();
        }
        o == NULLORDER
    }

    fn find_text(&self, str: &[u16], key: &[u16], starting_at: i32) -> [i32; 2] {
        let mut p = starting_at;
        let mut key_len = 0;

        // basically just isolate smaller and smaller substrings of
        // the target string (each running to the end of the string,
        // and with the first one running from startingAt to the end)
        // and then use prefixLength() to see if the search key is at
        // the beginning of each substring.  This is excruciatingly
        // slow, but it will locate the key and tell use how long the
        // matching text was.
        while p < str.len() as i32 && key_len == 0 {
            key_len = self.prefix_length(substring_from(str, p), key);
            if key_len != 0 {
                return [p, key_len];
            }
            p += 1;
        }
        // if we make it to here, we didn't find it.  Return -1 for the
        // location.  The length should be ignored, but set it to 0,
        // which should be "safe"
        [-1, 0]
    }

    fn prefix_length(&self, str: &[u16], prefix: &[u16]) -> i32 {
        // Create two collation element iterators, one over the target string
        // and another over the prefix.
        //
        // Previous code was matching "fifty-" against " fifty" and leaving
        // the number " fifty-7" to parse as 43 (50 - 7).
        // Also it seems that if we consume the entire prefix, that's ok even
        // if we've consumed the entire string, so I switched the logic to
        // reflect this.

        let collator = self.collator();
        let mut str_iter = collator.get_collation_element_iterator(str);
        let mut prefix_iter = collator.get_collation_element_iterator(prefix);

        // match collation elements between the strings
        let mut o_str = str_iter.next();
        let mut o_prefix = prefix_iter.next();

        while o_prefix != NULLORDER {
            // skip over ignorable characters in the target string
            while CollationElementIterator::primary_order(o_str) == 0 && o_str != NULLORDER {
                o_str = str_iter.next();
            }

            // skip over ignorable characters in the prefix
            while CollationElementIterator::primary_order(o_prefix) == 0 && o_prefix != NULLORDER {
                o_prefix = prefix_iter.next();
            }

            // if skipping over ignorables brought to the end of
            // the prefix, we DID match: drop out of the loop
            if o_prefix == NULLORDER {
                break;
            }

            // if skipping over ignorables brought us to the end
            // of the target string, we didn't match and return 0
            if o_str == NULLORDER {
                return 0;
            }

            // match collation elements from the two strings
            // (considering only primary differences).  If we
            // get a mismatch, dump out and return 0
            if CollationElementIterator::primary_order(o_str)
                != CollationElementIterator::primary_order(o_prefix)
            {
                return 0;
            }

            // otherwise, advance to the next character in each string
            // and loop (we drop out of the loop when we exhaust
            // collation elements in the prefix)

            o_str = str_iter.next();
            o_prefix = prefix_iter.next();
        }

        let mut result = str_iter.get_offset();
        if o_str != NULLORDER {
            result -= 1;
        }
        result
    }
}
