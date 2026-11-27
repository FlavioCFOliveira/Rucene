// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RuleBasedNumberFormat`.
//!
//! The module also holds the UTF-16 `String`/`StringBuilder` helpers and the
//! `Number` accessors the RBNF classes (`NFRuleSet`, `NFRule`,
//! `NFSubstitution`, `RBNFChinesePostProcessor`, `PluralFormat`) share, with
//! the exception texts of JDK 25.

use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character::{to_lower_case_unit, to_upper_case_unit};
use rucene_analysis_common::jdk::string::trim;
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::math::{BigDecimal, BigInteger};
use crate::jdk::text::{FieldPosition, Format, ParsedObject};
use crate::jdk::{code_point_at, index_of, units, ParsePosition};
use crate::lang::u_character::UCharacter;
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_debug::ICUDebug;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::text::rbnf_scanner_provider_impl::RbnfScannerProviderImpl;
use crate::text::break_iterator::{self, BreakIterator};
use crate::text::decimal_format::DecimalFormat;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::display_context::{self, DisplayContext};
use crate::text::nf_rule::NFRule;
use crate::text::nf_rule_set::{BuildContext, NFRuleSet};
use crate::text::number_format::{self, lock, NumberFormat, NumberFormatBase, NUMBERSTYLE};
use crate::text::plural_format::PluralFormat;
use crate::text::plural_rules::PluralType;
use crate::text::rbnf_chinese_post_processor::RBNFChinesePostProcessor;
use crate::text::rbnf_lenient_scanner::RbnfLenientScanner;
use crate::text::rbnf_lenient_scanner_provider::RbnfLenientScannerProvider;
use crate::text::rbnf_post_processor::RBNFPostProcessor;
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

// ---------------------------------------------------------------------------
// UTF-16 `String` / `StringBuilder` helpers shared by the RBNF classes
// ---------------------------------------------------------------------------

/// `StringBuilder.insert(int offset, String str)` (and `insert(int, char)`
/// with a one-unit text).
///
/// # Panics
///
/// `StringIndexOutOfBoundsException("Range [<offset>, <length>) out of
/// bounds for length <length>")` when `offset` is outside `[0, length]`.
pub(crate) fn sb_insert(buf: &mut Vec<u16>, offset: i32, s: &[u16]) {
    let len = buf.len() as i32;
    if offset < 0 || offset > len {
        panic!("java.lang.StringIndexOutOfBoundsException: Range [{offset}, {len}) out of bounds for length {len}");
    }
    let o = offset as usize;
    buf.splice(o..o, s.iter().copied());
}

/// `StringBuilder.delete(int start, int end)`: `end` is clamped to the
/// length first.
///
/// # Panics
///
/// `StringIndexOutOfBoundsException("Range [<start>, <end>) out of bounds
/// for length <length>")` when `start` is negative or past the clamped end.
pub(crate) fn sb_delete(buf: &mut Vec<u16>, start: i32, end: i32) {
    let count = buf.len() as i32;
    let end = end.min(count);
    if start < 0 || start > end {
        panic!("java.lang.StringIndexOutOfBoundsException: Range [{start}, {end}) out of bounds for length {count}");
    }
    buf.drain(start as usize..end as usize);
}

/// `String.substring(int beginIndex, int endIndex)`.
///
/// # Panics
///
/// `StringIndexOutOfBoundsException("Range [<begin>, <end>) out of bounds
/// for length <length>")` for an invalid range.
pub(crate) fn substring(s: &[u16], begin: i32, end: i32) -> &[u16] {
    let len = s.len() as i32;
    if begin < 0 || begin > end || end > len {
        panic!("java.lang.StringIndexOutOfBoundsException: Range [{begin}, {end}) out of bounds for length {len}");
    }
    &s[begin as usize..end as usize]
}

/// `String.substring(int beginIndex)`.
///
/// # Panics
///
/// As [`substring`].
pub(crate) fn substring_from(s: &[u16], begin: i32) -> &[u16] {
    substring(s, begin, s.len() as i32)
}

/// `String.charAt(int index)` (and `StringBuilder.charAt`).
///
/// # Panics
///
/// `StringIndexOutOfBoundsException("Index <index> out of bounds for length
/// <length>")` outside the text.
pub(crate) fn char_at(s: &[u16], index: i32) -> u16 {
    let len = s.len() as i32;
    if index < 0 || index >= len {
        panic!("java.lang.StringIndexOutOfBoundsException: Index {index} out of bounds for length {len}");
    }
    s[index as usize]
}

/// `String.startsWith(String prefix, int toffset)`.
pub(crate) fn starts_with_at(s: &[u16], prefix: &[u16], toffset: i32) -> bool {
    if toffset < 0 || toffset as i64 > s.len() as i64 - prefix.len() as i64 {
        return false;
    }
    let t = toffset as usize;
    &s[t..t + prefix.len()] == prefix
}

/// `String.startsWith(String prefix)`.
pub(crate) fn starts_with(s: &[u16], prefix: &[u16]) -> bool {
    starts_with_at(s, prefix, 0)
}

/// `String.endsWith(String suffix)`.
pub(crate) fn ends_with(s: &[u16], suffix: &[u16]) -> bool {
    s.len() >= suffix.len() && &s[s.len() - suffix.len()..] == suffix
}

/// The access `array[index]` on a Java array of `length` elements.
///
/// # Panics
///
/// `ArrayIndexOutOfBoundsException("Index <index> out of bounds for length
/// <length>")` outside the array.
pub(crate) fn array_index(index: i64, length: usize) -> usize {
    if index < 0 || index >= length as i64 {
        panic!("java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {length}");
    }
    index as usize
}

/// `new NullPointerException()`: the error of a dereference of `null`.
pub(crate) fn npe() -> LuceneError {
    LuceneError::null_pointer()
}

/// `Math.round(double)`: the closest `long`, ties toward positive infinity,
/// saturated at the `long` range (`NaN` gives 0).
pub(crate) fn java_math_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    // floor(a + 1/2) computed exactly (`a - floor(a)` is exact), as the JDK
    // does with its bit arithmetic; the cast saturates as Java's does.
    let f = a.floor();
    if a - f >= 0.5 {
        (f as i64).saturating_add(1)
    } else {
        f as i64
    }
}

/// `Number.intValue()` of a boxed number (the narrowing conversion of each
/// `Number` class; for another `Number` subclass, the narrowing of its
/// `doubleValue()`).
pub(crate) fn number_int_value(n: &JavaNumber) -> i32 {
    match n {
        JavaNumber::Long(l) => *l as i32,
        JavaNumber::Integer(i) => *i,
        JavaNumber::Float(f) => *f as i32,
        JavaNumber::Double(d) => *d as i32,
        JavaNumber::BigInteger(b) => b.int_value(),
        JavaNumber::BigDecimal(b) => b.int_value(),
        JavaNumber::IcuBigDecimal(b) => b.int_value(),
        JavaNumber::Other { double_value, .. } => *double_value as i32,
    }
}

/// `Number.longValue()` of a boxed number (see [`number_int_value`]).
pub(crate) fn number_long_value(n: &JavaNumber) -> i64 {
    match n {
        JavaNumber::Long(l) => *l,
        JavaNumber::Integer(i) => i64::from(*i),
        JavaNumber::Float(f) => *f as i64,
        JavaNumber::Double(d) => *d as i64,
        JavaNumber::BigInteger(b) => b.long_value(),
        JavaNumber::BigDecimal(b) => b.long_value(),
        JavaNumber::IcuBigDecimal(b) => b.long_value(),
        JavaNumber::Other { double_value, .. } => *double_value as i64,
    }
}

/// The RBNF parse idiom `if (result == (long)result) return (long) result;
/// else return result;`: a `Long` when the value is integral, else a
/// `Double`.
pub(crate) fn long_or_double(result: f64) -> JavaNumber {
    if result == (result as i64) as f64 {
        JavaNumber::Long(result as i64)
    } else {
        JavaNumber::Double(result)
    }
}

/// `String.CASE_INSENSITIVE_ORDER` over UTF-16 units: each pair of
/// differing units compared through `Character.toUpperCase` and then
/// `Character.toLowerCase` (the precedent of
/// `rucene_queryparser::jdk::message_format`).
fn compare_ignore_case(a: &[u16], b: &[u16]) -> Ordering {
    for (&c1, &c2) in a.iter().zip(b) {
        if c1 != c2 {
            let u1 = to_upper_case_unit(c1);
            let u2 = to_upper_case_unit(c2);
            if u1 != u2 {
                let l1 = to_lower_case_unit(u1);
                let l2 = to_lower_case_unit(u2);
                if l1 != l2 {
                    return l1.cmp(&l2);
                }
            }
        }
    }
    a.len().cmp(&b.len())
}

// ---------------------------------------------------------------------------
// constants
// ---------------------------------------------------------------------------

/// Generated by serialver from JDK 1.4.1_01. `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = -7664252765575395068;

/// Selector code that tells the constructor to create a spellout formatter.
/// `SPELLOUT`.
pub const SPELLOUT: i32 = 1;

/// Selector code that tells the constructor to create an ordinal formatter.
/// `ORDINAL`.
pub const ORDINAL: i32 = 2;

/// Selector code that tells the constructor to create a duration formatter.
/// `DURATION` (deprecated).
pub const DURATION: i32 = 3;

/// Selector code that tells the constructor to create a numbering system
/// formatter. `NUMBERING_SYSTEM`.
pub const NUMBERING_SYSTEM: i32 = 4;

/// `rulenames` (private).
const RULENAMES: [&str; 4] = [
    "SpelloutRules",
    "OrdinalRules",
    "DurationRules",
    "NumberingSystemRules",
];

/// `locnames` (private).
const LOCNAMES: [&str; 4] = [
    "SpelloutLocalizations",
    "OrdinalLocalizations",
    "DurationLocalizations",
    "NumberingSystemLocalizations",
];

/// `DEBUG` (private): `ICUDebug.enabled("rbnf")`.
static DEBUG: LazyLock<bool> = LazyLock::new(|| ICUDebug::enabled_arg("rbnf"));

/// `MAX_VALUE` (private): `com.ibm.icu.math.BigDecimal.valueOf(Long.MAX_VALUE)`.
static MAX_VALUE: LazyLock<IcuBigDecimal> = LazyLock::new(|| IcuBigDecimal::value_of(i64::MAX));

/// `MIN_VALUE` (private): `com.ibm.icu.math.BigDecimal.valueOf(Long.MIN_VALUE)`.
static MIN_VALUE: LazyLock<IcuBigDecimal> = LazyLock::new(|| IcuBigDecimal::value_of(i64::MIN));

/// The class name a `%%post-process:` rule names, resolved as Java's
/// `Class.forName(ppClassName).newInstance()` (precedent "forName tables").
/// The only `RBNFPostProcessor` in ICU4J is `RBNFChinesePostProcessor`.
fn post_processor_for_name(class_name: &[u16]) -> Option<Box<dyn RBNFPostProcessor>> {
    if class_name == units("com.ibm.icu.text.RBNFChinesePostProcessor").as_slice() {
        Some(Box::new(RBNFChinesePostProcessor::new()))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// the rule-set graph and the formatter's fields
// ---------------------------------------------------------------------------

/// The rule sets of a formatter (`ruleSets` and `ruleSetsMap`), built once by
/// `init` and shared by every clone, as Java's shallow `clone()` shares the
/// arrays.
///
/// **Divergence.** Java's rule sets, rules and substitutions point at their
/// formatter (`NFRuleSet.owner`, `NFRule.formatter`) and substitutions at
/// other rule sets of the same formatter (`NFSubstitution.ruleSet`,
/// `ModulusSubstitution.ruleToUse`), a cyclic graph of shared mutable objects
/// that Rust's ownership forbids. The graph holds the rule sets by value;
/// substitutions name rule sets and rules by index; the formatter that built
/// the graph (Java's `owner`/`formatter`, which a clone does not change) is a
/// `Weak` reference to its [`RbnfFields`], which every formatter holding the
/// graph keeps alive; the methods of the rule classes receive the graph as a
/// parameter. Behaviour, including the clone sharing the original's rule
/// sets and their owner, is Java's.
#[derive(Debug)]
pub(crate) struct RuleGraph {
    /// `ruleSets`.
    pub(crate) rule_sets: Vec<NFRuleSet>,
    /// `ruleSetsMap`: the index of each rule set by name.
    pub(crate) rule_sets_map: HashMap<Vec<u16>, usize>,
    /// The formatter that built the rule sets (`NFRuleSet.owner`,
    /// `NFRule.formatter`).
    owner: Weak<RbnfFields>,
}

impl RuleGraph {
    /// The formatter that built the rule sets (`NFRuleSet.owner`,
    /// `NFRule.formatter`).
    pub(crate) fn owner(&self) -> Arc<RbnfFields> {
        self.owner
            .upgrade()
            .expect("the formatter that built a rule graph is kept alive by every formatter holding the graph")
    }

    /// `ruleSets[index]`.
    pub(crate) fn rule_set(&self, index: usize) -> &NFRuleSet {
        &self.rule_sets[index]
    }

    /// `findRuleSet(String name)` over this graph's `ruleSetsMap`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("No rule set named <name>")`.
    pub(crate) fn find_rule_set(&self, name: &[u16]) -> Result<usize> {
        find_rule_set_in(&self.rule_sets_map, name)
    }
}

/// `findRuleSet(String name)` over a `ruleSetsMap`.
///
/// # Errors
///
/// `IllegalArgumentException("No rule set named <name>")`.
pub(crate) fn find_rule_set_in(map: &HashMap<Vec<u16>, usize>, name: &[u16]) -> Result<usize> {
    match map.get(name) {
        Some(&i) => Ok(i),
        None => Err(LuceneError::illegal_argument(format!(
            "No rule set named {}",
            String::from_utf16_lossy(name)
        ))),
    }
}

/// The instance fields of a `RuleBasedNumberFormat` (all but those of its
/// superclass `NumberFormat`), behind one lock.
#[derive(Debug, Clone)]
struct FieldsState {
    /// `ruleSets` and `ruleSetsMap` (`None` only while `init` runs).
    rule_sets: Option<Arc<RuleGraph>>,
    /// The fields of the formatter that built `rule_sets` when it is another
    /// formatter (a clone's original, a deserialized formatter's temporary),
    /// kept alive as Java's references keep it.
    graph_owner: Option<Arc<RbnfFields>>,
    /// `defaultRuleSet`: an index into `rule_sets`.
    default_rule_set: Option<usize>,
    /// `locale`.
    locale: Option<ULocale>,
    /// `roundingMode`.
    rounding_mode: i32,
    /// `scannerProvider`.
    scanner_provider: Option<Arc<dyn RbnfLenientScannerProvider>>,
    /// `lookedForScanner`.
    looked_for_scanner: bool,
    /// `decimalFormatSymbols`.
    decimal_format_symbols: Option<Arc<DecimalFormatSymbols>>,
    /// `decimalFormat` (shared with the `PluralFormat`s of the rules, as in
    /// Java).
    decimal_format: Option<Arc<DecimalFormat>>,
    /// `defaultInfinityRule`.
    default_infinity_rule: Option<Arc<NFRule>>,
    /// `defaultNaNRule`.
    default_nan_rule: Option<Arc<NFRule>>,
    /// `lenientParse`.
    lenient_parse: bool,
    /// `lenientParseRules`.
    lenient_parse_rules: Option<Vec<u16>>,
    /// `postProcessRules`.
    post_process_rules: Option<Vec<u16>>,
    /// `postProcessor` (shared by clones, as in Java).
    post_processor: Option<Arc<Mutex<Box<dyn RBNFPostProcessor>>>>,
    /// `ruleSetDisplayNames`: locale name to localized rule set names.
    rule_set_display_names: Option<HashMap<Vec<u16>, Vec<Vec<u16>>>>,
    /// `publicRuleSetNames`.
    public_rule_set_names: Option<Vec<Vec<u16>>>,
    /// `capitalizationInfoIsSet`.
    capitalization_info_is_set: bool,
    /// `capitalizationForListOrMenu`.
    capitalization_for_list_or_menu: bool,
    /// `capitalizationForStandAlone`.
    capitalization_for_stand_alone: bool,
    /// `capitalizationBrkIter` (shared by clones, as in Java).
    capitalization_brk_iter: Option<Arc<Mutex<Box<dyn BreakIterator>>>>,
}

impl FieldsState {
    /// The field values of a newly constructed formatter (the field
    /// initializers).
    fn new(locale: Option<ULocale>) -> Self {
        FieldsState {
            rule_sets: None,
            graph_owner: None,
            default_rule_set: None,
            locale,
            rounding_mode: IcuBigDecimal::ROUND_UNNECESSARY,
            scanner_provider: None,
            looked_for_scanner: false,
            decimal_format_symbols: None,
            decimal_format: None,
            default_infinity_rule: None,
            default_nan_rule: None,
            lenient_parse: false,
            lenient_parse_rules: None,
            post_process_rules: None,
            post_processor: None,
            rule_set_display_names: None,
            public_rule_set_names: None,
            capitalization_info_is_set: false,
            capitalization_for_list_or_menu: false,
            capitalization_for_stand_alone: false,
            capitalization_brk_iter: None,
        }
    }

    /// The graph built by `init`.
    fn graph(&self) -> Arc<RuleGraph> {
        self.rule_sets
            .clone()
            .expect("init sets the rule sets before the formatter is returned")
    }

    /// `getDecimalFormatSymbols()` (package-private): lazily created for
    /// `locale` and shared by all the `DecimalFormat`s of the formatter.
    fn decimal_format_symbols(&mut self) -> Result<Arc<DecimalFormatSymbols>> {
        // lazy-evaluate the DecimalFormatSymbols object.  This object
        // is shared by all DecimalFormat instances belonging to this
        // formatter
        if self.decimal_format_symbols.is_none() {
            let locale = self.locale.as_ref().ok_or_else(npe)?;
            self.decimal_format_symbols =
                Some(Arc::new(DecimalFormatSymbols::for_ulocale(locale)?));
        }
        Ok(self.decimal_format_symbols.clone().expect("set above"))
    }

    /// `getDecimalFormat()` (package-private).
    fn decimal_format(&mut self) -> Result<Arc<DecimalFormat>> {
        if self.decimal_format.is_none() {
            // Don't use NumberFormat.getInstance, which can cause a recursive call
            let locale = self.locale.clone().ok_or_else(npe)?;
            let pattern = number_format::get_pattern(&locale, NUMBERSTYLE)?;
            let symbols = self.decimal_format_symbols()?;
            self.decimal_format = Some(Arc::new(DecimalFormat::with_symbols(
                Some(&pattern),
                &symbols,
            )?));
        }
        Ok(self.decimal_format.clone().expect("set above"))
    }

    /// `getDefaultInfinityRule()` (package-private).
    fn default_infinity_rule(&mut self) -> Result<Arc<NFRule>> {
        if self.default_infinity_rule.is_none() {
            let mut text = units("Inf: ");
            text.extend_from_slice(self.decimal_format_symbols()?.get_infinity());
            self.default_infinity_rule = Some(Arc::new(NFRule::new(Some(&text))?));
        }
        Ok(self.default_infinity_rule.clone().expect("set above"))
    }

    /// `getDefaultNaNRule()` (package-private).
    fn default_nan_rule(&mut self) -> Result<Arc<NFRule>> {
        if self.default_nan_rule.is_none() {
            let mut text = units("NaN: ");
            text.extend_from_slice(self.decimal_format_symbols()?.get_nan());
            self.default_nan_rule = Some(Arc::new(NFRule::new(Some(&text))?));
        }
        Ok(self.default_nan_rule.clone().expect("set above"))
    }

    /// `getLenientScannerProvider()`.
    fn lenient_scanner_provider(&mut self) -> Option<Arc<dyn RbnfLenientScannerProvider>> {
        // there's a potential race condition if two threads try to set/get the scanner at
        // the same time, but you get what you get, and you shouldn't be using this from
        // multiple threads anyway.
        if self.scanner_provider.is_none() && self.lenient_parse && !self.looked_for_scanner {
            self.looked_for_scanner = true;
            // Java: Class.forName("com.ibm.icu.impl.text.RbnfScannerProviderImpl")
            // .newInstance(); the class is resolved directly (precedent
            // "forName tables") and its constructor cannot fail.
            let provider: Arc<dyn RbnfLenientScannerProvider> =
                Arc::new(RbnfScannerProviderImpl::new());
            self.scanner_provider = Some(provider);
        }
        self.scanner_provider.clone()
    }
}

/// The fields of one `RuleBasedNumberFormat` object, which its rule sets
/// reach as their `owner` (see [`RuleGraph`]).
#[derive(Debug)]
pub(crate) struct RbnfFields {
    state: Mutex<FieldsState>,
}

impl RbnfFields {
    fn lock(&self) -> MutexGuard<'_, FieldsState> {
        lock(&self.state)
    }

    /// `getDecimalFormatSymbols()` (package-private).
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub(crate) fn get_decimal_format_symbols(&self) -> Result<Arc<DecimalFormatSymbols>> {
        self.lock().decimal_format_symbols()
    }

    /// `getDecimalFormat()` (package-private).
    ///
    /// # Errors
    ///
    /// The errors of the locale data and of the pattern.
    pub(crate) fn get_decimal_format(&self) -> Result<Arc<DecimalFormat>> {
        self.lock().decimal_format()
    }

    /// `getDefaultInfinityRule()` (package-private).
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub(crate) fn get_default_infinity_rule(&self) -> Result<Arc<NFRule>> {
        self.lock().default_infinity_rule()
    }

    /// `getDefaultNaNRule()` (package-private).
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub(crate) fn get_default_nan_rule(&self) -> Result<Arc<NFRule>> {
        self.lock().default_nan_rule()
    }

    /// `lenientParseEnabled()`.
    pub(crate) fn lenient_parse_enabled(&self) -> bool {
        self.lock().lenient_parse
    }

    /// `getRoundingMode()`.
    pub(crate) fn get_rounding_mode(&self) -> i32 {
        self.lock().rounding_mode
    }

    /// `getLenientScanner()` (package-private): the scanner of the provider
    /// for `locale` and the lenient-parse rules, `None` unless lenient
    /// parsing is on and a provider exists.
    ///
    /// # Errors
    ///
    /// The provider's.
    pub(crate) fn get_lenient_scanner(&self) -> Result<Option<Arc<dyn RbnfLenientScanner>>> {
        let (provider, locale, rules) = {
            let mut s = self.lock();
            if !s.lenient_parse {
                return Ok(None);
            }
            (
                s.lenient_scanner_provider(),
                s.locale.clone(),
                s.lenient_parse_rules.clone(),
            )
        };
        match provider {
            Some(provider) => {
                let locale = locale.ok_or_else(npe)?;
                Ok(Some(provider.get(&locale, rules.as_deref())?))
            }
            None => Ok(None),
        }
    }

    /// `createPluralFormat(PluralRules.PluralType pluralType, String
    /// pattern)` (package-private).
    ///
    /// # Errors
    ///
    /// The errors of the plural rules, of the decimal format and of the
    /// pattern.
    pub(crate) fn create_plural_format(
        &self,
        plural_type: PluralType,
        pattern: &[u16],
    ) -> Result<PluralFormat> {
        let (locale, decimal_format) = {
            let mut s = self.lock();
            let df = s.decimal_format()?;
            (s.locale.clone(), df)
        };
        let locale = locale.ok_or_else(npe)?;
        let nf: Arc<dyn NumberFormat> = decimal_format;
        PluralFormat::with_number_format(&locale, plural_type, pattern, Some(nf))
    }
}

// ---------------------------------------------------------------------------
// RuleBasedNumberFormat
// ---------------------------------------------------------------------------

/// A class that formats numbers according to a set of rules. This number
/// formatter is typically used for spelling out numeric values in words
/// (e.g., 25,3476 as "twenty-five thousand three hundred seventy-six" or
/// "vingt-cinq mille trois cents soixante-seize" or "funfundzwanzigtausend
/// dreihundertsechsundsiebzig"), but can also be used for other complicated
/// formatting tasks, such as formatting a number of seconds as hours,
/// minutes and seconds (e.g., 3,730 as "1:02:10").
///
/// Equivalent to `com.ibm.icu.text.RuleBasedNumberFormat` (a
/// `NumberFormat`). The `NumberFormat` state is a [`NumberFormatBase`]; the
/// fields of the class are an [`RbnfFields`] that the formatter's rule sets
/// reach as their owner (see [`RuleGraph`]). Methods take `&self`, as for
/// every `NumberFormat` of the crate. Rule texts and rule set names are
/// UTF-16 texts.
#[derive(Debug)]
pub struct RuleBasedNumberFormat {
    base: Mutex<NumberFormatBase>,
    fields: Arc<RbnfFields>,
}

impl RuleBasedNumberFormat {
    //-----------------------------------------------------------------------
    // constructors
    //-----------------------------------------------------------------------

    /// A formatter whose fields have their initial values, before `init`.
    fn with_locale_field(locale: Option<ULocale>) -> Self {
        RuleBasedNumberFormat {
            base: Mutex::new(NumberFormatBase::new()),
            fields: Arc::new(RbnfFields {
                state: Mutex::new(FieldsState::new(locale)),
            }),
        }
    }

    /// Creates a RuleBasedNumberFormat that behaves according to the
    /// description passed in. The formatter uses the default `FORMAT`
    /// locale. Equivalent to `RuleBasedNumberFormat(String description)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed description; the errors of
    /// the locale data.
    pub fn from_description(description: &[u16]) -> Result<Self> {
        let this = Self::with_locale_field(Some(ULocale::get_default_category(Category::Format)));
        this.init(description, None)?;
        Ok(this)
    }

    /// Creates a RuleBasedNumberFormat that behaves according to the
    /// description passed in, with localized display names for its public
    /// rule sets. Equivalent to `RuleBasedNumberFormat(String description,
    /// String[][] localizations)`.
    ///
    /// # Errors
    ///
    /// As [`from_description`](Self::from_description); an
    /// `IllegalArgumentException` for inconsistent localizations.
    pub fn from_description_localizations(
        description: &[u16],
        localizations: Option<&[Vec<Vec<u16>>]>,
    ) -> Result<Self> {
        let this = Self::with_locale_field(Some(ULocale::get_default_category(Category::Format)));
        this.init(description, localizations)?;
        Ok(this)
    }

    /// Equivalent to `RuleBasedNumberFormat(String description, Locale
    /// locale)`.
    ///
    /// # Errors
    ///
    /// As [`from_description`](Self::from_description).
    pub fn from_description_locale(description: &[u16], locale: &Locale) -> Result<Self> {
        Self::from_description_ulocale(description, ULocale::for_locale(Some(locale))?.as_ref())
    }

    /// Creates a RuleBasedNumberFormat that behaves according to the
    /// description passed in, using `locale` for its decimal format symbols
    /// (Java's `null` locale is `None`). Equivalent to
    /// `RuleBasedNumberFormat(String description, ULocale locale)`.
    ///
    /// # Errors
    ///
    /// As [`from_description`](Self::from_description).
    pub fn from_description_ulocale(description: &[u16], locale: Option<&ULocale>) -> Result<Self> {
        let this = Self::with_locale_field(locale.cloned());
        this.init(description, None)?;
        Ok(this)
    }

    /// Equivalent to `RuleBasedNumberFormat(String description, String[][]
    /// localizations, ULocale locale)`.
    ///
    /// # Errors
    ///
    /// As [`from_description_localizations`](Self::from_description_localizations).
    pub fn from_description_localizations_ulocale(
        description: &[u16],
        localizations: Option<&[Vec<Vec<u16>>]>,
        locale: Option<&ULocale>,
    ) -> Result<Self> {
        let this = Self::with_locale_field(locale.cloned());
        this.init(description, localizations)?;
        Ok(this)
    }

    /// Equivalent to `RuleBasedNumberFormat(Locale locale, int format)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn for_locale(locale: &Locale, format: i32) -> Result<Self> {
        let locale = ULocale::for_locale(Some(locale))?.ok_or_else(npe)?;
        Self::new(&locale, format)
    }

    /// Creates a RuleBasedNumberFormat from the predefined description
    /// associated with the passed-in locale and format selector
    /// ([`SPELLOUT`], [`ORDINAL`], [`DURATION`] or [`NUMBERING_SYSTEM`]).
    /// Equivalent to `RuleBasedNumberFormat(ULocale locale, int format)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` without RBNF data; the errors of
    /// [`from_description`](Self::from_description).
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for a selector outside `1..=4`.
    pub fn new(locale: &ULocale, format: i32) -> Result<Self> {
        let this = Self::with_locale_field(Some(locale.clone()));
        let bundle = UResourceBundle::get_bundle_instance_for(
            Some(ICUData::ICU_RBNF_BASE_NAME),
            Some(locale),
        )?;
        let bundle = bundle.as_icu()?.clone();
        // TODO: determine correct actual/valid locale.  Note ambiguity
        // here -- do actual/valid refer to pattern, DecimalFormatSymbols,
        // or Collator?
        let uloc = bundle.get_u_locale();
        this.set_locale(Some(uloc.clone()), Some(uloc))?;

        let mut description: Vec<u16> = Vec::new();
        let mut localizations: Option<Vec<Vec<Vec<u16>>>> = None;

        let read_rules = |description: &mut Vec<u16>| -> Result<()> {
            let name = RULENAMES[array_index(i64::from(format) - 1, RULENAMES.len())];
            let rules = bundle.get_with_fallback(&format!("RBNFRules/{name}"))?;
            let mut it = UResourceBundle::Icu(rules).get_iterator();
            while it.has_next() {
                description.extend_from_slice(&it.next_string()?);
            }
            Ok(())
        };
        match read_rules(&mut description) {
            Ok(()) => {}
            Err(e1) if is_missing_resource(&e1) => {}
            Err(e) => return Err(e),
        }

        // We use findTopLevel() instead of get() because
        // it's faster when we know that it's usually going to fail.
        let loc_names_bundle = UResourceBundle::Icu(bundle.clone())
            .find_top_level(LOCNAMES[array_index(i64::from(format) - 1, LOCNAMES.len())])?;
        if let Some(loc_names_bundle) = loc_names_bundle {
            let size = loc_names_bundle.get_size();
            let mut l = Vec::with_capacity(size.max(0) as usize);
            for i in 0..size {
                l.push(loc_names_bundle.get_at(i)?.get_string_array()?);
            }
            localizations = Some(l);
        }
        // else there are no localized names. It's not that important.

        this.init(&description, localizations.as_deref())?;
        Ok(this)
    }

    /// Creates a RuleBasedNumberFormat from the predefined description
    /// associated with the default `FORMAT` locale and the format selector.
    /// Equivalent to `RuleBasedNumberFormat(int format)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn for_format(format: i32) -> Result<Self> {
        Self::new(&ULocale::get_default_category(Category::Format), format)
    }

    //-----------------------------------------------------------------------
    // boilerplate
    //-----------------------------------------------------------------------

    /// Duplicates this formatter. Equivalent to `clone()`: Java's shallow
    /// copy, so that the clone shares the rule sets (whose owner stays this
    /// formatter), the decimal format, the symbols and the other referenced
    /// objects.
    pub fn clone_rbnf(&self) -> RuleBasedNumberFormat {
        let base = lock(&self.base).clone();
        let mut state = self.fields.lock().clone();
        state.graph_owner = state.rule_sets.as_ref().map(|g| g.owner());
        RuleBasedNumberFormat {
            base: Mutex::new(base),
            fields: Arc::new(RbnfFields {
                state: Mutex::new(state),
            }),
        }
    }

    /// Generates a textual description of this formatter: the descriptions
    /// of all the rule sets. Equivalent to `toString()`.
    ///
    /// # Errors
    ///
    /// The errors of the patterns of the substitutions' decimal formats.
    pub fn to_java_string(&self) -> Result<Vec<u16>> {
        // accumulate the descriptions of all the rule sets in a
        // StringBuffer, then cast it to a String and return it
        let g = self.fields.lock().graph();
        let mut result = Vec::new();
        for rule_set in &g.rule_sets {
            result.extend_from_slice(&rule_set.to_java_string(&g)?);
        }
        Ok(result)
    }

    /// The data `writeObject(ObjectOutputStream)` writes: the textual
    /// description (`writeUTF(toString())`), the locale and the rounding
    /// mode.
    ///
    /// # Errors
    ///
    /// As [`to_java_string`](Self::to_java_string).
    pub fn write_object(&self) -> Result<(Vec<u16>, Option<ULocale>, i32)> {
        // we just write the textual description to the stream, so we
        // have an implementation-independent streaming format
        let description = self.to_java_string()?;
        let s = self.fields.lock();
        Ok((description, s.locale.clone(), s.rounding_mode))
    }

    /// Deserializes a formatter: `readObject(ObjectInputStream)` on an
    /// object whose `NumberFormat` state was read as `base` and whose own
    /// fields have Java's default values (deserialization runs no field
    /// initializer). `locale` is `None` when reading it failed (the default
    /// `FORMAT` locale is used), `rounding_mode` `None` when reading it
    /// failed (the field keeps its default, 0).
    ///
    /// # Errors
    ///
    /// The errors of building a formatter from the description.
    pub fn read_object(
        base: NumberFormatBase,
        description: &[u16],
        locale: Option<ULocale>,
        rounding_mode: Option<i32>,
    ) -> Result<Self> {
        let mut state = FieldsState::new(None);
        state.rounding_mode = 0;
        // read the description in from the stream
        let loc = locale.unwrap_or_else(|| ULocale::get_default_category(Category::Format));
        if let Some(rm) = rounding_mode {
            state.rounding_mode = rm;
        }
        // build a brand-new RuleBasedNumberFormat from the description,
        // then steal its substructure.  This object's substructure and
        // the temporary RuleBasedNumberFormat drop on the floor and
        // get swept up by the garbage collector
        let temp = RuleBasedNumberFormat::from_description_ulocale(description, Some(&loc))?;
        {
            let t = temp.fields.lock();
            state.rule_sets = t.rule_sets.clone();
            state.default_rule_set = t.default_rule_set;
            state.public_rule_set_names = t.public_rule_set_names.clone();
            state.decimal_format_symbols = t.decimal_format_symbols.clone();
            state.decimal_format = t.decimal_format.clone();
            state.locale = t.locale.clone();
            state.default_infinity_rule = t.default_infinity_rule.clone();
            state.default_nan_rule = t.default_nan_rule.clone();
        }
        state.graph_owner = Some(temp.fields.clone());
        Ok(RuleBasedNumberFormat {
            base: Mutex::new(base),
            fields: Arc::new(RbnfFields {
                state: Mutex::new(state),
            }),
        })
    }

    //-----------------------------------------------------------------------
    // public API functions
    //-----------------------------------------------------------------------

    /// Returns a list of the names of all of this formatter's public rule
    /// sets. Equivalent to `getRuleSetNames()`.
    pub fn get_rule_set_names(&self) -> Vec<Vec<u16>> {
        self.fields
            .lock()
            .public_rule_set_names
            .clone()
            .expect("init sets publicRuleSetNames")
    }

    /// Returns the locales for which there are localizations for the rule
    /// set display names, sorted case-insensitively; `None` when there are
    /// none. Equivalent to `getRuleSetDisplayNameLocales()`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale`.
    pub fn get_rule_set_display_name_locales(&self) -> Result<Option<Vec<ULocale>>> {
        let s = self.fields.lock();
        if let Some(names) = &s.rule_set_display_names {
            let mut locales: Vec<&Vec<u16>> = names.keys().collect();
            locales.sort_by(|a, b| compare_ignore_case(a, b));
            let mut result = Vec::with_capacity(locales.len());
            for l in locales {
                result.push(ULocale::new(&String::from_utf16_lossy(l))?);
            }
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// `getNameListForLocale(ULocale loc)` (private).
    fn get_name_list_for_locale(&self, loc: Option<&ULocale>) -> Result<Option<Vec<Vec<u16>>>> {
        let s = self.fields.lock();
        if let (Some(loc), Some(display_names)) = (loc, &s.rule_set_display_names) {
            let locale_names = [
                loc.get_base_name(),
                ULocale::get_default_category(Category::Display).get_base_name(),
            ];
            for lname in locale_names {
                let mut lname = lname;
                while !lname.is_empty() {
                    if let Some(names) = display_names.get(&units(&lname)) {
                        return Ok(Some(names.clone()));
                    }
                    lname = ULocale::get_fallback_of(&lname)?;
                }
            }
        }
        Ok(None)
    }

    /// Returns the rule set display names in the provided locale; the rule
    /// set names without their leading `%` when none are localized.
    /// Equivalent to `getRuleSetDisplayNames(ULocale loc)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale fallback.
    pub fn get_rule_set_display_names_for(&self, loc: Option<&ULocale>) -> Result<Vec<Vec<u16>>> {
        if let Some(names) = self.get_name_list_for_locale(loc)? {
            return Ok(names);
        }
        let mut names = self.get_rule_set_names();
        for name in names.iter_mut() {
            *name = substring_from(name, 1).to_vec();
        }
        Ok(names)
    }

    /// Equivalent to `getRuleSetDisplayNames()`: the names for the default
    /// `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// As [`get_rule_set_display_names_for`](Self::get_rule_set_display_names_for).
    pub fn get_rule_set_display_names(&self) -> Result<Vec<Vec<u16>>> {
        self.get_rule_set_display_names_for(Some(&ULocale::get_default_category(Category::Display)))
    }

    /// Returns the rule set display name for the provided rule set and
    /// locale. Equivalent to `getRuleSetDisplayName(String ruleSetName,
    /// ULocale loc)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("unrecognized rule set name: <name>")`.
    pub fn get_rule_set_display_name_for(
        &self,
        rule_set_name: &[u16],
        loc: Option<&ULocale>,
    ) -> Result<Vec<u16>> {
        let rsnames = self.get_rule_set_names();
        for (ix, name) in rsnames.iter().enumerate() {
            if name.as_slice() == rule_set_name {
                if let Some(names) = self.get_name_list_for_locale(loc)? {
                    return Ok(names[array_index(ix as i64, names.len())].clone());
                }
                return Ok(substring_from(name, 1).to_vec());
            }
        }
        Err(LuceneError::illegal_argument(format!(
            "unrecognized rule set name: {}",
            String::from_utf16_lossy(rule_set_name)
        )))
    }

    /// Equivalent to `getRuleSetDisplayName(String ruleSetName)`: for the
    /// default `DISPLAY` locale.
    ///
    /// # Errors
    ///
    /// As [`get_rule_set_display_name_for`](Self::get_rule_set_display_name_for).
    pub fn get_rule_set_display_name(&self, rule_set_name: &[u16]) -> Result<Vec<u16>> {
        self.get_rule_set_display_name_for(
            rule_set_name,
            Some(&ULocale::get_default_category(Category::Display)),
        )
    }

    /// Formats the specified number according to the specified rule set.
    /// Equivalent to `format(double number, String ruleSet)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Can't use internal rule set")` for a
    /// private rule set, `IllegalArgumentException("No rule set named
    /// <name>")`; the errors of the rules.
    pub fn format_double_with_rule_set(&self, number: f64, rule_set: &[u16]) -> Result<Vec<u16>> {
        if starts_with(rule_set, &units("%%")) {
            return Err(LuceneError::illegal_argument("Can't use internal rule set"));
        }
        let rs = self.find_rule_set(rule_set)?;
        let s = self.format_double_rule_set(number, Some(rs))?;
        self.adjust_for_context(s)
    }

    /// Formats the specified number according to the specified rule set.
    /// Equivalent to `format(long number, String ruleSet)`.
    ///
    /// # Errors
    ///
    /// As [`format_double_with_rule_set`](Self::format_double_with_rule_set).
    pub fn format_long_with_rule_set(&self, number: i64, rule_set: &[u16]) -> Result<Vec<u16>> {
        if starts_with(rule_set, &units("%%")) {
            return Err(LuceneError::illegal_argument("Can't use internal rule set"));
        }
        let rs = self.find_rule_set(rule_set)?;
        let s = self.format_long_rule_set(number, Some(rs))?;
        self.adjust_for_context(s)
    }

    /// Turns lenient parse mode on and off. Equivalent to
    /// `setLenientParseMode(boolean enabled)`.
    pub fn set_lenient_parse_mode(&self, enabled: bool) {
        self.fields.lock().lenient_parse = enabled;
    }

    /// Returns true if lenient-parse mode is turned on. Equivalent to
    /// `lenientParseEnabled()`.
    pub fn lenient_parse_enabled(&self) -> bool {
        self.fields.lock().lenient_parse
    }

    /// Sets the provider for the lenient scanner (`None` to remove it).
    /// Equivalent to the deprecated `setLenientScannerProvider(...)`.
    pub fn set_lenient_scanner_provider(
        &self,
        scanner_provider: Option<Arc<dyn RbnfLenientScannerProvider>>,
    ) {
        self.fields.lock().scanner_provider = scanner_provider;
    }

    /// Returns the lenient scanner provider; when none is set and lenient
    /// parsing is on, the first call loads `RbnfScannerProviderImpl`.
    /// Equivalent to the deprecated `getLenientScannerProvider()`.
    pub fn get_lenient_scanner_provider(&self) -> Option<Arc<dyn RbnfLenientScannerProvider>> {
        self.fields.lock().lenient_scanner_provider()
    }

    /// Override the default rule set to use. `None` restores the original
    /// default rule set (the first public one, else one of the predefined
    /// names, else the last public one). Equivalent to
    /// `setDefaultRuleSet(String ruleSetName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("cannot use private rule set: <name>")`,
    /// `IllegalArgumentException("No rule set named <name>")`.
    pub fn set_default_rule_set<S: AsRef<[u16]>>(&self, rule_set_name: Option<S>) -> Result<()> {
        let mut s = self.fields.lock();
        let g = s.graph();
        match rule_set_name {
            None => {
                let public = s
                    .public_rule_set_names
                    .clone()
                    .expect("init sets publicRuleSetNames");
                if !public.is_empty() {
                    s.default_rule_set = Some(g.find_rule_set(&public[0])?);
                } else {
                    s.default_rule_set = None;
                    let mut n = g.rule_sets.len();
                    while n > 0 {
                        n -= 1;
                        let current_name = g.rule_sets[n].get_name();
                        if current_name == units("%spellout-numbering").as_slice()
                            || current_name == units("%digits-ordinal").as_slice()
                            || current_name == units("%duration").as_slice()
                        {
                            s.default_rule_set = Some(n);
                            return Ok(());
                        }
                    }
                    n = g.rule_sets.len();
                    while n > 0 {
                        n -= 1;
                        if g.rule_sets[n].is_public() {
                            s.default_rule_set = Some(n);
                            break;
                        }
                    }
                }
            }
            Some(name) => {
                let name = name.as_ref();
                if starts_with(name, &units("%%")) {
                    return Err(LuceneError::illegal_argument(format!(
                        "cannot use private rule set: {}",
                        String::from_utf16_lossy(name)
                    )));
                }
                s.default_rule_set = Some(g.find_rule_set(name)?);
            }
        }
        Ok(())
    }

    /// Return the name of the current default rule set, empty when it is
    /// not public. Equivalent to `getDefaultRuleSetName()`.
    pub fn get_default_rule_set_name(&self) -> Vec<u16> {
        let s = self.fields.lock();
        if let Some(d) = s.default_rule_set {
            let g = s.graph();
            if g.rule_sets[d].is_public() {
                return g.rule_sets[d].get_name().to_vec();
            }
        }
        Vec::new()
    }

    /// Sets the decimal format symbols used by this formatter (a copy of
    /// `new_symbols`; `None` leaves them unchanged). Equivalent to
    /// `setDecimalFormatSymbols(DecimalFormatSymbols newSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of the decimal formats and of the default rules.
    pub fn set_decimal_format_symbols(
        &self,
        new_symbols: Option<&DecimalFormatSymbols>,
    ) -> Result<()> {
        if let Some(new_symbols) = new_symbols {
            let (g, symbols) = {
                let mut s = self.fields.lock();
                let symbols = Arc::new(new_symbols.clone());
                s.decimal_format_symbols = Some(symbols.clone());
                if let Some(df) = &s.decimal_format {
                    df.set_decimal_format_symbols(&symbols)?;
                }
                if s.default_infinity_rule.is_some() {
                    s.default_infinity_rule = None;
                    s.default_infinity_rule()?; // Reset with the new DecimalFormatSymbols
                }
                if s.default_nan_rule.is_some() {
                    s.default_nan_rule = None;
                    s.default_nan_rule()?; // Reset with the new DecimalFormatSymbols
                }
                (s.graph(), symbols)
            };
            // Apply the new decimalFormatSymbols by reparsing the rulesets
            for rule_set in &g.rule_sets {
                rule_set.set_decimal_format_symbols(&symbols, &g)?;
            }
        }
        Ok(())
    }

    //-----------------------------------------------------------------------
    // package-internal API
    //-----------------------------------------------------------------------

    /// Returns a reference to the formatter's default rule set (its graph
    /// and index). Equivalent to the package-private `getDefaultRuleSet()`.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn get_default_rule_set(&self) -> (Arc<RuleGraph>, Option<usize>) {
        let s = self.fields.lock();
        (s.graph(), s.default_rule_set)
    }

    /// Returns the scanner to use for lenient parsing. Equivalent to the
    /// package-private `getLenientScanner()`.
    ///
    /// # Errors
    ///
    /// The provider's.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn get_lenient_scanner(&self) -> Result<Option<Arc<dyn RbnfLenientScanner>>> {
        self.fields.get_lenient_scanner()
    }

    /// Returns the DecimalFormatSymbols object that should be used by all
    /// DecimalFormat instances owned by this formatter. Equivalent to the
    /// package-private `getDecimalFormatSymbols()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn get_decimal_format_symbols(&self) -> Result<Arc<DecimalFormatSymbols>> {
        self.fields.get_decimal_format_symbols()
    }

    /// Equivalent to the package-private `getDecimalFormat()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data and of the pattern.
    pub(crate) fn get_decimal_format(&self) -> Result<Arc<DecimalFormat>> {
        self.fields.get_decimal_format()
    }

    /// Equivalent to the package-private `createPluralFormat(PluralType,
    /// String)`.
    ///
    /// # Errors
    ///
    /// As [`RbnfFields::create_plural_format`].
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn create_plural_format(
        &self,
        plural_type: PluralType,
        pattern: &[u16],
    ) -> Result<PluralFormat> {
        self.fields.create_plural_format(plural_type, pattern)
    }

    /// Returns the default rule for infinity. Equivalent to the
    /// package-private `getDefaultInfinityRule()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn get_default_infinity_rule(&self) -> Result<Arc<NFRule>> {
        self.fields.get_default_infinity_rule()
    }

    /// Returns the default rule for NaN. Equivalent to the package-private
    /// `getDefaultNaNRule()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    #[allow(dead_code)] // Java member with no caller in the closure
    pub(crate) fn get_default_nan_rule(&self) -> Result<Arc<NFRule>> {
        self.fields.get_default_nan_rule()
    }

    //-----------------------------------------------------------------------
    // construction implementation
    //-----------------------------------------------------------------------

    /// `extractSpecial(StringBuilder description, String specialName)`
    /// (private static): cuts the special rules named `special_name` out of
    /// the description and returns them.
    fn extract_special(description: &mut Vec<u16>, special_name: &str) -> Option<Vec<u16>> {
        let special_name = units(special_name);
        let mut result = None;
        let lp = index_of(description, &special_name, 0);
        if lp != -1 {
            // we've got to make sure we're not in the middle of a rule
            // (where specialName would actually get treated as
            // rule text)
            if lp == 0 || char_at(description, lp - 1) == u16::from(b';') {
                // locate the beginning and end of the actual special
                // rules (there may be whitespace between the name and
                // the first token in the description)
                let mut lp_end = index_of(description, &units(";%"), lp);

                if lp_end == -1 {
                    lp_end = description.len() as i32 - 1; // later we add 1 back to get the '%'
                }
                let mut lp_start = lp + special_name.len() as i32;
                while lp_start < lp_end
                    && PatternProps::is_white_space(i32::from(char_at(description, lp_start)))
                {
                    lp_start += 1;
                }

                // copy out the special rules
                result = Some(substring(description, lp_start, lp_end).to_vec());

                // remove the special rule from the description
                sb_delete(description, lp, lp_end + 1); // delete the semicolon but not the '%'
            }
        }
        result
    }

    /// `init(String description, String[][] localizations)` (private): this
    /// function parses the description and uses it to build all of the
    /// internal data structures that the formatter uses to do formatting.
    fn init(&self, description: &[u16], localizations: Option<&[Vec<Vec<u16>>]>) -> Result<()> {
        self.init_localizations(localizations)?;

        // start by stripping the trailing whitespace from all the rules
        // (this is all the whitespace following each semicolon in the
        // description).  This allows us to look for rule-set boundaries
        // by searching for ";%" without having to worry about whitespace
        // between the ; and the %
        let mut desc_buf = Self::strip_whitespace(description);

        // check to see if there's a set of lenient-parse rules.  If there
        // is, pull them out into our temporary holding place for them,
        // and delete them from the description before the real description-
        // parsing code sees them
        let lenient_parse_rules = Self::extract_special(&mut desc_buf, "%%lenient-parse:");
        let post_process_rules = Self::extract_special(&mut desc_buf, "%%post-process:");
        {
            let mut s = self.fields.lock();
            s.lenient_parse_rules = lenient_parse_rules;
            s.post_process_rules = post_process_rules;
        }

        // pre-flight parsing the description and count the number of
        // rule sets (";%" marks the end of one rule set and the beginning
        // of the next)
        let rule_set_delimiter = units(";%");
        let mut num_rule_sets = 1usize;
        let mut p = 0i32;
        loop {
            p = index_of(&desc_buf, &rule_set_delimiter, p);
            if p == -1 {
                break;
            }
            num_rule_sets += 1;
            p += 2; // Skip the length of ";%"
        }

        // our rule list is an array of the appropriate size
        let mut rule_sets: Vec<NFRuleSet> = Vec::with_capacity(num_rule_sets);
        let mut rule_sets_map: HashMap<Vec<u16>, usize> =
            HashMap::with_capacity(num_rule_sets * 2 + 1);
        let mut default_rule_set: Option<usize> = None;

        // Used to count the number of public rule sets
        // Public rule sets have names that begin with % instead of %%.
        let mut public_rule_set_count = 0usize;

        // divide up the descriptions into individual rule-set descriptions
        // and store them in a temporary array.  At each step, we also
        // create a rule set, but all this does is initialize its name
        // and remove it from its description.  We can't actually parse
        // the rest of the descriptions and finish initializing everything
        // because we have to know the names and locations of all the rule
        // sets before we can actually set everything up
        let mut rule_set_descriptions: Vec<Vec<u16>> = vec![Vec::new(); num_rule_sets];

        let mut cur_rule_set = 0usize;
        let mut start = 0i32;

        while cur_rule_set < num_rule_sets {
            p = index_of(&desc_buf, &rule_set_delimiter, start);
            if p < 0 {
                p = desc_buf.len() as i32 - 1;
            }
            rule_set_descriptions[cur_rule_set] = substring(&desc_buf, start, p + 1).to_vec();
            let rule_set = NFRuleSet::new(&mut rule_set_descriptions, cur_rule_set)?;
            let current_name = rule_set.get_name().to_vec();
            rule_sets.push(rule_set);
            rule_sets_map.insert(current_name.clone(), cur_rule_set);
            if !starts_with(&current_name, &units("%%")) {
                public_rule_set_count += 1;
                if (default_rule_set.is_none() && current_name == units("%spellout-numbering"))
                    || current_name == units("%digits-ordinal")
                    || current_name == units("%duration")
                {
                    default_rule_set = Some(cur_rule_set);
                }
            }
            cur_rule_set += 1;
            start = p + 1;
        }

        // now we can take note of the formatter's default rule set, which
        // is the last public rule set in the description (it's the last
        // rather than the first so that a user can create a new formatter
        // from an existing formatter and change its default behavior just
        // by appending more rule sets to the end)

        // {dlf} Initialization of a fraction rule set requires the default rule
        // set to be known.  For purposes of initialization, this is always the
        // last public rule set, no matter what the localization data says.

        // Set the default ruleset to the last public ruleset, unless one of the predefined
        // ruleset names %spellout-numbering, %digits-ordinal, or %duration is found

        if default_rule_set.is_none() {
            for i in (0..rule_sets.len()).rev() {
                if !starts_with(rule_sets[i].get_name(), &units("%%")) {
                    default_rule_set = Some(i);
                    break;
                }
            }
        }
        if default_rule_set.is_none() {
            default_rule_set = Some(rule_sets.len() - 1);
        }

        // Now that we know all the rule names, we can go back through
        // the temporary descriptions list and finish setting up the substructure
        for i in 0..rule_sets.len() {
            let body = {
                let ctx = BuildContext {
                    rule_sets: &rule_sets,
                    rule_sets_map: &rule_sets_map,
                    default_rule_set,
                    owner: &self.fields,
                };
                NFRuleSet::parse_rules(&ctx, i, &rule_set_descriptions[i])?
            };
            rule_sets[i].install(body);
        }

        // Now that the rules are initialized, the 'real' default rule
        // set can be adjusted by the localization data.

        // prepare an array of the proper size and copy the names into it
        let mut public_rule_set_temp: Vec<Vec<u16>> = Vec::with_capacity(public_rule_set_count);
        for i in (0..rule_sets.len()).rev() {
            if !starts_with(rule_sets[i].get_name(), &units("%%")) {
                public_rule_set_temp.push(rule_sets[i].get_name().to_vec());
            }
        }

        let public_rule_set_names = self.fields.lock().public_rule_set_names.clone();
        let public_rule_set_names = match public_rule_set_names {
            Some(names) => {
                // confirm the names, if any aren't in the rules, that's an error
                // it is ok if the rules contain public rule sets that are not in this list
                'outer: for name in &names {
                    for temp in &public_rule_set_temp {
                        if name == temp {
                            continue 'outer;
                        }
                    }
                    return Err(LuceneError::illegal_argument(format!(
                        "did not find public rule set: {}",
                        String::from_utf16_lossy(name)
                    )));
                }
                // might be different
                default_rule_set = Some(find_rule_set_in(
                    &rule_sets_map,
                    &names[array_index(0, names.len())],
                )?);
                names
            }
            None => public_rule_set_temp,
        };

        let graph = Arc::new(RuleGraph {
            rule_sets,
            rule_sets_map,
            owner: Arc::downgrade(&self.fields),
        });
        let mut s = self.fields.lock();
        s.rule_sets = Some(graph);
        s.default_rule_set = default_rule_set;
        s.public_rule_set_names = Some(public_rule_set_names);
        Ok(())
    }

    /// `initLocalizations(String[][] localizations)` (private): takes the
    /// localizations array and adds it to the localizations map.
    fn init_localizations(&self, localizations: Option<&[Vec<Vec<u16>>]>) -> Result<()> {
        if let Some(localizations) = localizations {
            let public = localizations[array_index(0, localizations.len())].clone();
            self.fields.lock().public_rule_set_names = Some(public.clone());

            let mut m: HashMap<Vec<u16>, Vec<Vec<u16>>> = HashMap::new();
            for (i, data) in localizations.iter().enumerate().skip(1) {
                let loc = data[array_index(0, data.len())].clone();
                let names: Vec<Vec<u16>> = data[1..].to_vec();
                if names.len() != public.len() {
                    return Err(LuceneError::illegal_argument(format!(
                        "public name length: {} != localized names[{}] length: {}",
                        public.len(),
                        i,
                        names.len()
                    )));
                }
                m.insert(loc, names);
            }

            if !m.is_empty() {
                self.fields.lock().rule_set_display_names = Some(m);
            }
        }
        Ok(())
    }

    /// `initCapitalizationContextInfo(ULocale theLocale)` (private): set
    /// capitalization information from the locale data.
    fn init_capitalization_context_info(state: &mut FieldsState) -> Result<()> {
        let rb = UResourceBundle::get_bundle_instance_for(
            Some(ICUData::ICU_BASE_NAME),
            state.locale.as_ref(),
        )?;
        let rb = rb.as_icu()?.clone();
        let read = || -> Result<Vec<i32>> {
            rb.get_with_fallback("contextTransforms/number-spellout")?
                .get_int_vector()
        };
        match read() {
            Ok(int_vector) => {
                if int_vector.len() >= 2 {
                    state.capitalization_for_list_or_menu = int_vector[0] != 0;
                    state.capitalization_for_stand_alone = int_vector[1] != 0;
                }
            }
            Err(e) if is_missing_resource(&e) => {
                // use default
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// `stripWhitespace(String description)` (private): returns a copy of
    /// the description with all the whitespace following each semicolon
    /// (and the empty rules) removed.
    fn strip_whitespace(description: &[u16]) -> Vec<u16> {
        // since we don't have a method that deletes characters (why?!!)
        // create a new StringBuffer to copy the text into
        let mut result = Vec::new();
        let description_length = description.len() as i32;

        // iterate through the characters...
        let mut start = 0i32;
        while start < description_length {
            // Seek to the first non-whitespace character...
            // If the first non-whitespace character is semicolon, skip it and continue
            while start < description_length {
                let ch = char_at(description, start);
                if PatternProps::is_white_space(i32::from(ch)) || ch == u16::from(b';') {
                    start += 1;
                } else {
                    break;
                }
            }

            // locate the next semicolon in the text and copy the text from
            // our current position up to that semicolon into the result
            let p = crate::jdk::index_of_unit(description, u16::from(b';'), start);
            if p == -1 {
                // or if we don't find a semicolon, just copy the rest of
                // the string into the result
                result.extend_from_slice(substring(description, start, description_length));
                break;
            } else if p < description_length {
                let end = p + 1;
                result.extend_from_slice(substring(description, start, end));
                start = end;
            }
            // when we get here from the else, we've seeked off the end of the string, and
            // we terminate the loop (we continue until *start* is -1 rather
            // than until *p* is -1, because otherwise we'd miss the last
            // rule in the description)
        }
        result
    }

    //-----------------------------------------------------------------------
    // formatting implementation
    //-----------------------------------------------------------------------

    /// `format(double number, NFRuleSet ruleSet)` (private): the formatting
    /// of a double with a rule set of this formatter's graph.
    fn format_double_rule_set(&self, number: f64, rule_set: Option<usize>) -> Result<Vec<u16>> {
        // all API format() routines that take a double vector through
        // here.  Create an empty string buffer where the result will
        // be built, and pass it to the rule set (along with an insertion
        // position of 0 and the number being formatted) to the rule set
        // for formatting
        let mut number = number;
        let mut result = Vec::new();
        let rounding_mode = self.get_rounding_mode()?;
        if rounding_mode != IcuBigDecimal::ROUND_UNNECESSARY
            && !number.is_nan()
            && !number.is_infinite()
        {
            // We convert to a string because BigDecimal insists on excessive precision.
            let field_rounding_mode = self.fields.lock().rounding_mode;
            number = IcuBigDecimal::from_string(&units(
                &rucene::util::java_fmt::double_to_string(number),
            ))?
            .set_scale_round(self.get_maximum_fraction_digits(), field_rounding_mode)?
            .double_value();
        }
        let g = self.fields.lock().graph();
        let rule_set = rule_set.ok_or_else(npe)?;
        g.rule_set(rule_set)
            .format_double(number, &mut result, 0, 0, &g)?;
        self.post_process(&mut result, Some(g.rule_set(rule_set)))?;
        Ok(result)
    }

    /// `format(long number, NFRuleSet ruleSet)` (private): the formatting
    /// of a long with a rule set of this formatter's graph.
    fn format_long_rule_set(&self, number: i64, rule_set: Option<usize>) -> Result<Vec<u16>> {
        // all API format() routines that take a double vector through
        // here.  We have these two identical functions-- one taking a
        // double and one taking a long-- the couple digits of precision
        // that long has but double doesn't (both types are 8 bytes long,
        // but double has to borrow some of the mantissa bits to hold
        // the exponent).
        // Create an empty string buffer where the result will
        // be built, and pass it to the rule set (along with an insertion
        // position of 0 and the number being formatted) to the rule set
        // for formatting
        let mut result = Vec::new();
        let g = self.fields.lock().graph();
        if number == i64::MIN {
            // We can't handle this value right now. Provide an accurate default value.
            let df = self.get_decimal_format()?;
            result.extend_from_slice(&NumberFormat::format_i64(&*df, i64::MIN)?);
        } else {
            let rule_set = rule_set.ok_or_else(npe)?;
            g.rule_set(rule_set)
                .format_long(number, &mut result, 0, 0, &g)?;
        }
        self.post_process(&mut result, rule_set.map(|i| g.rule_set(i)))?;
        Ok(result)
    }

    /// `postProcess(StringBuilder result, NFRuleSet ruleSet)` (private):
    /// post-process the rules if we have a post-processor.
    fn post_process(&self, result: &mut Vec<u16>, rule_set: Option<&NFRuleSet>) -> Result<()> {
        let (post_process_rules, post_processor) = {
            let s = self.fields.lock();
            (s.post_process_rules.clone(), s.post_processor.clone())
        };
        if let Some(post_process_rules) = post_process_rules {
            let post_processor = match post_processor {
                Some(p) => p,
                None => {
                    let mut ix = crate::jdk::index_of_unit(&post_process_rules, u16::from(b';'), 0);
                    if ix == -1 {
                        ix = post_process_rules.len() as i32;
                    }
                    let pp_class_name = trim(substring(&post_process_rules, 0, ix)).to_vec();
                    let created: Result<Box<dyn RBNFPostProcessor>> =
                        match post_processor_for_name(&pp_class_name) {
                            Some(mut p) => p.init(self, &post_process_rules).map(|()| p),
                            None => Err(LuceneError::unchecked(
                                "java.lang.ClassNotFoundException",
                                String::from_utf16_lossy(&pp_class_name),
                            )),
                        };
                    match created {
                        Ok(p) => {
                            let p = Arc::new(Mutex::new(p));
                            self.fields.lock().post_processor = Some(p.clone());
                            p
                        }
                        Err(e) => {
                            // if debug, print it out
                            if *DEBUG {
                                println!(
                                    "could not locate {}, error {}",
                                    String::from_utf16_lossy(&pp_class_name),
                                    e
                                );
                            }
                            let mut s = self.fields.lock();
                            s.post_processor = None;
                            s.post_process_rules = None; // don't try again
                            return Ok(());
                        }
                    }
                }
            };
            lock(&post_processor).process(result, rule_set)?;
        }
        Ok(())
    }

    /// `adjustForContext(String result)` (private): adjust capitalization
    /// of formatted result for display context.
    fn adjust_for_context(&self, result: Vec<u16>) -> Result<Vec<u16>> {
        let capitalization = self.get_context(display_context::Type::Capitalization);
        if capitalization != DisplayContext::CapitalizationNone
            && !result.is_empty()
            && UCharacter::is_lower_case(code_point_at(&result, 0))
        {
            let (iter, locale) = {
                let mut s = self.fields.lock();
                if capitalization == DisplayContext::CapitalizationForBeginningOfSentence
                    || (capitalization == DisplayContext::CapitalizationForUiListOrMenu
                        && s.capitalization_for_list_or_menu)
                    || (capitalization == DisplayContext::CapitalizationForStandalone
                        && s.capitalization_for_stand_alone)
                {
                    if s.capitalization_brk_iter.is_none() {
                        // should only happen when deserializing, etc.
                        let locale = s.locale.clone().ok_or_else(npe)?;
                        s.capitalization_brk_iter = Some(Arc::new(Mutex::new(
                            break_iterator::get_sentence_instance_ulocale(&locale)?,
                        )));
                    }
                    (
                        s.capitalization_brk_iter.clone().expect("set above"),
                        s.locale.clone(),
                    )
                } else {
                    return Ok(result);
                }
            };
            let mut iter = lock(&iter);
            return UCharacter::to_title_case_ulocale_options(
                locale.as_ref(),
                &result,
                Some(iter.as_mut()),
                UCharacter::TITLECASE_NO_LOWERCASE | UCharacter::TITLECASE_NO_BREAK_ADJUSTMENT,
            );
        }
        Ok(result)
    }

    /// Returns the index of the rule set with the specified name in this
    /// formatter's graph. Equivalent to the package-private
    /// `findRuleSet(String name)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("No rule set named <name>")`.
    pub(crate) fn find_rule_set(&self, name: &[u16]) -> Result<usize> {
        self.fields.lock().graph().find_rule_set(name)
    }

    /// Formats `number` with the default rule set, capitalizing for the
    /// display context when `to_append_to` is empty (the body of the
    /// inherited `format` methods).
    fn format_default<F>(&self, to_append_to: &mut Vec<u16>, format: F) -> Result<()>
    where
        F: FnOnce(&Self, Option<usize>) -> Result<Vec<u16>>,
    {
        let default_rule_set = self.fields.lock().default_rule_set;
        if to_append_to.is_empty() {
            let s = format(self, default_rule_set)?;
            to_append_to.extend_from_slice(&self.adjust_for_context(s)?);
        } else {
            // appending to other text, don't capitalize
            to_append_to.extend_from_slice(&format(self, default_rule_set)?);
        }
        Ok(())
    }
}

impl NumberFormat for RuleBasedNumberFormat {
    fn number_format_base(&self) -> &Mutex<NumberFormatBase> {
        &self.base
    }

    /// Formats the specified number using the formatter's default rule set.
    /// Equivalent to `format(double, StringBuffer, FieldPosition)`.
    fn format_double_to(
        &self,
        number: f64,
        to_append_to: &mut Vec<u16>,
        _ignore: &mut FieldPosition,
    ) -> Result<()> {
        // this is one of the inherited format() methods.  Since it doesn't
        // have a way to select the rule set to use, it just uses the
        // default one
        // Note, the BigInteger/BigDecimal methods below currently go through this.
        self.format_default(to_append_to, |this, rs| {
            this.format_double_rule_set(number, rs)
        })
    }

    /// Formats the specified number using the formatter's default rule set.
    /// Equivalent to `format(long, StringBuffer, FieldPosition)`.
    fn format_long_to(
        &self,
        number: i64,
        to_append_to: &mut Vec<u16>,
        _ignore: &mut FieldPosition,
    ) -> Result<()> {
        // this is one of the inherited format() methods.  Since it doesn't
        // have a way to select the rule set to use, it just uses the
        // default one
        self.format_default(to_append_to, |this, rs| {
            this.format_long_rule_set(number, rs)
        })
    }

    /// Equivalent to `format(BigInteger, StringBuffer, FieldPosition)`.
    fn format_big_integer_to(
        &self,
        number: &BigInteger,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.format_icu_big_decimal_to(&IcuBigDecimal::from_big_integer(number)?, to_append_to, pos)
    }

    /// Equivalent to `format(java.math.BigDecimal, StringBuffer,
    /// FieldPosition)`.
    fn format_big_decimal_to(
        &self,
        number: &BigDecimal,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.format_icu_big_decimal_to(
            &IcuBigDecimal::from_java_big_decimal(number)?,
            to_append_to,
            pos,
        )
    }

    /// Equivalent to `format(com.ibm.icu.math.BigDecimal, StringBuffer,
    /// FieldPosition)`: outside the `long` range, the decimal format.
    fn format_icu_big_decimal_to(
        &self,
        number: &IcuBigDecimal,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        if MIN_VALUE.compare_to(number)? > 0 || MAX_VALUE.compare_to(number)? < 0 {
            // We're outside of our normal range that this framework can handle.
            // The DecimalFormat will provide more accurate results.
            return self
                .get_decimal_format()?
                .format_icu_big_decimal_to(number, to_append_to, pos);
        }
        if number.scale() == 0 {
            return self.format_long_to(number.long_value(), to_append_to, pos);
        }
        self.format_double_to(number.double_value(), to_append_to, pos)
    }

    /// Parses the specified string, beginning at the specified position,
    /// according to this formatter's rules: the public rule set that
    /// consumes the most characters wins. Equivalent to `parse(String,
    /// ParsePosition)` (never `null`).
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` when the parse position is outside
    /// `text`.
    fn parse(
        &self,
        text: &[u16],
        parse_position: &mut ParsePosition,
    ) -> Result<Option<JavaNumber>> {
        // parsePosition tells us where to start parsing.  We copy the
        // text in the string from here to the end into a new string,
        // and create a new ParsePosition and result variable to use
        // for the duration of the parse operation
        let working_text = substring_from(text, parse_position.get_index());
        let mut working_pos = ParsePosition::new(0);

        // keep track of the largest number of characters consumed in
        // the various trials, and the result that corresponds to it
        let mut result = JavaNumber::Long(0);
        let mut high_water_mark = ParsePosition::new(working_pos.get_index());

        // iterate over the public rule sets (beginning with the default one)
        // and try parsing the text with each of them.  Keep track of which
        // one consumes the most characters: that's the one that determines
        // the result we return
        let g = self.fields.lock().graph();
        for rule_set in g.rule_sets.iter().rev() {
            // skip private or unparseable rule sets
            if !rule_set.is_public() || !rule_set.is_parseable() {
                continue;
            }

            // try parsing the string with the rule set.  If it gets past the
            // high-water mark, update the high-water mark and the result
            let temp_result = rule_set.parse(working_text, &mut working_pos, f64::MAX, 0, 0, &g)?;
            if working_pos.get_index() > high_water_mark.get_index() {
                result = temp_result;
                high_water_mark.set_index(working_pos.get_index());
            }
            if working_pos.get_error_index() > high_water_mark.get_error_index() {
                high_water_mark.set_error_index(working_pos.get_error_index());
            }

            // if we manage to use up all the characters in the string,
            // we don't have to try any more rule sets
            if high_water_mark.get_index() == working_text.len() as i32 {
                break;
            }

            // otherwise, reset our internal parse position to the
            // beginning and try again with the next rule set
            working_pos.set_index(0);
        }

        // add the high watermark to our original parse position and
        // return the result
        parse_position.set_index(parse_position.get_index() + high_water_mark.get_index());
        if high_water_mark.get_index() == 0 {
            parse_position
                .set_error_index(parse_position.get_index() + high_water_mark.get_error_index());
        }
        Ok(Some(result))
    }

    fn clone_number_format(&self) -> Box<dyn NumberFormat> {
        Box::new(self.clone_rbnf())
    }

    /// Sets the capitalization context, initializing the capitalization
    /// data and the sentence break iterator lazily. Equivalent to
    /// `setContext(DisplayContext context)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions Java's `setContext` lets through (the
    /// errors of the locale data and of the break iterator factory); a
    /// missing locale is Java's `NullPointerException`, a panic.
    fn set_context(&self, context: DisplayContext) -> Result<()> {
        number_format::super_set_context(self.number_format_base(), context);
        let mut s = self.fields.lock();
        if !s.capitalization_info_is_set
            && (context == DisplayContext::CapitalizationForUiListOrMenu
                || context == DisplayContext::CapitalizationForStandalone)
        {
            Self::init_capitalization_context_info(&mut s)?;
            s.capitalization_info_is_set = true;
        }
        if s.capitalization_brk_iter.is_none()
            && (context == DisplayContext::CapitalizationForBeginningOfSentence
                || (context == DisplayContext::CapitalizationForUiListOrMenu
                    && s.capitalization_for_list_or_menu)
                || (context == DisplayContext::CapitalizationForStandalone
                    && s.capitalization_for_stand_alone))
        {
            let iter = match &s.locale {
                Some(locale) => break_iterator::get_sentence_instance_ulocale(locale)?,
                None => panic!("{}", npe()),
            };
            s.capitalization_brk_iter = Some(Arc::new(Mutex::new(iter)));
        }
        Ok(())
    }

    /// Returns the rounding mode. Equivalent to `getRoundingMode()`.
    fn get_rounding_mode(&self) -> Result<i32> {
        Ok(self.fields.lock().rounding_mode)
    }

    /// Sets the rounding mode, one of the `ROUND_` constants of
    /// `com.ibm.icu.math.BigDecimal`. Equivalent to `setRoundingMode(int)`.
    fn set_rounding_mode(&self, rounding_mode: i32) -> Result<()> {
        if !(IcuBigDecimal::ROUND_UP..=IcuBigDecimal::ROUND_UNNECESSARY).contains(&rounding_mode) {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid rounding mode: {rounding_mode}"
            )));
        }
        self.fields.lock().rounding_mode = rounding_mode;
        Ok(())
    }

    /// Tests two RuleBasedNumberFormats for equality: same locale, same
    /// lenient-parse mode and equal rule sets. Equivalent to
    /// `equals(Object that)`.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when this formatter's locale is `null`.
    fn java_equals(&self, that: &dyn NumberFormat) -> bool {
        // if the other object isn't a RuleBasedNumberFormat, that's
        // all we need to know
        // Test for capitalization info equality is adequately handled
        // by the NumberFormat test for capitalizationSetting equality;
        // the info here is just derived from that.
        let Some(that2) = Format::as_any(that).downcast_ref::<RuleBasedNumberFormat>() else {
            return false;
        };
        let (my_locale, my_lenient, my_graph) = {
            let s = self.fields.lock();
            (s.locale.clone(), s.lenient_parse, s.graph())
        };
        let (that_locale, that_lenient, that_graph) = {
            let s = that2.fields.lock();
            (s.locale.clone(), s.lenient_parse, s.graph())
        };
        // compare their locales and lenient-parse modes
        let my_locale = my_locale.unwrap_or_else(|| panic!("{}", npe()));
        if Some(&my_locale) != that_locale.as_ref() || my_lenient != that_lenient {
            return false;
        }

        // if that succeeds, then compare their rule set lists
        if my_graph.rule_sets.len() != that_graph.rule_sets.len() {
            return false;
        }
        my_graph
            .rule_sets
            .iter()
            .zip(&that_graph.rule_sets)
            .all(|(a, b)| a.java_equals(b))
    }
}

impl Format for RuleBasedNumberFormat {
    fn format_object_to(
        &self,
        obj: &dyn Any,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        self.format_any_to(obj, to_append_to, pos)
    }

    fn parse_object_at(
        &self,
        source: &[u16],
        pos: &mut ParsePosition,
    ) -> Result<Option<ParsedObject>> {
        Ok(NumberFormat::parse(self, source, pos)?.map(|n| Box::new(n) as ParsedObject))
    }

    fn clone_format(&self) -> Box<dyn Format> {
        Box::new(self.clone_rbnf())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
