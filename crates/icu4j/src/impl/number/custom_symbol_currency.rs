// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.CustomSymbolCurrency`.

use rucene::error::Result;

use crate::jdk::units;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::util::currency::{self, Currency};
use crate::util::u_locale::ULocale;

/// A `Currency` with custom symbols: the currency symbol and the
/// international currency symbol of a `DecimalFormatSymbols`.
///
/// Equivalent to `com.ibm.icu.impl.number.CustomSymbolCurrency` (a
/// `Currency` subclass). Its instances are [`Currency`] values whose unit
/// carries the two symbols; its overrides (`getName` for `SYMBOL_NAME`,
/// `getCurrencyCode`, `hashCode`, `equals`) are applied by [`Currency`] and
/// `MeasureUnit` on those symbols (Java's virtual dispatch), and its
/// plural `getName` override only delegates to the superclass.
pub struct CustomSymbolCurrency;

impl CustomSymbolCurrency {
    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = 2497493016770137670;

    /// Equivalent to `resolve(Currency, ULocale, DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of `Currency.getInstance` and of the display-name data.
    pub fn resolve(currency: Option<Currency>, _locale: &ULocale, symbols: &DecimalFormatSymbols) -> Result<Currency> {
        let currency = match currency.or_else(|| symbols.get_currency()) {
            None => return Currency::get_instance(Some("XXX")),
            Some(c) => c,
        };
        match symbols.get_currency() {
            Some(sc) if currency.java_equals(&sc) => {}
            _ => return Ok(currency),
        }
        let currency1_sym = symbols.get_currency_symbol();
        let currency2_sym = symbols.get_international_currency_symbol();
        let currency1 = currency.get_name(&symbols.get_u_locale(), currency::SYMBOL_NAME, None)?;
        let currency2 = currency.get_currency_code();
        if currency1.map(|s| units(&s)) != Some(currency1_sym.clone()) || units(&currency2) != currency2_sym {
            return Ok(CustomSymbolCurrency::new(&currency2, currency1_sym, currency2_sym));
        }
        Ok(currency)
    }

    /// Equivalent to `CustomSymbolCurrency(String, String, String)`.
    pub fn new(iso_code: &str, currency1_sym: Vec<u16>, currency2_sym: Vec<u16>) -> Currency {
        Currency::new_custom(iso_code, currency1_sym, currency2_sym)
    }
}
