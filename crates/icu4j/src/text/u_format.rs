// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.UFormat`.

use rucene::error::{LuceneError, Result};

use crate::jdk::text::{Attribute, Format};
use crate::util::u_locale::{Type, ULocale, ACTUAL_LOCALE};

/// The Java class name of `UFormat.SpanField`.
pub const SPAN_FIELD_CLASS: &str = "com.ibm.icu.text.UFormat$SpanField";

/// `instanceof UFormat.SpanField`: the abstract `Format.Field` subclass
/// whose instances identify spans (whose field values carry an index).
///
/// Equivalent to the abstract class `UFormat.SpanField`; a subclass is
/// declared with [`crate::java_format_field_class!`] listing
/// [`SPAN_FIELD_CLASS`] as its superclass.
pub fn is_span_field(field: &dyn Attribute) -> bool {
    field.is_instance_of(SPAN_FIELD_CLASS)
}

/// The locale state of a `UFormat` (`validLocale`, `actualLocale`; Java's
/// `null` is `None`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UFormatBase {
    valid_locale: Option<ULocale>,
    actual_locale: Option<ULocale>,
}

impl UFormatBase {
    /// `new UFormat()`: no locales.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getLocale(ULocale.Type type)`: the actual locale for
    /// `ACTUAL_LOCALE`, the valid locale otherwise.
    pub fn get_locale(&self, locale_type: Type) -> Option<ULocale> {
        if locale_type == ACTUAL_LOCALE {
            self.actual_locale.clone()
        } else {
            self.valid_locale.clone()
        }
    }

    /// `setLocale(ULocale valid, ULocale actual)` (package-private).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (no message) when exactly one of the two
    /// is `None`.
    pub fn set_locale(&mut self, valid: Option<ULocale>, actual: Option<ULocale>) -> Result<()> {
        // Change the following to an assertion later
        if valid.is_none() != actual.is_none() {
            return Err(LuceneError::unchecked_with_message("java.lang.IllegalArgumentException", None));
        }
        // Another check we could do is that the actual locale is at
        // the same level or less specific than the valid locale.
        self.valid_locale = valid;
        self.actual_locale = actual;
        Ok(())
    }
}

/// An abstract class that extends `java.text.Format` to provide additional
/// ICU protocol, specifically, the `getLocale()` API.
///
/// Equivalent to the abstract class `com.ibm.icu.text.UFormat`: its
/// private fields are the [`UFormatBase`] an implementation holds, its
/// final `getLocale` the provided [`get_locale`](Self::get_locale).
pub trait UFormat: Format {
    /// The `UFormat` fields of this object.
    fn u_format_base(&self) -> &UFormatBase;

    /// The `UFormat` fields of this object, mutable (for `setLocale`).
    fn u_format_base_mut(&mut self) -> &mut UFormatBase;

    /// `getLocale(ULocale.Type type)` (final).
    fn get_locale(&self, locale_type: Type) -> Option<ULocale> {
        self.u_format_base().get_locale(locale_type)
    }

    /// `setLocale(ULocale valid, ULocale actual)` (final, package-private).
    ///
    /// # Errors
    ///
    /// As [`UFormatBase::set_locale`].
    fn set_locale(&mut self, valid: Option<ULocale>, actual: Option<ULocale>) -> Result<()> {
        self.u_format_base_mut().set_locale(valid, actual)
    }
}
