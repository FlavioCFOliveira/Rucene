// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.StaticUnicodeSets`.

use std::collections::HashMap;
use std::sync::LazyLock;

use rucene::error::Result;

use crate::jdk::{index_of_unit, units};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::u_resource::{Key as ResKey, Sink, Value};
use crate::text::unicode_set::{self, UnicodeSet};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// The keys of the static sets.
///
/// Equivalent to the enum `StaticUnicodeSets.Key` (declaration order
/// kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// `EMPTY`.
    Empty,
    /// `DEFAULT_IGNORABLES`.
    DefaultIgnorables,
    /// `STRICT_IGNORABLES`.
    StrictIgnorables,
    /// `COMMA`.
    Comma,
    /// `PERIOD`.
    Period,
    /// `STRICT_COMMA`.
    StrictComma,
    /// `STRICT_PERIOD`.
    StrictPeriod,
    /// `APOSTROPHE_SIGN`.
    ApostropheSign,
    /// `OTHER_GROUPING_SEPARATORS`.
    OtherGroupingSeparators,
    /// `ALL_SEPARATORS`.
    AllSeparators,
    /// `STRICT_ALL_SEPARATORS`.
    StrictAllSeparators,
    /// `MINUS_SIGN`.
    MinusSign,
    /// `PLUS_SIGN`.
    PlusSign,
    /// `PERCENT_SIGN`.
    PercentSign,
    /// `PERMILLE_SIGN`.
    PermilleSign,
    /// `INFINITY_SIGN`.
    InfinitySign,
    /// `DOLLAR_SIGN`.
    DollarSign,
    /// `POUND_SIGN`.
    PoundSign,
    /// `RUPEE_SIGN`.
    RupeeSign,
    /// `YEN_SIGN`.
    YenSign,
    /// `WON_SIGN`.
    WonSign,
    /// `DIGITS`.
    Digits,
    /// `DIGITS_OR_ALL_SEPARATORS`.
    DigitsOrAllSeparators,
    /// `DIGITS_OR_STRICT_ALL_SEPARATORS`.
    DigitsOrStrictAllSeparators,
}

/// Frozen `UnicodeSet`s of characters that are significant to number
/// parsing, read from the root locale's `parse` data.
///
/// Equivalent to `com.ibm.icu.impl.StaticUnicodeSets` (static members
/// only). Java's `static {}` block is the `LazyLock` initializer of the
/// set map; a failure there (Java's `ExceptionInInitializerError`, an
/// `Error` ICU never catches) is a panic with the Java message (crate
/// precedent, rmp DECISION 4726).
pub struct StaticUnicodeSets;

static UNICODE_SETS: LazyLock<HashMap<Key, UnicodeSet>> = LazyLock::new(|| match init() {
    Ok(m) => m,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

impl StaticUnicodeSets {
    /// `get(Key key)`: the frozen set of `key`, or `UnicodeSet.EMPTY`
    /// when none is defined.
    pub fn get(key: Key) -> &'static UnicodeSet {
        UNICODE_SETS.get(&key).unwrap_or(&unicode_set::EMPTY)
    }

    /// `chooseFrom(String str, Key key1)`: `key1` when its set contains
    /// `str`, else `None` (Java's `null`).
    pub fn choose_from(str: &[u16], key1: Key) -> Option<Key> {
        if Self::get(key1).contains_str(str) {
            Some(key1)
        } else {
            None
        }
    }

    /// `chooseFrom(String str, Key key1, Key key2)`.
    pub fn choose_from2(str: &[u16], key1: Key, key2: Key) -> Option<Key> {
        if Self::get(key1).contains_str(str) {
            Some(key1)
        } else {
            Self::choose_from(str, key2)
        }
    }

    /// `chooseCurrency(String str)`: the currency-sign key whose set
    /// contains `str`, or `None`.
    pub fn choose_currency(str: &[u16]) -> Option<Key> {
        [Key::DollarSign, Key::PoundSign, Key::RupeeSign, Key::YenSign, Key::WonSign]
            .into_iter()
            .find(|k| Self::get(*k).contains_str(str))
    }
}

fn get_in(map: &HashMap<Key, UnicodeSet>, key: Key) -> &UnicodeSet {
    map.get(&key).unwrap_or(&unicode_set::EMPTY)
}

/// `computeUnion(Key k1, Key k2)` and `computeUnion(Key k1, Key k2, Key
/// k3)`.
fn compute_union(map: &HashMap<Key, UnicodeSet>, keys: &[Key]) -> Result<UnicodeSet> {
    let mut s = UnicodeSet::new();
    for k in keys {
        s.add_all(get_in(map, *k))?;
    }
    s.freeze();
    Ok(s)
}

fn frozen(pattern: &str) -> Result<UnicodeSet> {
    let mut s = UnicodeSet::from_pattern(&units(pattern))?;
    s.freeze();
    Ok(s)
}

/// `saveSet(Key key, String unicodeSetPattern)`.
fn save_set(map: &mut HashMap<Key, UnicodeSet>, key: Key, unicode_set_pattern: &[u16]) -> Result<()> {
    debug_assert!(!map.contains_key(&key));
    let mut s = UnicodeSet::from_pattern(unicode_set_pattern)?;
    s.freeze();
    map.insert(key, s);
    Ok(())
}

/// `StaticUnicodeSets.ParseDataSink`.
struct ParseDataSink<'a> {
    unicode_sets: &'a mut HashMap<Key, UnicodeSet>,
}

impl Sink for ParseDataSink<'_> {
    fn put(&mut self, key: &mut ResKey, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let contexts_table = value.get_table()?;
        let mut i = 0;
        while contexts_table.get_key_and_value(i, key, value) {
            if key.content_equals("date") {
                // ignore
            } else {
                debug_assert!(key.content_equals("general") || key.content_equals("number"));
                let strictness_table = value.get_table()?;
                let mut j = 0;
                while strictness_table.get_key_and_value(j, key, value) {
                    let is_lenient = key.content_equals("lenient");
                    let array = value.get_array()?;
                    for k in 0..array.get_size() {
                        array.get_value(k, value);
                        let str = value.get_string()?;
                        let has = |c: u16| index_of_unit(&str, c, 0) != -1;
                        // There is both lenient and strict data for comma/period,
                        // but not for any of the other symbols.
                        let target = if has(b'.' as u16) {
                            if is_lenient { Key::Period } else { Key::StrictPeriod }
                        } else if has(b',' as u16) {
                            if is_lenient { Key::Comma } else { Key::StrictComma }
                        } else if has(b'+' as u16) {
                            Key::PlusSign
                        } else if has(b'-' as u16) {
                            Key::MinusSign
                        } else if has(b'$' as u16) {
                            Key::DollarSign
                        } else if has(0x00A3) {
                            Key::PoundSign
                        } else if has(0x20B9) {
                            Key::RupeeSign
                        } else if has(0x00A5) {
                            Key::YenSign
                        } else if has(0x20A9) {
                            Key::WonSign
                        } else if has(b'%' as u16) {
                            Key::PercentSign
                        } else if has(0x2030) {
                            Key::PermilleSign
                        } else if has(0x2019) {
                            Key::ApostropheSign
                        } else {
                            // Unknown class of parse lenients
                            panic!(
                                "java.lang.AssertionError: Unknown class of parse lenients: {}",
                                crate::jdk::to_rust_string(&str)
                            );
                        };
                        save_set(self.unicode_sets, target, &str)?;
                    }
                    j += 1;
                }
            }
            i += 1;
        }
        Ok(())
    }
}

/// The body of Java's `static {}` block.
fn init() -> Result<HashMap<Key, UnicodeSet>> {
    let mut unicode_sets: HashMap<Key, UnicodeSet> = HashMap::new();
    unicode_sets.insert(Key::Empty, frozen("[]")?);
    // These sets were decided after discussion with icu-design@. See tickets #13084 and #13309.
    // Zs+TAB is "horizontal whitespace" according to UTS #18 (blank property).
    unicode_sets.insert(Key::DefaultIgnorables, frozen(r"[[:Zs:][\u0009][:Bidi_Control:][:Variation_Selector:]]")?);
    unicode_sets.insert(Key::StrictIgnorables, frozen("[[:Bidi_Control:]]")?);

    // CLDR provides data for comma, period, minus sign, and plus sign.
    let rb = match UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(&ULocale::root()))? {
        UResourceBundle::Icu(b) => b,
        UResourceBundle::Wrapper(_) => panic!(
            "java.lang.ClassCastException: class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle"
        ),
    };
    rb.get_all_items_with_fallback("parse", &mut ParseDataSink { unicode_sets: &mut unicode_sets })?;

    // NOTE: It is OK for these assertions to fail if there was a no-data build.
    debug_assert!(unicode_sets.contains_key(&Key::Comma));
    debug_assert!(unicode_sets.contains_key(&Key::StrictComma));
    debug_assert!(unicode_sets.contains_key(&Key::Period));
    debug_assert!(unicode_sets.contains_key(&Key::StrictPeriod));
    debug_assert!(unicode_sets.contains_key(&Key::ApostropheSign));

    let mut other_grouping = UnicodeSet::from_pattern(&units(r"[٬‘   -   　]"))?;
    let apostrophe = get_in(&unicode_sets, Key::ApostropheSign).clone();
    other_grouping.add_all(&apostrophe)?;
    other_grouping.freeze();
    unicode_sets.insert(Key::OtherGroupingSeparators, other_grouping);
    let all = compute_union(&unicode_sets, &[Key::Comma, Key::Period, Key::OtherGroupingSeparators])?;
    unicode_sets.insert(Key::AllSeparators, all);
    let strict_all = compute_union(&unicode_sets, &[Key::StrictComma, Key::StrictPeriod, Key::OtherGroupingSeparators])?;
    unicode_sets.insert(Key::StrictAllSeparators, strict_all);

    debug_assert!(unicode_sets.contains_key(&Key::MinusSign));
    debug_assert!(unicode_sets.contains_key(&Key::PlusSign));
    debug_assert!(unicode_sets.contains_key(&Key::PercentSign));
    debug_assert!(unicode_sets.contains_key(&Key::PermilleSign));

    unicode_sets.insert(Key::InfinitySign, frozen("[∞]")?);

    debug_assert!(unicode_sets.contains_key(&Key::DollarSign));
    debug_assert!(unicode_sets.contains_key(&Key::PoundSign));
    debug_assert!(unicode_sets.contains_key(&Key::RupeeSign));
    debug_assert!(unicode_sets.contains_key(&Key::YenSign));
    debug_assert!(unicode_sets.contains_key(&Key::WonSign));

    unicode_sets.insert(Key::Digits, frozen("[:digit:]")?);

    let d1 = compute_union(&unicode_sets, &[Key::Digits, Key::AllSeparators])?;
    unicode_sets.insert(Key::DigitsOrAllSeparators, d1);
    let d2 = compute_union(&unicode_sets, &[Key::Digits, Key::StrictAllSeparators])?;
    unicode_sets.insert(Key::DigitsOrStrictAllSeparators, d2);
    Ok(unicode_sets)
}
