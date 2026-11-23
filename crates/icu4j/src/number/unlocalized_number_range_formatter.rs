// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.UnlocalizedNumberRangeFormatter`.

use std::sync::Arc;

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::number::localized_number_range_formatter::LocalizedNumberRangeFormatter;
use crate::number::number_range_formatter_settings::{
    NumberRangeFormatterSettings, RangeSettingValue, RangeSettingsNode, KEY_LOCALE, KEY_MACROS,
};
use crate::util::u_locale::ULocale;

/// A NumberRangeFormatter that does not yet have a locale. In order to
/// format, a locale must be specified.
///
/// Instances of this class are immutable and thread-safe.
///
/// Equivalent to `com.ibm.icu.number.UnlocalizedNumberRangeFormatter`.
#[derive(Debug, Clone)]
pub struct UnlocalizedNumberRangeFormatter(Arc<RangeSettingsNode>);

impl UnlocalizedNumberRangeFormatter {
    /// Base constructor; called during startup only. Equivalent to the
    /// package-private `UnlocalizedNumberRangeFormatter()`.
    pub(crate) fn new() -> Self {
        UnlocalizedNumberRangeFormatter(RangeSettingsNode::new(None, KEY_MACROS, None))
    }

    /// Equivalent to the package-private
    /// `UnlocalizedNumberRangeFormatter(NumberRangeFormatterSettings<?>,
    /// int, Object)`.
    pub(crate) fn with_parent(parent: &Arc<RangeSettingsNode>, key: i32, value: Option<RangeSettingValue>) -> Self {
        UnlocalizedNumberRangeFormatter(RangeSettingsNode::new(Some(parent.clone()), key, value))
    }

    /// Associate the given locale with the number range formatter. The
    /// locale is used for picking the appropriate symbols, formats, and
    /// other data for number display. Equivalent to `locale(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn locale_java(&self, locale: &Locale) -> Result<LocalizedNumberRangeFormatter> {
        let ulocale = ULocale::for_locale(Some(locale))?;
        Ok(LocalizedNumberRangeFormatter::with_parent(&self.0, KEY_LOCALE, ulocale.map(RangeSettingValue::Locale)))
    }

    /// Equivalent to `locale(ULocale)`.
    pub fn locale(&self, locale: &ULocale) -> LocalizedNumberRangeFormatter {
        LocalizedNumberRangeFormatter::with_parent(&self.0, KEY_LOCALE, Some(RangeSettingValue::Locale(locale.clone())))
    }
}

impl NumberRangeFormatterSettings for UnlocalizedNumberRangeFormatter {
    fn node(&self) -> &Arc<RangeSettingsNode> {
        &self.0
    }

    fn create(&self, key: i32, value: Option<RangeSettingValue>) -> Self {
        UnlocalizedNumberRangeFormatter::with_parent(&self.0, key, value)
    }
}
