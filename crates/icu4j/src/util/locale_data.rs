// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.LocaleData`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{self, ICUResourceBundle};
use crate::text::unicode_set::UnicodeSet;
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};
use crate::util::version_info::VersionInfo;

/// `MEASUREMENT_SYSTEM`.
const MEASUREMENT_SYSTEM: &str = "MeasurementSystem";
/// `PAPER_SIZE`.
const PAPER_SIZE: &str = "PaperSize";
/// `LOCALE_DISPLAY_PATTERN`.
const LOCALE_DISPLAY_PATTERN: &str = "localeDisplayPattern";
/// `PATTERN`.
const PATTERN: &str = "pattern";
/// `SEPARATOR`.
const SEPARATOR: &str = "separator";

/// `DELIMITER_TYPES`.
const DELIMITER_TYPES: [&str; 4] = [
    "quotationStart",
    "quotationEnd",
    "alternateQuotationStart",
    "alternateQuotationEnd",
];

/// A class for accessing miscellaneous data in the locale bundles.
///
/// Equivalent to `com.ibm.icu.util.LocaleData`.
#[derive(Debug)]
pub struct LocaleData {
    no_substitute: bool,
    bundle: Arc<ICUResourceBundle>,
    lang_bundle: Option<Arc<ICUResourceBundle>>,
    lang_bundle_exception: Option<LuceneError>,
}

/// The measurement systems.
///
/// Equivalent to `LocaleData.MeasurementSystem` with its instances `SI`,
/// `US` and `UK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasurementSystem {
    /// `SI`: the metric system.
    Si,
    /// `US`: the US customary system.
    Us,
    /// `UK`: the mix of metric and imperial units used in Great Britain.
    Uk,
}

/// A paper size, in millimeters.
///
/// Equivalent to `LocaleData.PaperSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperSize {
    height: i32,
    width: i32,
}

impl PaperSize {
    /// `getHeight()`.
    pub fn get_height(&self) -> i32 {
        self.height
    }

    /// `getWidth()`.
    pub fn get_width(&self) -> i32 {
        self.width
    }
}

impl LocaleData {
    /// `ES_STANDARD`.
    pub const ES_STANDARD: i32 = 0;
    /// `ES_AUXILIARY`.
    pub const ES_AUXILIARY: i32 = 1;
    /// `ES_INDEX`.
    pub const ES_INDEX: i32 = 2;
    /// `ES_CURRENCY` (deprecated).
    pub const ES_CURRENCY: i32 = 3;
    /// `ES_PUNCTUATION`.
    pub const ES_PUNCTUATION: i32 = 4;
    /// `ES_COUNT` (deprecated).
    pub const ES_COUNT: i32 = 5;
    /// `QUOTATION_START`.
    pub const QUOTATION_START: i32 = 0;
    /// `QUOTATION_END`.
    pub const QUOTATION_END: i32 = 1;
    /// `ALT_QUOTATION_START`.
    pub const ALT_QUOTATION_START: i32 = 2;
    /// `ALT_QUOTATION_END`.
    pub const ALT_QUOTATION_END: i32 = 3;
    /// `DELIMITER_COUNT` (deprecated).
    pub const DELIMITER_COUNT: i32 = 4;

    /// `getInstance(ULocale locale)`: the locale data of a locale.
    ///
    /// # Errors
    ///
    /// The errors of opening the locale bundle (a missing `lang` bundle is
    /// kept and thrown by the display-pattern getters, ICU-22149).
    pub fn get_instance(locale: &ULocale) -> Result<LocaleData> {
        let bundle =
            UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(locale))?
                .as_icu()?
                .clone();
        let (lang_bundle, lang_bundle_exception) = match UResourceBundle::get_bundle_instance_for(
            Some(ICUData::ICU_LANG_BASE_NAME),
            Some(locale),
        ) {
            Ok(b) => (Some(b.as_icu()?.clone()), None),
            // Expected error case: ICU-22149
            Err(mre) if is_missing_resource(&mre) => (None, Some(mre)),
            Err(e) => return Err(e),
        };
        Ok(LocaleData {
            no_substitute: false,
            bundle,
            lang_bundle,
            lang_bundle_exception,
        })
    }

    /// `getInstance()`: for the default format locale.
    ///
    /// # Errors
    ///
    /// As [`get_instance`](Self::get_instance).
    pub fn get_instance_default() -> Result<LocaleData> {
        Self::get_instance(&ULocale::get_default_category(Category::Format))
    }

    /// `setNoSubstitute(boolean setting)`.
    pub fn set_no_substitute(&mut self, setting: bool) {
        self.no_substitute = setting;
    }

    /// `getNoSubstitute()`.
    pub fn get_no_substitute(&self) -> bool {
        self.no_substitute
    }

    /// `getExemplarSet(ULocale locale, int options)`: the standard exemplar
    /// set of the locale.
    ///
    /// # Errors
    ///
    /// The errors of opening the locale bundle.
    pub fn get_exemplar_set_for(locale: &ULocale, options: i32) -> Result<Option<UnicodeSet>> {
        LocaleData::get_instance(locale)?.get_exemplar_set(options, Self::ES_STANDARD)
    }

    /// `getExemplarSet(ULocale locale, int options, int extype)`.
    ///
    /// # Errors
    ///
    /// As [`get_exemplar_set`](Self::get_exemplar_set); the errors of
    /// opening the locale bundle.
    pub fn get_exemplar_set_for_type(
        locale: &ULocale,
        options: i32,
        extype: i32,
    ) -> Result<Option<UnicodeSet>> {
        LocaleData::get_instance(locale)?.get_exemplar_set(options, extype)
    }

    /// `getExemplarSet(int options, int extype)`: the exemplar set of a
    /// type (`UnicodeSet` options such as `CASE` are applied); `None` when
    /// `noSubstitute` and the data comes from root, the empty frozen set
    /// when the data is missing or malformed.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (wrapping the
    /// `ArrayIndexOutOfBoundsException`) for a type outside `0..5`.
    pub fn get_exemplar_set(&self, options: i32, extype: i32) -> Result<Option<UnicodeSet>> {
        let exemplar_set_types: [&str; 5] = [
            "ExemplarCharacters",
            "AuxExemplarCharacters",
            "ExemplarCharactersIndex",
            "ExemplarCharactersCurrency",
            "ExemplarCharactersPunctuation",
        ];

        if extype == Self::ES_CURRENCY {
            // currency symbol exemplar is no longer available
            return Ok(if self.no_substitute {
                None
            } else {
                Some(crate::text::unicode_set::EMPTY.clone())
            });
        }

        let Some(a_key) = exemplar_set_types
            .get(extype as usize)
            .filter(|_| extype >= 0)
        else {
            return Err(LuceneError::illegal_argument(format!(
                "java.lang.ArrayIndexOutOfBoundsException: Index {extype} out of bounds for length 5"
            )));
        };
        let attempt = (|| -> Result<Option<UnicodeSet>> {
            let string_bundle = UResourceBundle::Icu(self.bundle.clone())
                .get(a_key)?
                .as_icu()?
                .clone();

            if self.no_substitute && !self.bundle.is_root() && string_bundle.is_root() {
                return Ok(None);
            }

            let unicode_set_pattern = string_bundle.get_string()?;
            Ok(Some(UnicodeSet::from_pattern_options(
                &unicode_set_pattern,
                crate::text::unicode_set::IGNORE_SPACE | options,
            )?))
        })();
        match attempt {
            Ok(r) => Ok(r),
            Err(_ex) => Ok(if self.no_substitute {
                None
            } else {
                Some(crate::text::unicode_set::EMPTY.clone())
            }),
        }
    }

    /// `getDelimiter(int type)`: a quotation mark, `None` when substitution
    /// is off and it comes from root.
    ///
    /// # Errors
    ///
    /// `MissingResourceException`; `UResourceTypeMismatchException`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for a type outside `0..4`.
    pub fn get_delimiter(&self, ty: i32) -> Result<Option<Vec<u16>>> {
        let delimiters_bundle = UResourceBundle::Icu(self.bundle.clone())
            .get("delimiters")?
            .as_icu()?
            .clone();
        // Only some of the quotation marks may be here. So we make sure that we do a multilevel fallback.
        let ty_name = match DELIMITER_TYPES.get(ty as usize).filter(|_| ty >= 0) {
            Some(t) => *t,
            None => crate::jdk::array_index_out_of_bounds(ty),
        };
        let string_bundle = delimiters_bundle.get_with_fallback(ty_name)?;

        if self.no_substitute && !self.bundle.is_root() && string_bundle.is_root() {
            return Ok(None);
        }

        Ok(Some(string_bundle.get_string()?))
    }

    fn measurement_type_bundle_for_locale(
        locale: &ULocale,
        measurement_type: &str,
    ) -> Result<Option<UResourceBundle>> {
        // Much of this is taken from getCalendarType in impl/CalendarUtil.java
        let region = ULocale::get_region_for_supplemental_data(locale, true)?;
        let attempt = (|| -> Result<UResourceBundle> {
            let rb = UResourceBundle::get_bundle_instance_loader(
                Some(ICUData::ICU_BASE_NAME),
                "supplementalData",
                icu_resource_bundle::icu_data_class_loader(),
            )?;
            let measurement_data = rb.get("measurementData")?;
            match measurement_data
                .get(&region)
                .and_then(|d| d.get(measurement_type))
            {
                Ok(b) => Ok(b),
                Err(mre) if is_missing_resource(&mre) => {
                    // use "001" as fallback
                    measurement_data.get("001")?.get(measurement_type)
                }
                Err(e) => Err(e),
            }
        })();
        match attempt {
            Ok(b) => Ok(Some(b)),
            // fall through
            Err(mre) if is_missing_resource(&mre) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `getMeasurementSystem(ULocale locale)`: `None` for an unknown value.
    ///
    /// # Errors
    ///
    /// `NullPointerException` when the data is missing;
    /// `UResourceTypeMismatchException`.
    pub fn get_measurement_system(locale: &ULocale) -> Result<Option<MeasurementSystem>> {
        let sys_bundle = Self::measurement_type_bundle_for_locale(locale, MEASUREMENT_SYSTEM)?
            .ok_or_else(LuceneError::null_pointer)?;

        Ok(match sys_bundle.get_int()? {
            0 => Some(MeasurementSystem::Si),
            1 => Some(MeasurementSystem::Us),
            2 => Some(MeasurementSystem::Uk),
            // return null if the object is null or is not an instance
            // of integer indicating an error
            _ => None,
        })
    }

    /// `getPaperSize(ULocale locale)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` when the data is missing;
    /// `UResourceTypeMismatchException`.
    ///
    /// # Panics
    ///
    /// `ArrayIndexOutOfBoundsException` for a vector shorter than 2.
    pub fn get_paper_size(locale: &ULocale) -> Result<PaperSize> {
        let obj = Self::measurement_type_bundle_for_locale(locale, PAPER_SIZE)?
            .ok_or_else(LuceneError::null_pointer)?;
        let size = obj.get_int_vector()?;
        Ok(PaperSize {
            height: size[0],
            width: size[1],
        })
    }

    fn lang_bundle(&self) -> Result<&Arc<ICUResourceBundle>> {
        match &self.lang_bundle {
            Some(b) => Ok(b),
            None => Err(self
                .lang_bundle_exception
                .as_ref()
                .map(LuceneError::duplicate)
                .unwrap_or_else(LuceneError::null_pointer)),
        }
    }

    /// `getLocaleDisplayPattern()`.
    ///
    /// # Errors
    ///
    /// The `MissingResourceException` of a missing `lang` bundle, or of the
    /// pattern.
    pub fn get_locale_display_pattern(&self) -> Result<Vec<u16>> {
        let lang_bundle = self.lang_bundle()?;
        let loc_disp_bundle = UResourceBundle::Icu(lang_bundle.clone())
            .get(LOCALE_DISPLAY_PATTERN)?
            .as_icu()?
            .clone();
        loc_disp_bundle.get_string_with_fallback(PATTERN)
    }

    /// `getLocaleSeparator()`: the text between `{0}` and `{1}` of the
    /// separator pattern, or the whole pattern.
    ///
    /// # Errors
    ///
    /// As [`get_locale_display_pattern`](Self::get_locale_display_pattern).
    pub fn get_locale_separator(&self) -> Result<Vec<u16>> {
        let lang_bundle = self.lang_bundle()?;
        let sub0 = "{0}";
        let sub1 = "{1}";
        let loc_disp_bundle = UResourceBundle::Icu(lang_bundle.clone())
            .get(LOCALE_DISPLAY_PATTERN)?
            .as_icu()?
            .clone();
        let locale_separator = loc_disp_bundle.get_string_with_fallback(SEPARATOR)?;
        let sub0u: Vec<u16> = sub0.encode_utf16().collect();
        let sub1u: Vec<u16> = sub1.encode_utf16().collect();
        let index0 = crate::jdk::index_of(&locale_separator, &sub0u, 0);
        let index1 = crate::jdk::index_of(&locale_separator, &sub1u, 0);
        if index0 >= 0 && index1 >= 0 && index0 <= index1 {
            return Ok(locale_separator[(index0 as usize + sub0u.len())..index1 as usize].to_vec());
        }
        Ok(locale_separator)
    }

    /// `getCLDRVersion()`: the CLDR version of the data, read once.
    ///
    /// # Errors
    ///
    /// `MissingResourceException`; `IllegalArgumentException` for a
    /// malformed version.
    pub fn get_cldr_version() -> Result<VersionInfo> {
        static G_CLDR_VERSION: Mutex<Option<VersionInfo>> = Mutex::new(None);
        let mut g = G_CLDR_VERSION.lock().unwrap_or_else(|e| e.into_inner());
        // fetching this data should be idempotent.
        if g.is_none() {
            // from ZoneMeta.java
            let supplemental_data_bundle = UResourceBundle::get_bundle_instance_loader(
                Some(ICUData::ICU_BASE_NAME),
                "supplementalData",
                icu_resource_bundle::icu_data_class_loader(),
            )?;
            let cldr_version_bundle = supplemental_data_bundle.get("cldrVersion")?;
            *g = Some(VersionInfo::get_instance_str(
                &cldr_version_bundle.get_string()?,
            )?);
        }
        Ok(g.expect("set above"))
    }
}
