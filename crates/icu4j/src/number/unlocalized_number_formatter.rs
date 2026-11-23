// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.UnlocalizedNumberFormatter`.

use std::sync::Arc;

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::number::localized_number_formatter::LocalizedNumberFormatter;
use crate::number::number_formatter::DEFAULT_THRESHOLD;
use crate::number::number_formatter_settings::{
    NumberFormatterSettings, SettingValue, SettingsNode, KEY_LOCALE, KEY_THRESHOLD,
};
use crate::util::u_locale::ULocale;

/// A NumberFormatter that does not yet have a locale. In order to format
/// numbers, a locale must be specified.
///
/// Instances of this class are immutable and thread-safe.
///
/// Equivalent to `com.ibm.icu.number.UnlocalizedNumberFormatter`.
#[derive(Debug, Clone)]
pub struct UnlocalizedNumberFormatter(Arc<SettingsNode>);

impl UnlocalizedNumberFormatter {
    /// Base constructor; called during startup only. Sets the threshold to
    /// the default value of 3. Equivalent to `UnlocalizedNumberFormatter()`.
    pub(crate) fn base() -> Self {
        UnlocalizedNumberFormatter(SettingsNode::new(None, KEY_THRESHOLD, Some(SettingValue::Threshold(DEFAULT_THRESHOLD))))
    }

    /// `UnlocalizedNumberFormatter(NumberFormatterSettings<?>, int, Object)`.
    pub(crate) fn with_parent(parent: &Arc<SettingsNode>, key: i32, value: Option<SettingValue>) -> Self {
        UnlocalizedNumberFormatter(SettingsNode::new(Some(parent.clone()), key, value))
    }

    /// Associate the given locale with the number formatter. Equivalent to
    /// `locale(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn locale_java(&self, locale: &Locale) -> Result<LocalizedNumberFormatter> {
        let ulocale = ULocale::for_locale(Some(locale))?;
        Ok(LocalizedNumberFormatter::with_parent(&self.0, KEY_LOCALE, ulocale.map(SettingValue::Locale)))
    }

    /// `resolve().loc = loc`, from `NumberRangeFormatterSettings.resolve`.
    pub(crate) fn set_resolved_loc(&self, loc: Option<ULocale>) {
        self.0.set_resolved_loc(loc);
    }

    /// Equivalent to `locale(ULocale)`.
    pub fn locale(&self, locale: &ULocale) -> LocalizedNumberFormatter {
        LocalizedNumberFormatter::with_parent(&self.0, KEY_LOCALE, Some(SettingValue::Locale(locale.clone())))
    }
}

impl NumberFormatterSettings for UnlocalizedNumberFormatter {
    fn node(&self) -> &Arc<SettingsNode> {
        &self.0
    }

    fn create(&self, key: i32, value: Option<SettingValue>) -> Self {
        UnlocalizedNumberFormatter::with_parent(&self.0, key, value)
    }
}
