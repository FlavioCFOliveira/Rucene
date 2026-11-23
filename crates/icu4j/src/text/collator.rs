// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Collator`.

use std::any::Any;
use std::collections::{HashSet, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, OnceLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{to_rust_string, units};
use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::lang::u_script::UScript;
use crate::r#impl::coll::collation_root::CollationRoot;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{icu_data_class_loader, ICUResourceBundle};
use crate::r#impl::icu_service::Factory;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::text::collation_key::CollationKey;
use crate::text::collator_service_shim::CollatorServiceShim;
use crate::text::raw_collation_key::RawCollationKey;
use crate::text::rule_based_collator::RuleBasedCollator;
use crate::text::unicode_set::UnicodeSet;
use crate::util::u_locale::{Category, Type, ULocale};
use crate::util::u_resource_bundle::UResourceBundle;
use crate::util::version_info::VersionInfo;

/// Strongest collator strength value. Typically used to denote differences
/// between base characters.
pub const PRIMARY: i32 = 0;
/// Second level collator strength value. Accents in the characters are
/// considered secondary differences.
pub const SECONDARY: i32 = 1;
/// Third level collator strength value. Upper and lower case differences
/// in characters are distinguished at this strength level.
pub const TERTIARY: i32 = 2;
/// Fourth level collator strength value.
pub const QUATERNARY: i32 = 3;
/// Smallest Collator strength value. When all other strengths are equal,
/// the IDENTICAL strength is used as a tiebreaker.
pub const IDENTICAL: i32 = 15;
/// This is for backwards compatibility with Java APIs only.
pub const FULL_DECOMPOSITION: i32 = IDENTICAL;
/// Decomposition mode value. With NO_DECOMPOSITION set, Strings will not
/// be decomposed for collation.
pub const NO_DECOMPOSITION: i32 = 16;
/// Decomposition mode value. With CANONICAL_DECOMPOSITION set, characters
/// that are canonical variants according to the Unicode standard will be
/// decomposed for collation.
pub const CANONICAL_DECOMPOSITION: i32 = 17;

/// Reordering codes for non-script groups that can be reordered under
/// collation.
///
/// Equivalent to the interface `Collator.ReorderCodes`.
#[derive(Debug)]
pub struct ReorderCodes;

impl ReorderCodes {
    /// A special reordering code that is used to specify the default
    /// reordering codes for a locale.
    pub const DEFAULT: i32 = -1; // == UScript.INVALID_CODE
    /// A special reordering code that is used to specify no reordering codes.
    pub const NONE: i32 = UScript::UNKNOWN;
    /// A special reordering code that is used to specify all other codes
    /// used for reordering except for the codes listed as ReorderingCodes
    /// and those listed explicitly in a reordering.
    pub const OTHERS: i32 = UScript::UNKNOWN;
    /// Characters with the space property.
    pub const SPACE: i32 = 0x1000;
    /// The first entry in the enumeration of reordering groups.
    pub const FIRST: i32 = Self::SPACE;
    /// Characters with the punctuation property.
    pub const PUNCTUATION: i32 = 0x1001;
    /// Characters with the symbol property.
    pub const SYMBOL: i32 = 0x1002;
    /// Characters with the currency property.
    pub const CURRENCY: i32 = 0x1003;
    /// Characters with the digit property.
    pub const DIGIT: i32 = 0x1004;
    /// One more than the highest normal ReorderCodes value.
    #[deprecated]
    pub const LIMIT: i32 = 0x1005;
}

/// The registry key of `registerInstance`/`registerFactory` (Java's
/// `Object`, which is the service factory).
pub type RegistryKey = Arc<dyn Factory>;

fn frozen_error() -> LuceneError {
    LuceneError::unsupported_operation("Attempt to modify frozen Collator")
}

fn needs_subclass() -> LuceneError {
    LuceneError::unsupported_operation("Needs to be implemented by the subclass.")
}

/// Collator performs locale-sensitive string comparison.
///
/// Equivalent to the abstract class `com.ibm.icu.text.Collator`
/// (`Comparator<Object>`, `Freezable<Collator>`, `Cloneable`): the
/// abstract methods are required trait methods, the overridable ones
/// provided methods with Java's bodies; the static members are functions
/// of this module. Java's `Comparator<Object>.compare(Object, Object)`
/// casts to `CharSequence`, so it is [`do_compare`](Self::do_compare)
/// over UTF-16 text. Setters that Java declares `void` and that throw
/// `UnsupportedOperationException` on a frozen collator return `Result`.
pub trait Collator: Debug + Send + Sync + Any {
    /// The collator as `Any` (Java's runtime class, for `getClass()` and
    /// `instanceof`).
    fn as_any(&self) -> &dyn Any;

    /// `instanceof RuleBasedCollator`.
    fn as_rule_based(&self) -> Option<&RuleBasedCollator> {
        None
    }

    /// `(RuleBasedCollator) coll` for writing.
    fn as_rule_based_mut(&mut self) -> Option<&mut RuleBasedCollator> {
        None
    }

    /// Clones the collator.
    ///
    /// Equivalent to `clone()`.
    ///
    /// # Errors
    ///
    /// Java's `CloneNotSupportedException`.
    fn clone_box(&self) -> Result<Box<dyn Collator>>;

    /// Compares the equality of two Collator objects. Collator objects are
    /// equal if they have the same runtime class.
    ///
    /// Equivalent to `equals(Object)`.
    fn equals(&self, obj: &dyn Collator) -> bool {
        // Subclasses: Call this method and then add more specific checks.
        std::ptr::addr_eq(self as *const Self, obj as *const dyn Collator) || self.as_any().type_id() == obj.as_any().type_id()
    }

    /// Generates a hash code for this Collator object.
    ///
    /// Equivalent to `hashCode()`.
    fn hash_code(&self) -> i32 {
        // Dummy return to prevent compile warnings.
        0
    }

    /// Sets this Collator's strength attribute.
    ///
    /// Equivalent to `setStrength(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if the Collator is frozen.
    fn set_strength(&mut self, _new_strength: i32) -> Result<()> {
        if self.is_frozen() {
            return Err(frozen_error());
        }
        Ok(())
    }

    /// `setStrength2(int)` (deprecated): as `setStrength`, returning this.
    ///
    /// # Errors
    ///
    /// As [`set_strength`](Self::set_strength).
    fn set_strength2(&mut self, new_strength: i32) -> Result<&mut Self>
    where
        Self: Sized,
    {
        self.set_strength(new_strength)?;
        Ok(self)
    }

    /// Sets the decomposition mode of this Collator.
    ///
    /// Equivalent to `setDecomposition(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if the Collator is frozen.
    fn set_decomposition(&mut self, _decomposition: i32) -> Result<()> {
        if self.is_frozen() {
            return Err(frozen_error());
        }
        Ok(())
    }

    /// Sets the reordering codes for this collator.
    ///
    /// Equivalent to `setReorderCodes(int...)`; `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless the subclass implements it.
    fn set_reorder_codes(&mut self, _order: Option<&[i32]>) -> Result<()> {
        Err(needs_subclass())
    }

    /// Returns this Collator's strength attribute.
    ///
    /// Equivalent to `getStrength()`.
    fn get_strength(&self) -> i32 {
        TERTIARY
    }

    /// Returns the decomposition mode of this Collator.
    ///
    /// Equivalent to `getDecomposition()`.
    fn get_decomposition(&self) -> i32 {
        NO_DECOMPOSITION
    }

    /// Compares the equality of two text Strings using this Collator's
    /// rules, strength and decomposition mode.
    ///
    /// Equivalent to `equals(String, String)`.
    fn equals_strings(&self, source: &[u16], target: &[u16]) -> bool {
        self.compare(source, target) == 0
    }

    /// Returns a UnicodeSet that contains all the characters and sequences
    /// tailored in this collator.
    ///
    /// Equivalent to `getTailoredSet()`.
    ///
    /// # Errors
    ///
    /// The errors of building the set.
    fn get_tailored_set(&self) -> Result<UnicodeSet> {
        UnicodeSet::new_range(0, 0x10FFFF)
    }

    /// Compares the source text String to the target text String according
    /// to this Collator's rules, strength and decomposition mode.
    ///
    /// Equivalent to `compare(String, String)`.
    fn compare(&self, source: &[u16], target: &[u16]) -> i32;

    /// Compares two CharSequences.
    ///
    /// Equivalent to `doCompare(CharSequence, CharSequence)` (deprecated),
    /// which `compare(Object, Object)` calls.
    fn do_compare(&self, left: &[u16], right: &[u16]) -> i32 {
        self.compare(left, right)
    }

    /// Transforms the String into a CollationKey suitable for efficient
    /// repeated comparison.
    ///
    /// Equivalent to `getCollationKey(String)`; `None` for Java's `null`.
    fn get_collation_key(&self, source: Option<&[u16]>) -> Option<CollationKey>;

    /// Gets the simpler form of a CollationKey for the String source
    /// following the rules of this Collator and stores the result into the
    /// user provided argument key.
    ///
    /// Equivalent to `getRawCollationKey(String, RawCollationKey)`.
    fn get_raw_collation_key(&self, source: Option<&[u16]>, key: Option<RawCollationKey>) -> Option<RawCollationKey>;

    /// Sets the variable top to the top of the specified reordering group.
    ///
    /// Equivalent to `setMaxVariable(int)`, which returns this collator.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless the subclass implements it.
    fn set_max_variable(&mut self, _group: i32) -> Result<()> {
        Err(needs_subclass())
    }

    /// Returns the maximum reordering group whose characters are affected
    /// by the alternate handling behavior.
    ///
    /// Equivalent to `getMaxVariable()`.
    fn get_max_variable(&self) -> i32 {
        ReorderCodes::PUNCTUATION
    }

    /// Sets the variable top to the primary weight of the specified string.
    ///
    /// Equivalent to `setVariableTop(String)` (deprecated).
    ///
    /// # Errors
    ///
    /// As the subclass throws.
    fn set_variable_top_str(&mut self, var_top: Option<&[u16]>) -> Result<i32>;

    /// Gets the variable top value of a Collator.
    ///
    /// Equivalent to `getVariableTop()`.
    fn get_variable_top(&self) -> i32;

    /// Sets the variable top to the specified primary weight.
    ///
    /// Equivalent to `setVariableTop(int)` (deprecated).
    ///
    /// # Errors
    ///
    /// As the subclass throws.
    fn set_variable_top(&mut self, var_top: i32) -> Result<()>;

    /// Get the version of this collator object.
    ///
    /// Equivalent to `getVersion()`.
    ///
    /// # Errors
    ///
    /// The errors of `VersionInfo.getInstance`.
    fn get_version(&self) -> Result<VersionInfo>;

    /// Get the UCA version of this collator object.
    ///
    /// Equivalent to `getUCAVersion()`.
    ///
    /// # Errors
    ///
    /// The errors of `VersionInfo.getInstance`.
    fn get_uca_version(&self) -> Result<VersionInfo>;

    /// Retrieves the reordering codes for this collator.
    ///
    /// Equivalent to `getReorderCodes()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless the subclass implements it.
    fn get_reorder_codes(&self) -> Result<Vec<i32>> {
        Err(needs_subclass())
    }

    /// Determines whether the object has been frozen or not.
    ///
    /// Equivalent to `isFrozen()`.
    fn is_frozen(&self) -> bool {
        false
    }

    /// Freezes the collator.
    ///
    /// Equivalent to `freeze()`, which returns this collator.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless the subclass implements it.
    fn freeze(&mut self) -> Result<()> {
        Err(needs_subclass())
    }

    /// Provides for the clone operation. Any clone is initially unfrozen.
    ///
    /// Equivalent to `cloneAsThawed()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless the subclass implements it.
    fn clone_as_thawed(&self) -> Result<Box<dyn Collator>> {
        Err(needs_subclass())
    }

    /// Returns the locale that was used to create this object, or the
    /// locale that the data actually came from.
    ///
    /// Equivalent to `getLocale(ULocale.Type)`; `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an unknown type (subclasses).
    fn get_locale(&self, _type: Type) -> Result<Option<ULocale>> {
        Ok(Some(ULocale::root()))
    }

    /// Sets information about the locales that were used to create this
    /// object.
    ///
    /// Equivalent to `setLocale(ULocale, ULocale)` (package private).
    fn set_locale(&mut self, _valid: Option<ULocale>, _actual: Option<ULocale>) {}
}

/// A factory used with registerFactory to register multiple collators and
/// provide display names for them.
///
/// Equivalent to the abstract class `Collator.CollatorFactory`. Java's two
/// `createCollator` overloads call each other by default; a factory
/// overrides at least one, as in Java (otherwise the calls recurse, as
/// they do in Java).
pub trait CollatorFactory: Send + Sync + Any {
    /// Return true if this factory will be visible.
    ///
    /// Equivalent to `visible()`.
    fn visible(&self) -> bool {
        true
    }

    /// Return an instance of the appropriate collator.
    ///
    /// Equivalent to `createCollator(ULocale)`.
    ///
    /// # Errors
    ///
    /// Whatever the factory throws.
    fn create_collator(&self, loc: &ULocale) -> Result<Option<Box<dyn Collator>>> {
        self.create_collator_locale(&loc.to_locale())
    }

    /// Return an instance of the appropriate collator.
    ///
    /// Equivalent to `createCollator(Locale)`.
    ///
    /// # Errors
    ///
    /// Whatever the factory throws.
    fn create_collator_locale(&self, loc: &Locale) -> Result<Option<Box<dyn Collator>>> {
        let u = ULocale::for_locale(Some(loc))?.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        self.create_collator(&u)
    }

    /// Return the name of the collator for the objectLocale, localized for
    /// the displayLocale.
    ///
    /// Equivalent to `getDisplayName(ULocale, ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `getSupportedLocaleIDs()` and
    /// `ULocale.getDisplayName(ULocale)` (`UnsupportedOperationException`
    /// until `LocaleDisplayNames` is ported).
    fn get_display_name(&self, object_locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
        if self.visible() {
            let supported = self.get_supported_locale_ids()?;
            let name = object_locale.get_base_name();
            if supported.contains(&name) {
                return crate::r#impl::icu_locale_service::locale_key_factory_display_name(
                    object_locale.get_name(),
                    Some(display_locale),
                );
            }
        }
        Ok(None)
    }

    /// Return an unmodifiable collection of the locale names directly
    /// supported by this factory.
    ///
    /// Equivalent to `getSupportedLocaleIDs()`.
    ///
    /// # Errors
    ///
    /// Whatever the factory throws.
    fn get_supported_locale_ids(&self) -> Result<HashSet<String>>;
}

/// `Collator.ServiceShim` (package private abstract class).
pub(crate) trait ServiceShim: Send + Sync {
    fn get_instance(&self, l: &ULocale) -> Result<Box<dyn Collator>>;
    fn register_instance(&self, c: Box<dyn Collator>, l: &ULocale) -> Result<RegistryKey>;
    fn register_factory(&self, f: Arc<dyn CollatorFactory>) -> Result<RegistryKey>;
    fn unregister(&self, k: &RegistryKey) -> bool;
    fn get_available_locales(&self) -> Result<Vec<Locale>>; // TODO remove
    fn get_available_u_locales(&self) -> Result<Vec<ULocale>>;
    fn get_display_name(&self, ol: &ULocale, dl: &ULocale) -> Result<Option<String>>;
}

/// `shim`: created on first use.
///
/// Java loads `com.ibm.icu.text.CollatorServiceShim` by name
/// (`Class.forName` + `newInstance`) so that code without the service
/// does not depend on it statically; the class is part of this crate, so
/// it is created directly. Java's `shim == null` tests read whether it has
/// been created.
static SHIM: OnceLock<Box<dyn ServiceShim>> = OnceLock::new();

fn get_shim() -> &'static dyn ServiceShim {
    // Note: this instantiation is safe on loose-memory-model configurations
    // despite lack of synchronization, since the shim instance has no state--
    // it's all in the class init.  The worst problem is we might instantiate
    // two shim instances, but they'll share the same state so that's ok.
    SHIM.get_or_init(|| Box::new(CollatorServiceShim)).as_ref()
}

/// `ASCII.equalIgnoreCase(CharSequence, CharSequence)`.
fn ascii_equal_ignore_case(left: &str, right: &str) -> bool {
    let l = units(left);
    let r = units(right);
    let length = l.len();
    if length != r.len() {
        return false;
    }
    for i in 0..length {
        let lc = l[i] as i32;
        let rc = r[i] as i32;
        if lc == rc {
            continue;
        }
        if ('A' as i32..='Z' as i32).contains(&lc) {
            if (lc + 0x20) == rc {
                continue;
            }
        } else if ('A' as i32..='Z' as i32).contains(&rc) && (rc + 0x20) == lc {
            continue;
        }
        return false;
    }
    true
}

fn get_yes_or_no(keyword: &str, s: &str) -> Result<bool> {
    if ascii_equal_ignore_case(s, "yes") {
        return Ok(true);
    }
    if ascii_equal_ignore_case(s, "no") {
        return Ok(false);
    }
    Err(LuceneError::illegal_argument(format!("illegal locale keyword=value: {keyword}={s}")))
}

fn get_int_value(keyword: &str, s: &str, values: &[&str]) -> Result<i32> {
    for (i, v) in values.iter().enumerate() {
        if ascii_equal_ignore_case(s, v) {
            return Ok(i as i32);
        }
    }
    Err(LuceneError::illegal_argument(format!("illegal locale keyword=value: {keyword}={s}")))
}

fn get_reorder_code(keyword: &str, s: &str) -> Result<i32> {
    Ok(ReorderCodes::FIRST + get_int_value(keyword, s, &["space", "punct", "symbol", "currency", "digit"])?)
    // Not supporting "others" = UCOL_REORDER_CODE_OTHERS
    // as a synonym for Zzzz = USCRIPT_UNKNOWN for now:
    // Avoid introducing synonyms/aliases.
}

/// Sets collation attributes according to locale keywords. See
/// http://www.unicode.org/reports/tr35/tr35-collation.html#Collation_Settings
fn set_attributes_from_keywords(loc: &ULocale, coll: &mut dyn Collator) -> Result<()> {
    // Check for collation keywords that were already deprecated
    // before any were supported in createInstance() (except for "collation").
    let mut value = loc.get_keyword_value("colHiraganaQuaternary");
    if value.is_some() {
        return Err(LuceneError::unsupported_operation("locale keyword kh/colHiraganaQuaternary"));
    }
    value = loc.get_keyword_value("variableTop");
    if value.is_some() {
        return Err(LuceneError::unsupported_operation("locale keyword vt/variableTop"));
    }
    // Parse known collation keywords, ignore others.
    value = loc.get_keyword_value("colStrength");
    if let Some(value) = &value {
        // Note: Not supporting typo "quarternary" because it was never supported in locale IDs.
        let strength =
            get_int_value("colStrength", value, &["primary", "secondary", "tertiary", "quaternary", "identical"])?;
        coll.set_strength(if strength <= QUATERNARY { strength } else { IDENTICAL })?;
    }

    if let Some(value) = loc.get_keyword_value("colBackwards") {
        match coll.as_rule_based_mut() {
            Some(rbc) => rbc.set_french_collation(get_yes_or_no("colBackwards", &value)?)?,
            None => {
                return Err(LuceneError::unsupported_operation(
                    "locale keyword kb/colBackwards only settable for RuleBasedCollator",
                ))
            }
        }
    }

    if let Some(value) = loc.get_keyword_value("colCaseLevel") {
        match coll.as_rule_based_mut() {
            Some(rbc) => rbc.set_case_level(get_yes_or_no("colCaseLevel", &value)?)?,
            None => {
                return Err(LuceneError::unsupported_operation(
                    "locale keyword kb/colBackwards only settable for RuleBasedCollator",
                ))
            }
        }
    }

    if let Some(value) = loc.get_keyword_value("colCaseFirst") {
        match coll.as_rule_based_mut() {
            Some(rbc) => {
                let cf = get_int_value("colCaseFirst", &value, &["no", "lower", "upper"])?;
                if cf == 0 {
                    rbc.set_lower_case_first(false)?;
                    rbc.set_upper_case_first(false)?;
                } else if cf == 1 {
                    rbc.set_lower_case_first(true)?;
                } else {
                    rbc.set_upper_case_first(true)?;
                }
            }
            None => {
                return Err(LuceneError::unsupported_operation(
                    "locale keyword kf/colCaseFirst only settable for RuleBasedCollator",
                ))
            }
        }
    }

    if let Some(value) = loc.get_keyword_value("colAlternate") {
        match coll.as_rule_based_mut() {
            Some(rbc) => rbc.set_alternate_handling_shifted(
                get_int_value("colAlternate", &value, &["non-ignorable", "shifted"])? != 0,
            )?,
            None => {
                return Err(LuceneError::unsupported_operation(
                    "locale keyword ka/colAlternate only settable for RuleBasedCollator",
                ))
            }
        }
    }

    if let Some(value) = loc.get_keyword_value("colNormalization") {
        coll.set_decomposition(if get_yes_or_no("colNormalization", &value)? {
            CANONICAL_DECOMPOSITION
        } else {
            NO_DECOMPOSITION
        })?;
    }

    if let Some(value) = loc.get_keyword_value("colNumeric") {
        match coll.as_rule_based_mut() {
            Some(rbc) => rbc.set_numeric_collation(get_yes_or_no("colNumeric", &value)?)?,
            None => {
                return Err(LuceneError::unsupported_operation(
                    "locale keyword kn/colNumeric only settable for RuleBasedCollator",
                ))
            }
        }
    }

    if let Some(value) = loc.get_keyword_value("colReorder") {
        #[allow(deprecated)]
        let capacity = (UScript::CODE_LIMIT + ReorderCodes::LIMIT - ReorderCodes::FIRST) as usize;
        let mut codes = vec![0i32; capacity];
        let mut codes_length = 0usize;
        let vu = units(&value);
        let mut script_name_start = 0usize;
        loop {
            if codes_length == codes.len() {
                return Err(LuceneError::illegal_argument(format!(
                    "too many script codes for colReorder locale keyword: {value}"
                )));
            }
            let mut limit = script_name_start;
            while limit < vu.len() && vu[limit] != '-' as u16 {
                limit += 1;
            }
            let script_name = &vu[script_name_start..limit];
            let code = if script_name.len() == 4 {
                // Strict parsing, accept only 4-letter script codes, not long names.
                UCharacter::get_property_value_enum(UProperty::SCRIPT, script_name)?
            } else {
                get_reorder_code("colReorder", &to_rust_string(script_name))?
            };
            codes[codes_length] = code;
            codes_length += 1;
            if limit == vu.len() {
                break;
            }
            script_name_start = limit + 1;
        }
        if codes_length == 0 {
            return Err(LuceneError::illegal_argument("no script codes for colReorder locale keyword"));
        }
        coll.set_reorder_codes(Some(&codes[..codes_length]))?;
    }

    if let Some(value) = loc.get_keyword_value("kv") {
        coll.set_max_variable(get_reorder_code("kv", &value)?)?;
    }
    Ok(())
}

/// Returns the Collator for the current default locale.
///
/// Equivalent to `getInstance()`.
///
/// # Errors
///
/// The errors of loading the collation data.
pub fn get_instance() -> Result<Box<dyn Collator>> {
    get_instance_ulocale(Some(&ULocale::get_default()))
}

/// Returns the Collator for the desired locale.
///
/// Equivalent to `getInstance(ULocale)`; `None` for Java's `null` (the
/// default locale).
///
/// # Errors
///
/// The errors of loading the collation data; `IllegalArgumentException` and
/// `UnsupportedOperationException` for invalid collation keywords.
pub fn get_instance_ulocale(locale: Option<&ULocale>) -> Result<Box<dyn Collator>> {
    // fetching from service cache is faster than instantiation
    let default;
    let locale = match locale {
        Some(l) => l,
        None => {
            default = ULocale::get_default();
            &default
        }
    };
    let mut coll = get_shim().get_instance(locale)?;
    if locale.get_name() != locale.get_base_name() {
        // any keywords?
        set_attributes_from_keywords(locale, coll.as_mut())?;
    }
    Ok(coll)
}

/// Returns the Collator for the desired locale.
///
/// Equivalent to `getInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_ulocale`].
pub fn get_instance_locale(locale: Option<&Locale>) -> Result<Box<dyn Collator>> {
    get_instance_ulocale(ULocale::for_locale(locale)?.as_ref())
}

/// Registers a collator as the default collator for the provided locale.
///
/// Equivalent to `registerInstance(Collator, ULocale)`.
///
/// # Errors
///
/// The errors of the service.
pub fn register_instance(collator: Box<dyn Collator>, locale: &ULocale) -> Result<RegistryKey> {
    get_shim().register_instance(collator, locale)
}

/// Registers a collator factory.
///
/// Equivalent to `registerFactory(CollatorFactory)`.
///
/// # Errors
///
/// The errors of the service.
pub fn register_factory(factory: Arc<dyn CollatorFactory>) -> Result<RegistryKey> {
    get_shim().register_factory(factory)
}

/// Unregisters a collator previously registered using registerInstance.
///
/// Equivalent to `unregister(Object)`.
pub fn unregister(registry_key: &RegistryKey) -> bool {
    match SHIM.get() {
        None => false,
        Some(shim) => shim.unregister(registry_key),
    }
}

/// Returns the set of locales, as Locale objects, for which collators are
/// installed.
///
/// Equivalent to `getAvailableLocales()`.
///
/// # Errors
///
/// The errors of reading the available locales.
pub fn get_available_locales() -> Result<Vec<Locale>> {
    // TODO make this wrap getAvailableULocales later
    match SHIM.get() {
        None => Ok((*ICUResourceBundle::get_available_locales_loader(
            ICUData::ICU_COLLATION_BASE_NAME,
            icu_data_class_loader(),
        )?)
        .clone()),
        Some(shim) => shim.get_available_locales(),
    }
}

/// Returns the set of locales, as ULocale objects, for which collators are
/// installed.
///
/// Equivalent to `getAvailableULocales()`.
///
/// # Errors
///
/// The errors of reading the available locales.
pub fn get_available_u_locales() -> Result<Vec<ULocale>> {
    match SHIM.get() {
        None => Ok(ICUResourceBundle::get_available_u_locales_loader(
            ICUData::ICU_COLLATION_BASE_NAME,
            icu_data_class_loader(),
        )?
        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))),
        Some(shim) => shim.get_available_u_locales(),
    }
}

/// The list of keywords for collation.
const KEYWORDS: [&str; 1] = ["collation"];

/// The resource name for collation.
const RESOURCE: &str = "collations";

/// The resource bundle base name for collation.
const BASE: &str = ICUData::ICU_COLLATION_BASE_NAME;

/// Returns an array of all possible keywords that are relevant to
/// collation.
///
/// Equivalent to `getKeywords()`.
pub fn get_keywords() -> Vec<String> {
    KEYWORDS.iter().map(|s| s.to_string()).collect()
}

/// Given a keyword, returns an array of all values for that keyword that
/// are currently in use.
///
/// Equivalent to `getKeywordValues(String)`.
///
/// # Errors
///
/// `IllegalArgumentException` for an invalid keyword.
pub fn get_keyword_values(keyword: &str) -> Result<Vec<String>> {
    if keyword != KEYWORDS[0] {
        return Err(LuceneError::illegal_argument(format!("Invalid keyword: {keyword}")));
    }
    ICUResourceBundle::get_keyword_values(BASE, RESOURCE)
}

/// Given a key and a locale, returns an array of string values in a
/// preferred order that would make a difference.
///
/// Equivalent to `getKeywordValuesForLocale(String, ULocale, boolean)`.
///
/// # Errors
///
/// The errors of reading the collation bundles.
pub fn get_keyword_values_for_locale(_key: &str, locale: &ULocale, _commonly_used: bool) -> Result<Vec<String>> {
    // Note: The parameter commonlyUsed is not used.
    // The switch is in the method signature for consistency
    // with other locale services.

    // Read available collation values from collation bundles.
    let bundle = UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_COLLATION_BASE_NAME), Some(locale))?;
    let bundle = match bundle {
        UResourceBundle::Icu(b) => b,
        UResourceBundle::Wrapper(_) => panic!(
            "java.lang.ClassCastException: class com.ibm.icu.impl.ResourceBundleWrapper cannot be cast to class com.ibm.icu.impl.ICUResourceBundle"
        ),
    };
    let mut sink = KeywordsSink { values: VecDeque::new(), has_default: false };
    bundle.get_all_items_with_fallback("collations", &mut sink)?;
    Ok(sink.values.into_iter().collect())
}

/// `Collator.KeywordsSink` (private).
struct KeywordsSink {
    values: VecDeque<String>,
    has_default: bool,
}

impl Sink for KeywordsSink {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let collations = value.get_table()?;
        let mut i = 0;
        while collations.get_key_and_value(i, key, value) {
            let ty = value.get_type();
            if ty == UResourceBundle::STRING {
                if !self.has_default && key.content_equals("default") {
                    let defcoll = to_rust_string(&value.get_string()?);
                    if !defcoll.is_empty() {
                        if let Some(pos) = self.values.iter().position(|v| *v == defcoll) {
                            self.values.remove(pos);
                        }
                        self.values.push_front(defcoll);
                        self.has_default = true;
                    }
                }
            } else if ty == UResourceBundle::TABLE && !key.starts_with("private-") {
                let collkey = key.to_java_string();
                if !self.values.contains(&collkey) {
                    self.values.push_back(collkey);
                }
            }
            i += 1;
        }
        Ok(())
    }
}

/// Returns the functionally equivalent locale for the given requested
/// locale, with respect to given keyword, for the collation service.
///
/// Equivalent to `getFunctionalEquivalent(String, ULocale, boolean[])`.
///
/// # Errors
///
/// `MissingResourceException` and the errors of bundle loading.
pub fn get_functional_equivalent(keyword: &str, loc_id: &ULocale, is_available: Option<&mut bool>) -> Result<ULocale> {
    ICUResourceBundle::get_functional_equivalent(
        BASE,
        icu_data_class_loader(),
        RESOURCE,
        keyword,
        loc_id,
        is_available,
        true,
    )
}

/// Returns the functionally equivalent locale for the given requested
/// locale, with respect to given keyword, for the collation service.
///
/// Equivalent to `getFunctionalEquivalent(String, ULocale)`.
///
/// # Errors
///
/// As [`get_functional_equivalent`].
pub fn get_functional_equivalent2(keyword: &str, loc_id: &ULocale) -> Result<ULocale> {
    get_functional_equivalent(keyword, loc_id, None)
}

/// Returns the name of the collator for the objectLocale, localized for
/// the displayLocale.
///
/// Equivalent to `getDisplayName(Locale, Locale)`.
///
/// # Errors
///
/// The errors of the service (`ULocale.getDisplayName` is not ported yet).
pub fn get_display_name_locale(object_locale: &Locale, display_locale: &Locale) -> Result<Option<String>> {
    let ol = ULocale::for_locale(Some(object_locale))?.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
    let dl = ULocale::for_locale(Some(display_locale))?.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
    get_shim().get_display_name(&ol, &dl)
}

/// Returns the name of the collator for the objectLocale, localized for
/// the displayLocale.
///
/// Equivalent to `getDisplayName(ULocale, ULocale)`.
///
/// # Errors
///
/// The errors of the service.
pub fn get_display_name(object_locale: &ULocale, display_locale: &ULocale) -> Result<Option<String>> {
    get_shim().get_display_name(object_locale, display_locale)
}

/// Returns the name of the collator for the objectLocale, localized for
/// the default DISPLAY locale.
///
/// Equivalent to `getDisplayName(Locale)`.
///
/// # Errors
///
/// The errors of the service.
pub fn get_display_name_default_locale(object_locale: &Locale) -> Result<Option<String>> {
    let ol = ULocale::for_locale(Some(object_locale))?.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
    get_shim().get_display_name(&ol, &ULocale::get_default_category(Category::Display))
}

/// Returns the name of the collator for the objectLocale, localized for
/// the default DISPLAY locale.
///
/// Equivalent to `getDisplayName(ULocale)`.
///
/// # Errors
///
/// The errors of the service.
pub fn get_display_name_default(object_locale: &ULocale) -> Result<Option<String>> {
    get_shim().get_display_name(object_locale, &ULocale::get_default_category(Category::Display))
}

/// Retrieves all the reorder codes that are grouped with the given
/// reorder code.
///
/// Equivalent to `getEquivalentReorderCodes(int)`.
///
/// # Errors
///
/// The errors of loading the root collation data.
pub fn get_equivalent_reorder_codes(reorder_code: i32) -> Result<Vec<i32>> {
    let base_data = CollationRoot::get_data()?;
    Ok(base_data.get_equivalent_scripts(reorder_code))
}
