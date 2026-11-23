// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RbnfLenientScannerProvider`.

use std::sync::Arc;

use rucene::error::Result;

use crate::text::rbnf_lenient_scanner::RbnfLenientScanner;
use crate::util::u_locale::ULocale;

/// A provider for an `RbnfLenientScanner`.
///
/// Equivalent to the deprecated interface
/// `com.ibm.icu.text.RbnfLenientScannerProvider`; Java's `null` `extras`
/// is `None`.
pub trait RbnfLenientScannerProvider: std::fmt::Debug + Send + Sync {
    /// `get(ULocale locale, String extras)`: a scanner for `locale`, with
    /// `extras` rules tailoring its collator.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn get(&self, locale: &ULocale, extras: Option<&[u16]>) -> Result<Arc<dyn RbnfLenientScanner>>;
}
