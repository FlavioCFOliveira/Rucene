// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `java.util.Currency`, emulated for the ICU members that convert to and
//! from it (`com.ibm.icu.util.Currency.toJavaCurrency`, `fromJavaCurrency`,
//! the `CurrencyAmount(…, java.util.Currency)` constructors).
//!
//! JDK 25's `java.util.Currency` (`Currency.java`, read as specification
//! only) loads, in its static initializer, the generated resource
//! `/java/util/currency.data` (built by `GenerateCurrencyData` from
//! `CurrencyData.properties`): a format-3 table of `dataVersion` 180 holding
//! the 26 × 26 `mainTable`, 16 special cases and 82 other currencies. The
//! three tables below are that resource's content, read field by field from
//! the `java.base` module image of JDK 25.0.5 (`jrt:/modules/java.base/
//! java/util/currency.data`, 4,289 bytes), and the lookups are those of
//! `Currency.java`, including the time-dependent special case (Bulgaria:
//! `BGN` until `1767218400000` ms, 2025-12-31T22:00:00Z, then `EUR`), read
//! against the system clock as `System.currentTimeMillis()`.
//!
//! **Divergence.** `Currency.initStatic` then applies the overrides of the
//! file the system property `java.util.currency.data` names, or of
//! `${java.home}/lib/currency.properties`; a JDK installation ships no such
//! file (absent in the JDK 25.0.5 oracle, and the property unset), and this
//! crate has neither system properties nor a `java.home`, so the port is the
//! JDK without an override file. The locale-sensitive members `getSymbol`
//! and `getDisplayName` (`LocaleServiceProviderPool` over the CLDR currency
//! names) are not emulated: no ported caller reaches them.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

/// `INVALID_COUNTRY_ENTRY`.
const INVALID_COUNTRY_ENTRY: i32 = 0x0000_007F;
/// `COUNTRY_WITHOUT_CURRENCY_ENTRY`.
const COUNTRY_WITHOUT_CURRENCY_ENTRY: i32 = 0x0000_0200;
/// `SIMPLE_CASE_COUNTRY_MASK`.
const SIMPLE_CASE_COUNTRY_MASK: i32 = 0x0000_0000;
/// `SIMPLE_CASE_COUNTRY_FINAL_CHAR_MASK`.
const SIMPLE_CASE_COUNTRY_FINAL_CHAR_MASK: i32 = 0x0000_001F;
/// `SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_MASK`.
const SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_MASK: i32 = 0x0000_01E0;
/// `SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_SHIFT`.
const SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_SHIFT: i32 = 5;
/// `SPECIAL_CASE_COUNTRY_MASK`.
const SPECIAL_CASE_COUNTRY_MASK: i32 = 0x0000_0200;
/// `SPECIAL_CASE_COUNTRY_INDEX_MASK`.
const SPECIAL_CASE_COUNTRY_INDEX_MASK: i32 = 0x0000_001F;
/// `SPECIAL_CASE_COUNTRY_INDEX_DELTA`.
const SPECIAL_CASE_COUNTRY_INDEX_DELTA: i32 = 1;
/// `COUNTRY_TYPE_MASK`.
const COUNTRY_TYPE_MASK: i32 = SIMPLE_CASE_COUNTRY_MASK | SPECIAL_CASE_COUNTRY_MASK;
/// `NUMERIC_CODE_MASK`.
const NUMERIC_CODE_MASK: i32 = 0x000F_FC00;
/// `NUMERIC_CODE_SHIFT`.
const NUMERIC_CODE_SHIFT: i32 = 10;
/// `A_TO_Z`.
const A_TO_Z: i32 = (b'Z' - b'A') as i32 + 1;

/// `formatVersion` of the loaded `currency.data`.
pub const FORMAT_VERSION: i32 = 3;
/// `dataVersion` of the loaded `currency.data`.
pub const DATA_VERSION: i32 = 180;

/// `Currency.SpecialCaseEntry`.
#[derive(Debug)]
struct SpecialCaseEntry {
    cut_over_time: i64,
    old_currency: &'static str,
    new_currency: &'static str,
    old_currency_fraction: i32,
    new_currency_fraction: i32,
    old_currency_numeric_code: i32,
    new_currency_numeric_code: i32,
}

impl SpecialCaseEntry {
    const fn new(
        cut_over_time: i64,
        old_currency: &'static str,
        new_currency: &'static str,
        old_currency_fraction: i32,
        new_currency_fraction: i32,
        old_currency_numeric_code: i32,
        new_currency_numeric_code: i32,
    ) -> Self {
        SpecialCaseEntry {
            cut_over_time,
            old_currency,
            new_currency,
            old_currency_fraction,
            new_currency_fraction,
            old_currency_numeric_code,
            new_currency_numeric_code,
        }
    }

    /// `SpecialCaseEntry.findEntry(String)`: `[fraction, numericCode]`.
    fn find_entry(code: &str) -> Option<[i32; 2]> {
        for sc_entry in &SPECIAL_CASES {
            if sc_entry.old_currency == code
                && (sc_entry.cut_over_time == i64::MAX || current_time_millis() < sc_entry.cut_over_time)
            {
                return Some([sc_entry.old_currency_fraction, sc_entry.old_currency_numeric_code]);
            } else if sc_entry.new_currency == code && current_time_millis() >= sc_entry.cut_over_time {
                return Some([sc_entry.new_currency_fraction, sc_entry.new_currency_numeric_code]);
            }
        }
        None
    }

    /// `SpecialCaseEntry.toIndex(int)`.
    fn to_index(table_entry: i32) -> usize {
        ((table_entry & SPECIAL_CASE_COUNTRY_INDEX_MASK) - SPECIAL_CASE_COUNTRY_INDEX_DELTA) as usize
    }
}

/// `Currency.OtherCurrencyEntry`.
#[derive(Debug)]
struct OtherCurrencyEntry {
    currency_code: &'static str,
    fraction: i32,
    numeric_code: i32,
}

impl OtherCurrencyEntry {
    const fn new(currency_code: &'static str, fraction: i32, numeric_code: i32) -> Self {
        OtherCurrencyEntry { currency_code, fraction, numeric_code }
    }

    /// `OtherCurrencyEntry.findEntry(String)`: `equalsIgnoreCase`.
    fn find_entry(code: &[u16]) -> Option<&'static OtherCurrencyEntry> {
        OTHER_CURRENCIES.iter().find(|oc_entry| equals_ignore_case(oc_entry.currency_code, code))
    }
}

/// `String.equalsIgnoreCase` of an ASCII table code against UTF-16 units.
fn equals_ignore_case(table: &str, code: &[u16]) -> bool {
    let table: Vec<u16> = table.encode_utf16().collect();
    table.len() == code.len()
        && table.iter().zip(code).all(|(&a, &b)| {
            if a == b {
                return true;
            }
            // `String.regionMatches(true, …)`: compare `toUpperCase`, then
            // `toLowerCase` of the upper-cased units.
            let (ua, ub) = (to_upper(a), to_upper(b));
            ua == ub || to_lower(ua) == to_lower(ub)
        })
}

fn to_upper(c: u16) -> u16 {
    char::from_u32(u32::from(c))
        .map(|ch| {
            let mut up = ch.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) if (u as u32) <= 0xFFFF => u as u32 as u16,
                _ => c,
            }
        })
        .unwrap_or(c)
}

fn to_lower(c: u16) -> u16 {
    char::from_u32(u32::from(c))
        .map(|ch| {
            let mut low = ch.to_lowercase();
            match (low.next(), low.next()) {
                (Some(l), None) if (l as u32) <= 0xFFFF => l as u32 as u16,
                _ => c,
            }
        })
        .unwrap_or(c)
}

/// `System.currentTimeMillis()`.
fn current_time_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// `Currency.instances`: one object per currency code string.
static INSTANCES: LazyLock<Mutex<HashMap<Vec<u16>, Currency>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
struct CurrencyData {
    currency_code: Vec<u16>,
    default_fraction_digits: i32,
    numeric_code: i32,
}

/// Represents a currency. Currencies are identified by their ISO 4217
/// currency codes.
///
/// Equivalent to `java.util.Currency` (JDK 25). Java guarantees at most one
/// instance per currency code (`instances`), and `Currency` does not override
/// `equals`/`hashCode`, so identity is the equality: here the shared
/// instance is an `Arc`, compared and hashed by pointer.
#[derive(Clone)]
pub struct Currency(Arc<CurrencyData>);

impl PartialEq for Currency {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Currency {}

impl std::hash::Hash for Currency {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Currency").field(&String::from_utf16_lossy(&self.0.currency_code)).finish()
    }
}

impl fmt::Display for Currency {
    /// `toString()`: the currency code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf16_lossy(&self.0.currency_code))
    }
}

impl Currency {
    /// Returns the `Currency` instance for the given currency code.
    /// Equivalent to `getInstance(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the code is not 3 UTF-16 units long,
    /// when its first two units are not `A`–`Z`, or when it is not a valid
    /// ISO 4217 code of the data.
    pub fn get_instance(currency_code: &str) -> Result<Currency> {
        let units: Vec<u16> = currency_code.encode_utf16().collect();
        Self::get_instance_impl(&units, i32::MIN, 0)
    }

    /// `getInstance(String, int, int)`.
    fn get_instance_impl(currency_code: &[u16], mut default_fraction_digits: i32, mut numeric_code: i32) -> Result<Currency> {
        if let Some(instance) = INSTANCES.lock().unwrap_or_else(|e| e.into_inner()).get(currency_code) {
            return Ok(instance.clone());
        }
        if default_fraction_digits == i32::MIN {
            let mut found = false;
            if currency_code.len() != 3 {
                return Err(LuceneError::illegal_argument(format!(
                    "The input currency code: \"{}\" must have a length of 3 characters",
                    String::from_utf16_lossy(currency_code)
                )));
            }
            let char1 = currency_code[0];
            let char2 = currency_code[1];
            let table_entry = get_main_table_entry(char1, char2)?;
            if (table_entry & COUNTRY_TYPE_MASK) == SIMPLE_CASE_COUNTRY_MASK
                && table_entry != INVALID_COUNTRY_ENTRY
                && i32::from(currency_code[2]) - i32::from(b'A') == (table_entry & SIMPLE_CASE_COUNTRY_FINAL_CHAR_MASK)
            {
                default_fraction_digits =
                    (table_entry & SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_MASK) >> SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_SHIFT;
                numeric_code = (table_entry & NUMERIC_CODE_MASK) >> NUMERIC_CODE_SHIFT;
                found = true;
            } else {
                // special case
                if let Some(fraction_and_numeric_code) =
                    SpecialCaseEntry::find_entry(&String::from_utf16_lossy(currency_code))
                {
                    default_fraction_digits = fraction_and_numeric_code[0];
                    numeric_code = fraction_and_numeric_code[1];
                    found = true;
                }
            }
            if !found {
                let Some(oc_entry) = OtherCurrencyEntry::find_entry(currency_code) else {
                    return Err(LuceneError::illegal_argument(format!(
                        "The input currency code: \"{}\" is not a valid ISO 4217 code",
                        String::from_utf16_lossy(currency_code)
                    )));
                };
                default_fraction_digits = oc_entry.fraction;
                numeric_code = oc_entry.numeric_code;
            }
        }
        let currency_val = Currency(Arc::new(CurrencyData {
            currency_code: currency_code.to_vec(),
            default_fraction_digits,
            numeric_code,
        }));
        let mut instances = INSTANCES.lock().unwrap_or_else(|e| e.into_inner());
        Ok(instances.entry(currency_code.to_vec()).or_insert(currency_val).clone())
    }

    /// Returns the `Currency` instance for the country of the given locale.
    /// Equivalent to `getInstance(Locale)`; `None` is Java's `null` for a
    /// country without a currency (Antarctica).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the locale's country (after the `rg`
    /// region override) is not a valid ISO 3166 code of the data.
    pub fn get_instance_for_locale(locale: &Locale) -> Result<Option<Currency>> {
        if let Some(override_) = locale.get_unicode_locale_type("cu") {
            // `toUpperCase(Locale.ROOT)`; an unknown code falls through.
            if let Ok(c) = Self::get_instance(&override_.to_uppercase()) {
                return Ok(Some(c));
            }
        }
        let region_locale = find_region_override(locale)?;
        let country = region_locale.get_country();
        if !(country.len() == 2 && country.bytes().all(|b| b.is_ascii_alphabetic())) {
            return Err(LuceneError::illegal_argument(format!(
                "The country of the input locale: \"{locale}\" is not a valid ISO 3166 country code"
            )));
        }
        let units: Vec<u16> = country.encode_utf16().collect();
        let (char1, char2) = (units[0], units[1]);
        let table_entry = get_main_table_entry(char1, char2)?;
        if (table_entry & COUNTRY_TYPE_MASK) == SIMPLE_CASE_COUNTRY_MASK && table_entry != INVALID_COUNTRY_ENTRY {
            let final_char = ((table_entry & SIMPLE_CASE_COUNTRY_FINAL_CHAR_MASK) + i32::from(b'A')) as u16;
            let default_fraction_digits =
                (table_entry & SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_MASK) >> SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_SHIFT;
            let numeric_code = (table_entry & NUMERIC_CODE_MASK) >> NUMERIC_CODE_SHIFT;
            let mut sb = units.clone();
            sb.push(final_char);
            Self::get_instance_impl(&sb, default_fraction_digits, numeric_code).map(Some)
        } else {
            if table_entry == INVALID_COUNTRY_ENTRY {
                return Err(LuceneError::illegal_argument(format!(
                    "The country of the input locale: \"{locale}\" is not a valid ISO 3166 country code"
                )));
            }
            if table_entry == COUNTRY_WITHOUT_CURRENCY_ENTRY {
                Ok(None)
            } else {
                let sc_entry = &SPECIAL_CASES[SpecialCaseEntry::to_index(table_entry)];
                if sc_entry.cut_over_time == i64::MAX || current_time_millis() < sc_entry.cut_over_time {
                    Self::get_instance_impl(
                        &units_of(sc_entry.old_currency),
                        sc_entry.old_currency_fraction,
                        sc_entry.old_currency_numeric_code,
                    )
                    .map(Some)
                } else {
                    Self::get_instance_impl(
                        &units_of(sc_entry.new_currency),
                        sc_entry.new_currency_fraction,
                        sc_entry.new_currency_numeric_code,
                    )
                    .map(Some)
                }
            }
        }
    }

    /// Gets the set of available currencies. Equivalent to
    /// `getAvailableCurrencies()` (`computeAllCurrencies`).
    ///
    /// **Divergence.** Java answers a `HashSet` iterated in identity-hash
    /// order (unspecified); the port answers the distinct currencies in the
    /// order `computeAllCurrencies` adds them.
    pub fn get_available_currencies() -> Vec<Currency> {
        let sys_time = current_time_millis();
        let mut available: Vec<Currency> = Vec::with_capacity(256);
        let mut add = |c: Result<Currency>| {
            // Every table code is valid: `getInstance` cannot fail here.
            if let Ok(c) = c {
                if !available.contains(&c) {
                    available.push(c);
                }
            }
        };
        for c1 in b'A'..=b'Z' {
            for c2 in b'A'..=b'Z' {
                let table_entry = MAIN_TABLE[usize::from(c1 - b'A') * A_TO_Z as usize + usize::from(c2 - b'A')];
                if (table_entry & COUNTRY_TYPE_MASK) == SIMPLE_CASE_COUNTRY_MASK && table_entry != INVALID_COUNTRY_ENTRY {
                    let final_char = ((table_entry & SIMPLE_CASE_COUNTRY_FINAL_CHAR_MASK) + i32::from(b'A')) as u16;
                    let default_fraction_digits =
                        (table_entry & SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_MASK) >> SIMPLE_CASE_COUNTRY_DEFAULT_DIGITS_SHIFT;
                    let numeric_code = (table_entry & NUMERIC_CODE_MASK) >> NUMERIC_CODE_SHIFT;
                    let sb = [u16::from(c1), u16::from(c2), final_char];
                    add(Self::get_instance_impl(&sb, default_fraction_digits, numeric_code));
                } else if (table_entry & COUNTRY_TYPE_MASK) == SPECIAL_CASE_COUNTRY_MASK
                    && table_entry != INVALID_COUNTRY_ENTRY
                    && table_entry != COUNTRY_WITHOUT_CURRENCY_ENTRY
                {
                    let sc_entry = &SPECIAL_CASES[SpecialCaseEntry::to_index(table_entry)];
                    if sc_entry.cut_over_time == i64::MAX || sys_time < sc_entry.cut_over_time {
                        add(Self::get_instance_impl(
                            &units_of(sc_entry.old_currency),
                            sc_entry.old_currency_fraction,
                            sc_entry.old_currency_numeric_code,
                        ));
                    } else {
                        add(Self::get_instance_impl(
                            &units_of(sc_entry.new_currency),
                            sc_entry.new_currency_fraction,
                            sc_entry.new_currency_numeric_code,
                        ));
                    }
                }
            }
        }
        for entry in &OTHER_CURRENCIES {
            add(Self::get_instance(entry.currency_code));
        }
        available
    }

    /// Gets the ISO 4217 currency code of this currency. Equivalent to
    /// `getCurrencyCode()`.
    pub fn get_currency_code(&self) -> String {
        String::from_utf16_lossy(&self.0.currency_code)
    }

    /// Gets the default number of fraction digits used with this currency
    /// (`-1` for pseudo-currencies). Equivalent to `getDefaultFractionDigits()`.
    pub fn get_default_fraction_digits(&self) -> i32 {
        self.0.default_fraction_digits
    }

    /// Returns the ISO 4217 numeric code of this currency. Equivalent to
    /// `getNumericCode()`.
    pub fn get_numeric_code(&self) -> i32 {
        self.0.numeric_code
    }

    /// Returns the 3-digit ISO 4217 numeric code of this currency as a
    /// string. Equivalent to `getNumericCodeAsString()`.
    pub fn get_numeric_code_as_string(&self) -> String {
        let numeric_code = self.0.numeric_code;
        if numeric_code < 100 {
            let mut sb = String::from("0");
            if numeric_code < 10 {
                sb.push('0');
            }
            sb.push_str(&numeric_code.to_string());
            return sb;
        }
        numeric_code.to_string()
    }

    /// `readResolve()`: the canonical instance of this code.
    ///
    /// # Errors
    ///
    /// As [`get_instance`](Self::get_instance).
    pub fn read_resolve(&self) -> Result<Currency> {
        Self::get_instance_impl(&self.0.currency_code.clone(), i32::MIN, 0)
    }
}

/// `CalendarDataUtility.findRegionOverride(Locale)`: the locale with the
/// region of a valid `rg` extension (`[A-Z]{2}ZZZZ`, case-insensitive).
fn find_region_override(l: &Locale) -> Result<Locale> {
    if let Some(rg) = l.get_unicode_locale_type("rg") {
        if rg.encode_utf16().count() == 6 {
            let rg = rg.to_ascii_uppercase();
            let b = rg.as_bytes();
            if b[0].is_ascii_uppercase() && b[1].is_ascii_uppercase() && &rg[2..] == "ZZZZ" {
                return l.with_region(&rg[..2]);
            }
        }
    }
    Ok(l.clone())
}

/// `getMainTableEntry(char, char)`.
fn get_main_table_entry(char1: u16, char2: u16) -> Result<i32> {
    let (a, z) = (u16::from(b'A'), u16::from(b'Z'));
    if char1 < a || char1 > z || char2 < a || char2 > z {
        return Err(LuceneError::illegal_argument(format!(
            "The country code: \"{}\" is not a valid ISO 3166 code",
            String::from_utf16_lossy(&[char1, char2])
        )));
    }
    Ok(MAIN_TABLE[usize::from(char1 - a) * A_TO_Z as usize + usize::from(char2 - a)])
}

fn units_of(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `mainTable`: 26 × 26 entries, one per two-letter country code.
const MAIN_TABLE: [i32; 676] = [
    0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x000C4043, 0x000F2C4D, 0x00000202, 0x0000007F, 0x00000202, 0x0000007F, 0x0000007F, 0x0000204B, 0x0000CC43,
    0x00085046, 0x000F3440, 0x0000007F, 0x00000200, 0x00008052, 0x00000203, 0x00000201, 0x00009043, 0x0000007F, 0x00085446, 0x00000201, 0x0000007F, 0x000EC04D,
    0x000F444C, 0x0000D043, 0x0000007F, 0x0000C853, 0x00000201, 0x00000204, 0x00000205, 0x0000C063, 0x0001B005, 0x00000204, 0x0000007F, 0x00000201, 0x0000F043,
    0x00018043, 0x00011041, 0x0000007F, 0x00000203, 0x000F684B, 0x0000B043, 0x0001004D, 0x0000007F, 0x00000206, 0x0001204F, 0x0000007F, 0x000E944D, 0x00015043,
    0x0001F043, 0x0000007F, 0x00000207, 0x000F4045, 0x0000007F, 0x00000208, 0x00000208, 0x000BD045, 0x00000204, 0x0000007F, 0x00000209, 0x0002600F, 0x00000208,
    0x00027058, 0x0002A84F, 0x0000007F, 0x0000007F, 0x0002F042, 0x000DEC43, 0x0000007F, 0x0003004F, 0x00021044, 0x0000020A, 0x00000207, 0x00000201, 0x00032C4A,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00041805, 0x0003404A, 0x0000007F, 0x00000202,
    0x0000007F, 0x0003584F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00003043,
    0x0000007F, 0x0000007F, 0x00000203, 0x0000007F, 0x00000201, 0x0000007F, 0x000CC84F, 0x0000020B, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0003A04D, 0x00000201, 0x00039841, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x0003C843, 0x0003B84F, 0x0000007F, 0x00000203,
    0x0000007F, 0x0000020C, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x00000208, 0x000CE84F, 0x0000007F, 0x00000202, 0x000F544B, 0x00000201, 0x0000020D, 0x000EA052, 0x0004904F, 0x0000007F, 0x0000007F, 0x0000020C, 0x00043843,
    0x00051005, 0x0000007F, 0x00000201, 0x00000208, 0x00000201, 0x0000020D, 0x00050050, 0x00000203, 0x0000007F, 0x00000204, 0x0000007F, 0x00052043, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00056043, 0x0000007F, 0x00000207,
    0x0005504B, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F, 0x00053046, 0x00057045, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0005A051, 0x00000201, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0005E052, 0x0000020D,
    0x00059051, 0x00000203, 0x0000007F, 0x0005C063, 0x0005B051, 0x0005800A, 0x00000201, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000020D, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00061043,
    0x0000007F, 0x00064063, 0x00062018, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00065052, 0x0000007F, 0x00068452, 0x0001D051, 0x00000207, 0x0000007F, 0x0000007F, 0x0000007F, 0x0002B805,
    0x00000202, 0x0000007F, 0x00066056, 0x0000007F, 0x00066816, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00067863, 0x0000007F, 0x00022043, 0x00063853,
    0x0006884A, 0x0006984F, 0x00000202, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000020E, 0x0000007F, 0x00024051, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0006B843, 0x0006A84B, 0x00000201, 0x00000201, 0x00000201, 0x0000007F, 0x0000007F, 0x0006C863, 0x0000007F,
    0x0007E043, 0x0000007F, 0x00000201, 0x0007C84B, 0x00000201, 0x00000201, 0x000F2440, 0x00000203, 0x0000007F, 0x0000007F, 0x000C9C43, 0x00000204, 0x0001A04A,
    0x0007C053, 0x0006F84F, 0x00000203, 0x00000201, 0x000E8454, 0x00000202, 0x00000201, 0x00078051, 0x00073851, 0x0007184A, 0x0007904D, 0x00072851, 0x000EBC4D,
    0x00081043, 0x0000007F, 0x0000020F, 0x0000007F, 0x00000204, 0x00000207, 0x0008D84D, 0x0000007F, 0x0008B84E, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F,
    0x0000007F, 0x0009084A, 0x00083051, 0x0000007F, 0x00000207, 0x0000007F, 0x0000007F, 0x00000209, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0008A843,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00080071,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x00093841, 0x0000007F, 0x0000007F, 0x0000007F, 0x0009704D, 0x0000020F, 0x0009584A, 0x0009804F, 0x0000007F, 0x0000007F, 0x00092851, 0x000F644D, 0x00000201,
    0x00000209, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000203, 0x00000210, 0x00000201, 0x0000007F, 0x0000007F, 0x00000203, 0x0000007F, 0x00096006, 0x0000007F,
    0x0009E851, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x000EC84D, 0x0000007F, 0x0000007F, 0x0000007F, 0x000EB443, 0x0000007F, 0x000A0C41, 0x0000007F, 0x000A1805, 0x0000007F, 0x0000007F, 0x0000007F,
    0x000AA851, 0x00016843, 0x000AC851, 0x000EA846, 0x000BC04A, 0x0000007F, 0x000AF843, 0x000A384F, 0x00000201, 0x00000206, 0x00000201, 0x000E7444, 0x00000201,
    0x00000204, 0x000B0852, 0x0000007F, 0x0000007F, 0x000F2043, 0x000B604F, 0x000E884D, 0x0000007F, 0x00037842, 0x0000007F, 0x0000020A, 0x000BE04F, 0x000BB04B,
    0x0000007F, 0x0000007F, 0x00000203, 0x00000208, 0x0000007F, 0x00000201, 0x00000204, 0x000BF041, 0x0000007F, 0x000F3052, 0x00000209, 0x00000203, 0x000E9853,
    0x000C5063, 0x000C204F, 0x0000007F, 0x0000007F, 0x000ED458, 0x0000007F, 0x000C3043, 0x0000007F, 0x00000207, 0x000E1443, 0x0000007F, 0x0000007F, 0x000D0852,
    0x000F5047, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000C8017, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000203,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000D2043, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000D6854, 0x000D7052,
    0x00000201, 0x0000007F, 0x00000202, 0x0000007F, 0x000E8052, 0x0000007F, 0x00000203, 0x0000007F, 0x00000203, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x000B0003, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00089015, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000020F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000DC853, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000DD851, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x00000201, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F,
    0x000B1851, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000F1C56,
    0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x0000007F, 0x000E7046, 0x0000007F, 0x0000007F, 0x0000007F,
];

/// `specialCasesList`, in `currency.data` order: (cutOverTime, oldCurrency, newCurrency, oldCurrencyFraction, newCurrencyFraction, oldCurrencyNumericCode, newCurrencyNumericCode).
const SPECIAL_CASES: [SpecialCaseEntry; 16] = [
    SpecialCaseEntry::new(i64::MAX, "EUR", "", 2, 0, 978, 0),
    SpecialCaseEntry::new(i64::MAX, "XCD", "", 2, 0, 951, 0),
    SpecialCaseEntry::new(i64::MAX, "USD", "", 2, 0, 840, 0),
    SpecialCaseEntry::new(i64::MAX, "XOF", "", 0, 0, 952, 0),
    SpecialCaseEntry::new(1767218400000, "BGN", "EUR", 2, 2, 975, 978),
    SpecialCaseEntry::new(i64::MAX, "NOK", "", 2, 0, 578, 0),
    SpecialCaseEntry::new(i64::MAX, "AUD", "", 2, 0, 36, 0),
    SpecialCaseEntry::new(i64::MAX, "XAF", "", 0, 0, 950, 0),
    SpecialCaseEntry::new(i64::MAX, "NZD", "", 2, 0, 554, 0),
    SpecialCaseEntry::new(i64::MAX, "XCG", "", 2, 0, 532, 0),
    SpecialCaseEntry::new(i64::MAX, "MAD", "", 2, 0, 504, 0),
    SpecialCaseEntry::new(i64::MAX, "DKK", "", 2, 0, 208, 0),
    SpecialCaseEntry::new(i64::MAX, "GBP", "", 2, 0, 826, 0),
    SpecialCaseEntry::new(i64::MAX, "CHF", "", 2, 0, 756, 0),
    SpecialCaseEntry::new(i64::MAX, "XPF", "", 0, 0, 953, 0),
    SpecialCaseEntry::new(i64::MAX, "ILS", "", 2, 0, 376, 0),
];

/// `otherCurrenciesList`, in `currency.data` order: (currencyCode, fraction, numericCode).
const OTHER_CURRENCIES: [OtherCurrencyEntry; 82] = [
    OtherCurrencyEntry::new("ADP", 0, 20),
    OtherCurrencyEntry::new("AFA", 2, 4),
    OtherCurrencyEntry::new("ATS", 2, 40),
    OtherCurrencyEntry::new("AYM", 2, 945),
    OtherCurrencyEntry::new("AZM", 2, 31),
    OtherCurrencyEntry::new("BEF", 0, 56),
    OtherCurrencyEntry::new("BGL", 2, 100),
    OtherCurrencyEntry::new("BGN", 2, 975),
    OtherCurrencyEntry::new("BOV", 2, 984),
    OtherCurrencyEntry::new("BYB", 0, 112),
    OtherCurrencyEntry::new("BYR", 0, 974),
    OtherCurrencyEntry::new("CHE", 2, 947),
    OtherCurrencyEntry::new("CHW", 2, 948),
    OtherCurrencyEntry::new("CLF", 4, 990),
    OtherCurrencyEntry::new("COU", 2, 970),
    OtherCurrencyEntry::new("CUC", 2, 931),
    OtherCurrencyEntry::new("CYP", 2, 196),
    OtherCurrencyEntry::new("DEM", 2, 276),
    OtherCurrencyEntry::new("EEK", 2, 233),
    OtherCurrencyEntry::new("ESP", 0, 724),
    OtherCurrencyEntry::new("FIM", 2, 246),
    OtherCurrencyEntry::new("FRF", 2, 250),
    OtherCurrencyEntry::new("GHC", 2, 288),
    OtherCurrencyEntry::new("GRD", 0, 300),
    OtherCurrencyEntry::new("GWP", 2, 624),
    OtherCurrencyEntry::new("HRK", 2, 191),
    OtherCurrencyEntry::new("IEP", 2, 372),
    OtherCurrencyEntry::new("ITL", 0, 380),
    OtherCurrencyEntry::new("LTL", 2, 440),
    OtherCurrencyEntry::new("LUF", 0, 442),
    OtherCurrencyEntry::new("LVL", 2, 428),
    OtherCurrencyEntry::new("MGF", 0, 450),
    OtherCurrencyEntry::new("MRO", 2, 478),
    OtherCurrencyEntry::new("MTL", 2, 470),
    OtherCurrencyEntry::new("MXV", 2, 979),
    OtherCurrencyEntry::new("MZM", 2, 508),
    OtherCurrencyEntry::new("NLG", 2, 528),
    OtherCurrencyEntry::new("PTE", 0, 620),
    OtherCurrencyEntry::new("ROL", 0, 642),
    OtherCurrencyEntry::new("RUR", 2, 810),
    OtherCurrencyEntry::new("SDD", 2, 736),
    OtherCurrencyEntry::new("SIT", 2, 705),
    OtherCurrencyEntry::new("SKK", 2, 703),
    OtherCurrencyEntry::new("SLL", 2, 694),
    OtherCurrencyEntry::new("SRG", 2, 740),
    OtherCurrencyEntry::new("STD", 2, 678),
    OtherCurrencyEntry::new("TMM", 2, 795),
    OtherCurrencyEntry::new("TPE", 0, 626),
    OtherCurrencyEntry::new("TRL", 0, 792),
    OtherCurrencyEntry::new("USN", 2, 997),
    OtherCurrencyEntry::new("USS", 2, 998),
    OtherCurrencyEntry::new("UYI", 0, 940),
    OtherCurrencyEntry::new("VEB", 2, 862),
    OtherCurrencyEntry::new("VED", 2, 926),
    OtherCurrencyEntry::new("VEF", 2, 937),
    OtherCurrencyEntry::new("XAD", 2, 396),
    OtherCurrencyEntry::new("XAF", 0, 950),
    OtherCurrencyEntry::new("XAG", -1, 961),
    OtherCurrencyEntry::new("XAU", -1, 959),
    OtherCurrencyEntry::new("XBA", -1, 955),
    OtherCurrencyEntry::new("XBB", -1, 956),
    OtherCurrencyEntry::new("XBC", -1, 957),
    OtherCurrencyEntry::new("XBD", -1, 958),
    OtherCurrencyEntry::new("XCD", 2, 951),
    OtherCurrencyEntry::new("XCG", 2, 532),
    OtherCurrencyEntry::new("XDR", -1, 960),
    OtherCurrencyEntry::new("XFO", -1, 0),
    OtherCurrencyEntry::new("XFU", -1, 0),
    OtherCurrencyEntry::new("XOF", 0, 952),
    OtherCurrencyEntry::new("XPD", -1, 964),
    OtherCurrencyEntry::new("XPF", 0, 953),
    OtherCurrencyEntry::new("XPT", -1, 962),
    OtherCurrencyEntry::new("XSU", -1, 994),
    OtherCurrencyEntry::new("XTS", -1, 963),
    OtherCurrencyEntry::new("XUA", -1, 965),
    OtherCurrencyEntry::new("XXX", -1, 999),
    OtherCurrencyEntry::new("YUM", 2, 891),
    OtherCurrencyEntry::new("ZMK", 2, 894),
    OtherCurrencyEntry::new("ZWD", 2, 716),
    OtherCurrencyEntry::new("ZWL", 2, 932),
    OtherCurrencyEntry::new("ZWN", 2, 942),
    OtherCurrencyEntry::new("ZWR", 2, 935),
];
