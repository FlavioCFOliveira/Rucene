// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.RuleBasedCollator`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::CharacterIterator;

use crate::jdk::{char_count, code_point_at};
use crate::r#impl::coll::bocsu::BOCSU;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_builder::CollationBuilder;
use crate::r#impl::coll::collation_compare::CollationCompare;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fast_latin::CollationFastLatin;
use crate::r#impl::coll::collation_iterator::CollationIterator;
use crate::r#impl::coll::collation_keys::{CollationKeys, SortKeyByteSink, SortKeyByteSinkBase, SIMPLE_LEVEL_FALLBACK};
use crate::r#impl::coll::collation_loader::CollationLoader;
use crate::r#impl::coll::collation_root::CollationRoot;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::r#impl::coll::collation_tailoring::CollationTailoring;
use crate::r#impl::coll::contractions_and_expansions::ContractionsAndExpansions;
use crate::r#impl::coll::fcd_utf16_collation_iterator::FCDUTF16CollationIterator;
use crate::r#impl::coll::shared_object::Reference;
use crate::r#impl::coll::tailored_set::TailoredSet;
use crate::r#impl::coll::utf16_collation_iterator::UTF16CollationIterator;
use crate::r#impl::normalizer2_impl::{Normalizer2Impl, ReorderingBuffer};
use crate::text::collation_element_iterator::CollationElementIterator;
use crate::text::collation_key::CollationKey;
use crate::text::collator::{self, Collator, ReorderCodes};
use crate::text::raw_collation_key::RawCollationKey;
use crate::text::u_character_iterator::UCharacterIterator;
use crate::text::unicode_set::UnicodeSet;
use crate::text::unicode_set_iterator::{UnicodeSetIterator, IS_STRING};
use crate::util::u_locale::{Type, ULocale};
use crate::util::version_info::VersionInfo;

fn frozen_error() -> LuceneError {
    LuceneError::unsupported_operation("Attempt to modify frozen RuleBasedCollator")
}

/// `RuleBasedCollator.CollationKeyByteSink` (private).
struct CollationKeyByteSink<'k> {
    base: SortKeyByteSinkBase,
    key_: &'k mut RawCollationKey,
}

impl<'k> CollationKeyByteSink<'k> {
    fn new(key: &'k mut RawCollationKey) -> Self {
        let bytes = key.bytes.take().unwrap_or_default();
        Self { base: SortKeyByteSinkBase::new(bytes), key_: key }
    }

    /// Java's `buffer_` and `key_.bytes` alias one array; here the sink owns
    /// it while writing and hands it back to the key.
    fn sync_key_bytes(&mut self) {
        self.key_.bytes = Some(self.base.buffer.clone());
    }
}

impl SortKeyByteSink for CollationKeyByteSink<'_> {
    fn sink(&self) -> &SortKeyByteSinkBase {
        &self.base
    }

    fn sink_mut(&mut self) -> &mut SortKeyByteSinkBase {
        &mut self.base
    }

    fn append_beyond_capacity(&mut self, bytes: &[i8], start: i32, n: i32, length: i32) {
        // n > 0 && appended_ > capacity_
        if self.resize(n, length) {
            self.base.buffer[length as usize..(length + n) as usize]
                .copy_from_slice(&bytes[start as usize..(start + n) as usize]);
        }
    }

    fn resize(&mut self, append_capacity: i32, length: i32) -> bool {
        let mut new_capacity = 2 * self.base.buffer.len() as i32;
        let alt_capacity = length + 2 * append_capacity;
        if new_capacity < alt_capacity {
            new_capacity = alt_capacity;
        }
        if new_capacity < 200 {
            new_capacity = 200;
        }
        // Do not call key_.ensureCapacity(newCapacity) because we do not
        // keep key_.size in sync with appended_.
        // We only set it when we are done.
        let mut new_bytes = vec![0i8; new_capacity as usize];
        new_bytes[..length as usize].copy_from_slice(&self.base.buffer[..length as usize]);
        self.base.buffer = new_bytes;
        true
    }
}

/// `RuleBasedCollator.NFDIterator` and its subclasses (private): the
/// state of `UTF16NFDIterator`/`FCDUTF16NFDIterator`.
#[derive(Debug, Clone, Default)]
struct NFDIterator {
    decomp: Option<Vec<u16>>,
    index: i32,
    /// `UTF16NFDIterator.s` (copied text, or the FCD buffer).
    s: Vec<u16>,
    pos: i32,
    /// `FCDUTF16NFDIterator.str`.
    str: Option<Vec<u16>>,
}

impl NFDIterator {
    fn reset(&mut self) {
        self.index = -1;
    }

    fn next_code_point(&mut self) -> i32 {
        if self.index >= 0 {
            let decomp = self.decomp.as_ref().expect("decomposition in progress");
            if self.index == decomp.len() as i32 {
                self.index = -1;
            } else {
                let c = code_point_at(decomp, self.index);
                self.index += char_count(c);
                return c;
            }
        }
        self.next_raw_code_point()
    }

    fn next_decomposed_code_point(&mut self, nfc_impl: &Normalizer2Impl, mut c: i32) -> i32 {
        if self.index >= 0 {
            return c;
        }
        self.decomp = nfc_impl.get_decomposition(c);
        let Some(decomp) = self.decomp.as_ref() else {
            return c;
        };
        c = code_point_at(decomp, 0);
        self.index = char_count(c);
        c
    }

    /// `UTF16NFDIterator.nextRawCodePoint()`.
    fn next_raw_code_point(&mut self) -> i32 {
        if self.pos == self.s.len() as i32 {
            return Collation::SENTINEL_CP;
        }
        let c = code_point_at(&self.s, self.pos);
        self.pos += char_count(c);
        c
    }

    /// `UTF16NFDIterator.setText(CharSequence, int)`.
    fn set_text(&mut self, seq: &[u16], start: i32) {
        self.reset();
        self.s.clear();
        self.s.extend_from_slice(seq);
        self.pos = start;
    }

    /// `FCDUTF16NFDIterator.setText(Normalizer2Impl, CharSequence, int)`.
    fn set_text_fcd(&mut self, nfc_impl: &Normalizer2Impl, seq: &[u16], start: i32) {
        self.reset();
        let span_limit = nfc_impl.make_fcd(seq, start, seq.len() as i32, None);
        if span_limit == seq.len() as i32 {
            self.s.clear();
            self.s.extend_from_slice(seq);
            self.pos = start;
        } else {
            let mut str = self.str.take().unwrap_or_default();
            str.clear();
            str.extend_from_slice(&seq[start as usize..span_limit as usize]);
            {
                let mut buffer = ReorderingBuffer::new(nfc_impl, &mut str, seq.len() as i32 - start);
                nfc_impl.make_fcd(seq, span_limit, seq.len() as i32, Some(&mut buffer));
            }
            self.s.clear();
            self.s.extend_from_slice(&str);
            self.str = Some(str);
            self.pos = 0;
        }
    }
}

fn compare_nfd_iter(nfc_impl: &Normalizer2Impl, left: &mut NFDIterator, right: &mut NFDIterator) -> i32 {
    loop {
        // Fetch the next FCD code point from each string.
        let mut left_cp = left.next_code_point();
        let mut right_cp = right.next_code_point();
        if left_cp == right_cp {
            if left_cp < 0 {
                break;
            }
            continue;
        }
        // If they are different, then decompose each and compare again.
        if left_cp < 0 {
            left_cp = -2; // end of string
        } else if left_cp == 0xfffe {
            left_cp = -1; // U+FFFE: merge separator
        } else {
            left_cp = left.next_decomposed_code_point(nfc_impl, left_cp);
        }
        if right_cp < 0 {
            right_cp = -2; // end of string
        } else if right_cp == 0xfffe {
            right_cp = -1; // U+FFFE: merge separator
        } else {
            right_cp = right.next_decomposed_code_point(nfc_impl, right_cp);
        }
        if left_cp < right_cp {
            return Collation::LESS;
        }
        if left_cp > right_cp {
            return Collation::GREATER;
        }
    }
    Collation::EQUAL
}

/// `RuleBasedCollator.CollationBuffer` (private).
#[derive(Debug)]
struct CollationBuffer {
    left_utf16_coll_iter: UTF16CollationIterator,
    right_utf16_coll_iter: UTF16CollationIterator,
    left_fcd_utf16_iter: FCDUTF16CollationIterator,
    right_fcd_utf16_iter: FCDUTF16CollationIterator,
    left_utf16_nfd_iter: NFDIterator,
    right_utf16_nfd_iter: NFDIterator,
    left_fcd_utf16_nfd_iter: NFDIterator,
    right_fcd_utf16_nfd_iter: NFDIterator,
    raw_collation_key: Option<RawCollationKey>,
}

impl CollationBuffer {
    fn new(data: &Arc<CollationData>) -> Self {
        Self {
            left_utf16_coll_iter: UTF16CollationIterator::new(data.clone()),
            right_utf16_coll_iter: UTF16CollationIterator::new(data.clone()),
            left_fcd_utf16_iter: FCDUTF16CollationIterator::new(data.clone()),
            right_fcd_utf16_iter: FCDUTF16CollationIterator::new(data.clone()),
            left_utf16_nfd_iter: NFDIterator::default(),
            right_utf16_nfd_iter: NFDIterator::default(),
            left_fcd_utf16_nfd_iter: NFDIterator::default(),
            right_fcd_utf16_nfd_iter: NFDIterator::default(),
            raw_collation_key: None,
        }
    }
}

/// RuleBasedCollator is a concrete subclass of Collator. It allows
/// customization of the Collator via user-specified rule sets.
///
/// Equivalent to `com.ibm.icu.text.RuleBasedCollator`.
///
/// **Divergence.** Java lets a non-frozen collator reuse its
/// `collationBuffer` without synchronization (not thread-safe) and takes
/// `frozenLock` only when frozen (`RuleBasedCollator.java:1846-1866`); the
/// buffer is behind a [`Mutex`] here in both states, because Rust forbids
/// unsynchronized shared mutation. Single-threaded results are the same;
/// concurrent use of a non-frozen collator, a data race in Java, is
/// serialized. `clone()` of a frozen collator returns `this` in Java
/// (`RuleBasedCollator.java:313-318`); [`Collator::clone_box`] returns a
/// frozen copy sharing the same immutable data and settings.
#[derive(Debug)]
pub struct RuleBasedCollator {
    pub(crate) data: Arc<CollationData>,
    /// reference-counted
    pub(crate) settings: Reference<CollationSettings>,
    /// C++: reference-counted
    pub(crate) tailoring: Arc<CollationTailoring>,
    valid_locale: Option<ULocale>,
    // Note: No need in Java to track which attributes have been set explicitly.
    // int or EnumSet  explicitlySetAttributes;
    actual_locale_is_same_as_valid: bool,
    /// `frozenLock != null`.
    frozen: bool,
    collation_buffer: Mutex<Option<Box<CollationBuffer>>>,
}

impl RuleBasedCollator {
    /// Constructor that takes the argument rules for customization.
    ///
    /// Equivalent to `RuleBasedCollator(String rules)`; `None` for Java's
    /// `null`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for null rules; `ParseException` for
    /// invalid rules; the exceptions of building the tailoring.
    pub fn new(rules: Option<&[u16]>) -> Result<Self> {
        let Some(rules) = rules else {
            return Err(LuceneError::illegal_argument("Collation rules can not be null"));
        };
        let t = Self::internal_build_tailoring(rules)?;
        let mut c = Self::adopt_tailoring(t);
        c.valid_locale = c.tailoring.actual_locale.clone();
        Ok(c)
    }

    /// Implements from-rule constructors.
    ///
    /// Java builds through reflection (`loadClass("...CollationBuilder")`,
    /// so that code using Collator need not depend on the builder
    /// statically) and rethrows the builder's exception; the builder is
    /// part of this crate and is called directly.
    fn internal_build_tailoring(rules: &[u16]) -> Result<Arc<CollationTailoring>> {
        let base = CollationRoot::get_root()?;
        let mut builder = CollationBuilder::new(base)?;
        let mut t = builder.parse_and_build(rules)?;
        t.actual_locale = None;
        Ok(Arc::new(t))
    }

    /// `adoptTailoring(CollationTailoring)` (with the field defaults of the
    /// rules constructor, where `validLocale = ULocale.ROOT` is replaced by
    /// `t.actualLocale`).
    fn adopt_tailoring(t: Arc<CollationTailoring>) -> Self {
        Self {
            data: t.data().clone(),
            settings: t.settings.clone(),
            valid_locale: t.actual_locale.clone(),
            tailoring: t,
            actual_locale_is_same_as_valid: false,
            frozen: false,
            collation_buffer: Mutex::new(None),
        }
    }

    /// `RuleBasedCollator(CollationTailoring t, ULocale vl)` (package
    /// private).
    pub(crate) fn from_tailoring(t: Arc<CollationTailoring>, vl: Option<ULocale>) -> Self {
        Self {
            data: t.data().clone(),
            settings: t.settings.clone(),
            tailoring: t,
            valid_locale: vl,
            actual_locale_is_same_as_valid: false,
            frozen: false,
            collation_buffer: Mutex::new(None),
        }
    }

    fn ro(&self) -> &CollationSettings {
        self.settings.read_only().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn init_max_expansions(&self) {
        // synchronized(tailoring): set once.
        self.tailoring
            .max_expansions
            .get_or_init(|| CollationElementIterator::compute_max_expansions(&self.tailoring.data().clone()));
    }

    /// Return a CollationElementIterator for the given String.
    ///
    /// Equivalent to `getCollationElementIterator(String)`.
    pub fn get_collation_element_iterator(&self, source: &[u16]) -> CollationElementIterator<'_> {
        self.init_max_expansions();
        CollationElementIterator::from_string(source, self)
    }

    /// Return a CollationElementIterator for the given CharacterIterator.
    /// The source iterator's integrity will be preserved since a new copy
    /// will be created for use.
    ///
    /// Equivalent to `getCollationElementIterator(CharacterIterator)`.
    pub fn get_collation_element_iterator_ci(&self, source: &dyn CharacterIterator) -> CollationElementIterator<'_> {
        self.init_max_expansions();
        let newsource = source.clone_box();
        CollationElementIterator::from_character_iterator(newsource, self)
    }

    /// Return a CollationElementIterator for the given UCharacterIterator.
    /// The source iterator's integrity will be preserved since a new copy
    /// will be created for use.
    ///
    /// Equivalent to `getCollationElementIterator(UCharacterIterator)`.
    pub fn get_collation_element_iterator_uci(&self, source: &dyn UCharacterIterator) -> CollationElementIterator<'_> {
        self.init_max_expansions();
        CollationElementIterator::from_u_character_iterator(source, self)
    }

    /// `cloneAsThawed()` with the concrete type.
    pub fn clone_as_thawed_rbc(&self) -> RuleBasedCollator {
        // since all collation data in the RuleBasedCollator do not change
        // we can safely assign the result.fields to this collator
        // except in cases where we can't
        RuleBasedCollator {
            data: self.data.clone(),
            settings: self.settings.clone(),
            tailoring: self.tailoring.clone(),
            valid_locale: self.valid_locale.clone(),
            actual_locale_is_same_as_valid: self.actual_locale_is_same_as_valid,
            frozen: false,
            collation_buffer: Mutex::new(None),
        }
    }

    fn check_not_frozen(&self) -> Result<()> {
        if self.frozen {
            return Err(frozen_error());
        }
        Ok(())
    }

    fn get_owned_settings(&mut self) -> &mut CollationSettings {
        self.settings.copy_on_write()
    }

    fn get_default_settings(&self) -> &Arc<CollationSettings> {
        self.tailoring.settings.read_only_arc().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `settings.readOnly() == defaultSettings` (identity).
    fn settings_is_default(&self) -> bool {
        match self.settings.read_only_arc() {
            Some(s) => Arc::ptr_eq(s, self.get_default_settings()),
            None => false,
        }
    }

    /// `setHiraganaQuaternary(boolean)` (deprecated): does nothing.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_hiragana_quaternary(&mut self, _flag: bool) -> Result<()> {
        self.check_not_frozen()
    }

    /// `setHiraganaQuaternaryDefault()` (deprecated): does nothing.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_hiragana_quaternary_default(&mut self) -> Result<()> {
        self.check_not_frozen()
    }

    /// Sets whether uppercase characters sort before lowercase characters
    /// or vice versa, in strength TERTIARY.
    ///
    /// Equivalent to `setUpperCaseFirst(boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_upper_case_first(&mut self, upperfirst: bool) -> Result<()> {
        self.check_not_frozen()?;
        if upperfirst == self.is_upper_case_first() {
            return Ok(());
        }
        self.get_owned_settings()
            .set_case_first(if upperfirst { CollationSettings::CASE_FIRST_AND_UPPER_MASK } else { 0 });
        self.set_fast_latin_options();
        Ok(())
    }

    /// Sets the orders of lower cased characters to sort before upper cased
    /// characters, in strength TERTIARY.
    ///
    /// Equivalent to `setLowerCaseFirst(boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_lower_case_first(&mut self, lowerfirst: bool) -> Result<()> {
        self.check_not_frozen()?;
        if lowerfirst == self.is_lower_case_first() {
            return Ok(());
        }
        self.get_owned_settings().set_case_first(if lowerfirst { CollationSettings::CASE_FIRST } else { 0 });
        self.set_fast_latin_options();
        Ok(())
    }

    fn set_default_with(&mut self, f: impl FnOnce(&mut CollationSettings, i32)) -> Result<()> {
        self.check_not_frozen()?;
        if self.settings_is_default() {
            return Ok(());
        }
        let default_options = self.get_default_settings().options;
        f(self.get_owned_settings(), default_options);
        self.set_fast_latin_options();
        Ok(())
    }

    /// Sets the case first mode to the initial mode set during
    /// construction of the RuleBasedCollator.
    ///
    /// Equivalent to `setCaseFirstDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_case_first_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_case_first_default(d))
    }

    /// Sets the alternate handling mode to the initial mode set during
    /// construction of the RuleBasedCollator.
    ///
    /// Equivalent to `setAlternateHandlingDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_alternate_handling_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_alternate_handling_default(d))
    }

    /// Sets the case level mode to the initial mode set during construction
    /// of the RuleBasedCollator.
    ///
    /// Equivalent to `setCaseLevelDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_case_level_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_flag_default(CollationSettings::CASE_LEVEL, d))
    }

    /// Sets the decomposition mode to the initial mode set during
    /// construction of the RuleBasedCollator.
    ///
    /// Equivalent to `setDecompositionDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_decomposition_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_flag_default(CollationSettings::CHECK_FCD, d))
    }

    /// Sets the French collation mode to the initial mode set during
    /// construction of the RuleBasedCollator.
    ///
    /// Equivalent to `setFrenchCollationDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_french_collation_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_flag_default(CollationSettings::BACKWARD_SECONDARY, d))
    }

    /// Sets the collation strength to the initial mode set during the
    /// construction of the RuleBasedCollator.
    ///
    /// Equivalent to `setStrengthDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_strength_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_strength_default(d))
    }

    /// Method to set numeric collation to its default value.
    ///
    /// Equivalent to `setNumericCollationDefault()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_numeric_collation_default(&mut self) -> Result<()> {
        self.set_default_with(|s, d| s.set_flag_default(CollationSettings::NUMERIC, d))
    }

    /// Sets the mode for the direction of SECONDARY weights to be used in
    /// French collation.
    ///
    /// Equivalent to `setFrenchCollation(boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_french_collation(&mut self, flag: bool) -> Result<()> {
        self.check_not_frozen()?;
        if flag == self.is_french_collation() {
            return Ok(());
        }
        self.get_owned_settings().set_flag(CollationSettings::BACKWARD_SECONDARY, flag);
        self.set_fast_latin_options();
        Ok(())
    }

    /// Sets the alternate handling for QUATERNARY strength to be either
    /// shifted or non-ignorable.
    ///
    /// Equivalent to `setAlternateHandlingShifted(boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_alternate_handling_shifted(&mut self, shifted: bool) -> Result<()> {
        self.check_not_frozen()?;
        if shifted == self.is_alternate_handling_shifted() {
            return Ok(());
        }
        self.get_owned_settings().set_alternate_handling_shifted(shifted);
        self.set_fast_latin_options();
        Ok(())
    }

    /// When case level is set to true, an additional weight is formed
    /// between the SECONDARY and TERTIARY weight.
    ///
    /// Equivalent to `setCaseLevel(boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_case_level(&mut self, flag: bool) -> Result<()> {
        self.check_not_frozen()?;
        if flag == self.is_case_level() {
            return Ok(());
        }
        self.get_owned_settings().set_flag(CollationSettings::CASE_LEVEL, flag);
        self.set_fast_latin_options();
        Ok(())
    }

    fn internal_set_variable_top(&mut self, mut var_top: i64) -> Result<()> {
        if var_top != self.ro().variable_top {
            // Pin the variable top to the end of the reordering group which contains it.
            // Only a few special groups are supported.
            let group = self.data.get_group_for_primary(var_top);
            if !(ReorderCodes::FIRST..=ReorderCodes::CURRENCY).contains(&group) {
                return Err(LuceneError::illegal_argument(
                    "The variable top must be a primary weight in the space/punctuation/symbols/currency symbols range",
                ));
            }
            let v = self.data.get_last_primary_for_group(group);
            debug_assert!(v != 0 && v >= var_top);
            var_top = v;
            if var_top != self.ro().variable_top {
                let default_options = self.get_default_settings().options;
                let owned_settings = self.get_owned_settings();
                owned_settings.set_max_variable(group - ReorderCodes::FIRST, default_options)?;
                owned_settings.variable_top = var_top;
                self.set_fast_latin_options();
            }
        }
        Ok(())
    }

    /// When numeric collation is turned on, this Collator makes substrings
    /// of digits sort according to their numeric values.
    ///
    /// Equivalent to `setNumericCollation(boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` if frozen.
    pub fn set_numeric_collation(&mut self, flag: bool) -> Result<()> {
        self.check_not_frozen()?;
        // sort substrings of digits as numbers
        if flag == self.get_numeric_collation() {
            return Ok(());
        }
        self.get_owned_settings().set_flag(CollationSettings::NUMERIC, flag);
        self.set_fast_latin_options();
        Ok(())
    }

    fn set_fast_latin_options(&mut self) {
        let data = self.data.clone();
        let owned_settings = self.get_owned_settings();
        let mut primaries = std::mem::take(&mut owned_settings.fast_latin_primaries);
        owned_settings.fast_latin_options = CollationFastLatin::get_options(&data, owned_settings, &mut primaries);
        owned_settings.fast_latin_primaries = primaries;
    }

    /// Gets the collation tailoring rules for this RuleBasedCollator.
    ///
    /// Equivalent to `getRules()`.
    ///
    /// # Errors
    ///
    /// The errors of reading the rules resource.
    pub fn get_rules(&self) -> Result<Vec<u16>> {
        self.tailoring.get_rules()
    }

    /// Returns current rules. The argument defines whether full rules (root
    /// collation + tailored) rules are returned or just the tailoring.
    ///
    /// Equivalent to `getRules(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of reading the rules resources.
    pub fn get_rules_full(&self, fullrules: bool) -> Result<Vec<u16>> {
        if !fullrules {
            return self.tailoring.get_rules();
        }
        let mut r = CollationLoader::get_root_rules()?;
        r.extend_from_slice(&self.tailoring.get_rules()?);
        Ok(r)
    }

    /// Gets unicode sets containing contractions and/or expansions of a
    /// collator.
    ///
    /// Equivalent to `getContractionsAndExpansions(UnicodeSet, UnicodeSet,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when a set is frozen.
    pub fn get_contractions_and_expansions(
        &self,
        mut contractions: Option<&mut UnicodeSet>,
        mut expansions: Option<&mut UnicodeSet>,
        add_prefixes: bool,
    ) -> Result<()> {
        if let Some(c) = contractions.as_mut() {
            c.clear()?;
        }
        if let Some(e) = expansions.as_mut() {
            e.clear()?;
        }
        ContractionsAndExpansions::new(contractions, expansions, None, add_prefixes).for_data(&self.data)
    }

    /// Adds the contractions that start with character c to the set.
    ///
    /// Equivalent to `internalAddContractions(int, UnicodeSet)`
    /// (deprecated, package private).
    #[allow(dead_code)] // package private in Java (used by AlphabeticIndex, not ported)
    pub(crate) fn internal_add_contractions(&self, c: i32, set: &mut UnicodeSet) -> Result<()> {
        ContractionsAndExpansions::new(Some(set), None, None, false).for_code_point(&self.data, c)
    }

    fn lock_buffer(&self) -> std::sync::MutexGuard<'_, Option<Box<CollationBuffer>>> {
        let mut guard = self.collation_buffer.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(Box::new(CollationBuffer::new(&self.data)));
        }
        guard
    }

    fn get_raw_collation_key_with(&self, source: &[u16], key: Option<RawCollationKey>, buffer: &mut CollationBuffer) -> RawCollationKey {
        let mut key = match key {
            None => RawCollationKey::with_capacity(Self::simple_key_length_estimate(source)),
            Some(mut k) => {
                if k.bytes.is_none() {
                    k.bytes = Some(vec![0; Self::simple_key_length_estimate(source) as usize]);
                }
                k
            }
        };
        {
            let mut sink = CollationKeyByteSink::new(&mut key);
            self.write_sort_key(source, &mut sink, buffer);
            let appended = sink.number_of_bytes_appended();
            sink.sync_key_bytes();
            sink.key_.size = appended;
        }
        key
    }

    fn simple_key_length_estimate(source: &[u16]) -> i32 {
        2 * source.len() as i32 + 10
    }

    fn write_sort_key(&self, s: &[u16], sink: &mut CollationKeyByteSink<'_>, buffer: &mut CollationBuffer) {
        let ro = self.ro();
        let numeric = ro.is_numeric();
        let compressible = self.data.compressible_bytes.clone();
        let mut callback = SIMPLE_LEVEL_FALLBACK;
        if ro.dont_check_fcd() {
            buffer.left_utf16_coll_iter.set_text(numeric, s, 0);
            CollationKeys::write_sort_key_up_to_quaternary(
                &mut buffer.left_utf16_coll_iter,
                &compressible,
                ro,
                sink,
                Collation::PRIMARY_LEVEL,
                &mut callback,
                true,
            );
        } else {
            buffer.left_fcd_utf16_iter.set_text(numeric, s, 0);
            CollationKeys::write_sort_key_up_to_quaternary(
                &mut buffer.left_fcd_utf16_iter,
                &compressible,
                ro,
                sink,
                Collation::PRIMARY_LEVEL,
                &mut callback,
                true,
            );
        }
        if ro.get_strength() == collator::IDENTICAL {
            self.write_identical_level(s, sink);
        }
        sink.append(Collation::TERMINATOR_BYTE);
    }

    fn write_identical_level(&self, s: &[u16], sink: &mut CollationKeyByteSink<'_>) {
        // NFD quick check
        let nfd_qc_yes_limit = self.data.nfc_impl.decompose(s, 0, s.len() as i32, None);
        sink.append(Collation::LEVEL_SEPARATOR_BYTE);
        // Sync the ByteArrayWrapper size with the key length.
        sink.sync_key_bytes();
        sink.key_.size = sink.number_of_bytes_appended();
        let mut prev = 0;
        if nfd_qc_yes_limit != 0 {
            prev = BOCSU::write_identical_level_run(prev, s, 0, nfd_qc_yes_limit, &mut sink.key_.0);
        }
        // Is there non-NFD text?
        if nfd_qc_yes_limit < s.len() as i32 {
            let dest_length_estimate = s.len() as i32 - nfd_qc_yes_limit;
            let mut nfd: Vec<u16> = Vec::new();
            self.data.nfc_impl.decompose_range_into(s, nfd_qc_yes_limit, s.len() as i32, &mut nfd, dest_length_estimate);
            BOCSU::write_identical_level_run(prev, &nfd, 0, nfd.len() as i32, &mut sink.key_.0);
        }
        // Sync the key with the buffer again which got bytes appended and may have been reallocated.
        let bytes = sink.key_.bytes.clone().unwrap_or_default();
        let size = sink.key_.size;
        sink.set_buffer_and_appended(bytes, size);
    }

    /// Returns the CEs for the string.
    ///
    /// Equivalent to `internalGetCEs(CharSequence)` (deprecated, internal).
    pub fn internal_get_ces(&self, str: &[u16]) -> Vec<i64> {
        let mut guard = self.lock_buffer();
        let buffer = guard.as_mut().expect("created");
        let ro = self.ro();
        let numeric = ro.is_numeric();
        let iter: &mut dyn CollationIterator = if ro.dont_check_fcd() {
            buffer.left_utf16_coll_iter.set_text(numeric, str, 0);
            &mut buffer.left_utf16_coll_iter
        } else {
            buffer.left_fcd_utf16_iter.set_text(numeric, str, 0);
            &mut buffer.left_fcd_utf16_iter
        };
        let length = iter.fetch_ces() - 1;
        debug_assert!(length >= 0 && iter.base().get_ce(length) == Collation::NO_CE);
        iter.base().get_ces()[..length as usize].to_vec()
    }

    /// Return true if an uppercase character is sorted before the
    /// corresponding lowercase character.
    ///
    /// Equivalent to `isUpperCaseFirst()`.
    pub fn is_upper_case_first(&self) -> bool {
        self.ro().get_case_first() == CollationSettings::CASE_FIRST_AND_UPPER_MASK
    }

    /// Return true if a lowercase character is sorted before the
    /// corresponding uppercase character.
    ///
    /// Equivalent to `isLowerCaseFirst()`.
    pub fn is_lower_case_first(&self) -> bool {
        self.ro().get_case_first() == CollationSettings::CASE_FIRST
    }

    /// Checks if the alternate handling behavior is the UCA defined SHIFTED
    /// or NON_IGNORABLE.
    ///
    /// Equivalent to `isAlternateHandlingShifted()`.
    pub fn is_alternate_handling_shifted(&self) -> bool {
        self.ro().get_alternate_handling()
    }

    /// Checks if case level is set to true.
    ///
    /// Equivalent to `isCaseLevel()`.
    pub fn is_case_level(&self) -> bool {
        (self.ro().options & CollationSettings::CASE_LEVEL) != 0
    }

    /// Checks if French Collation is set to true.
    ///
    /// Equivalent to `isFrenchCollation()`.
    pub fn is_french_collation(&self) -> bool {
        (self.ro().options & CollationSettings::BACKWARD_SECONDARY) != 0
    }

    /// Checks if the Hiragana Quaternary mode is set on (deprecated).
    ///
    /// Equivalent to `isHiraganaQuaternary()`.
    pub fn is_hiragana_quaternary(&self) -> bool {
        false
    }

    /// Method to retrieve the numeric collation value.
    ///
    /// Equivalent to `getNumericCollation()`.
    pub fn get_numeric_collation(&self) -> bool {
        (self.ro().options & CollationSettings::NUMERIC) != 0
    }

    fn do_compare_impl(&self, left: &[u16], right: &[u16]) -> i32 {
        if std::ptr::eq(left, right) {
            return Collation::EQUAL;
        }

        // Identical-prefix test.
        let mut equal_prefix_length: i32 = 0;
        loop {
            if equal_prefix_length == left.len() as i32 {
                if equal_prefix_length == right.len() as i32 {
                    return Collation::EQUAL;
                }
                break;
            } else if equal_prefix_length == right.len() as i32
                || left[equal_prefix_length as usize] != right[equal_prefix_length as usize]
            {
                break;
            }
            equal_prefix_length += 1;
        }

        let ro_settings = self.ro();
        let numeric = ro_settings.is_numeric();
        if equal_prefix_length > 0
            && ((equal_prefix_length != left.len() as i32
                && self.data.is_unsafe_backward(left[equal_prefix_length as usize] as i32, numeric))
                || (equal_prefix_length != right.len() as i32
                    && self.data.is_unsafe_backward(right[equal_prefix_length as usize] as i32, numeric)))
        {
            // Identical prefix: Back up to the start of a contraction or reordering sequence.
            loop {
                equal_prefix_length -= 1;
                if !(equal_prefix_length > 0
                    && self.data.is_unsafe_backward(left[equal_prefix_length as usize] as i32, numeric))
                {
                    break;
                }
            }
        }
        // Notes:
        // - A longer string can compare equal to a prefix of it if only ignorables follow.
        // - With a backward level, a longer string can compare less-than a prefix of it.

        // Pass the actual start of each string into the CollationIterators,
        // plus the equalPrefixLength position,
        // so that prefix matches back into the equal prefix work.

        let mut result;
        let fast_latin_options = ro_settings.fast_latin_options;
        if fast_latin_options >= 0
            && (equal_prefix_length == left.len() as i32
                || left[equal_prefix_length as usize] as i32 <= CollationFastLatin::LATIN_MAX)
            && (equal_prefix_length == right.len() as i32
                || right[equal_prefix_length as usize] as i32 <= CollationFastLatin::LATIN_MAX)
        {
            let table = self.data.fast_latin_table.as_deref().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            result = CollationFastLatin::compare_utf16(
                table,
                &ro_settings.fast_latin_primaries,
                fast_latin_options,
                left,
                right,
                equal_prefix_length,
            );
        } else {
            result = CollationFastLatin::BAIL_OUT_RESULT;
        }

        if result == CollationFastLatin::BAIL_OUT_RESULT {
            let mut guard = self.lock_buffer();
            let buffer = guard.as_mut().expect("created");
            if ro_settings.dont_check_fcd() {
                buffer.left_utf16_coll_iter.set_text(numeric, left, equal_prefix_length);
                buffer.right_utf16_coll_iter.set_text(numeric, right, equal_prefix_length);
                result = CollationCompare::compare_up_to_quaternary(
                    &mut buffer.left_utf16_coll_iter,
                    &mut buffer.right_utf16_coll_iter,
                    ro_settings,
                );
            } else {
                buffer.left_fcd_utf16_iter.set_text(numeric, left, equal_prefix_length);
                buffer.right_fcd_utf16_iter.set_text(numeric, right, equal_prefix_length);
                result = CollationCompare::compare_up_to_quaternary(
                    &mut buffer.left_fcd_utf16_iter,
                    &mut buffer.right_fcd_utf16_iter,
                    ro_settings,
                );
            }
        }
        if result != Collation::EQUAL || ro_settings.get_strength() < collator::IDENTICAL {
            return result;
        }

        let mut guard = self.lock_buffer();
        let buffer = guard.as_mut().expect("created");
        // Compare identical level.
        let nfc_impl = &self.data.nfc_impl;
        if ro_settings.dont_check_fcd() {
            buffer.left_utf16_nfd_iter.set_text(left, equal_prefix_length);
            buffer.right_utf16_nfd_iter.set_text(right, equal_prefix_length);
            compare_nfd_iter(nfc_impl, &mut buffer.left_utf16_nfd_iter, &mut buffer.right_utf16_nfd_iter)
        } else {
            buffer.left_fcd_utf16_nfd_iter.set_text_fcd(nfc_impl, left, equal_prefix_length);
            buffer.right_fcd_utf16_nfd_iter.set_text_fcd(nfc_impl, right, equal_prefix_length);
            compare_nfd_iter(nfc_impl, &mut buffer.left_fcd_utf16_nfd_iter, &mut buffer.right_fcd_utf16_nfd_iter)
        }
    }

    /// Tests whether a character is "unsafe" for use as a collation
    /// starting point.
    ///
    /// Equivalent to `isUnsafe(int)` (package private).
    pub(crate) fn is_unsafe(&self, c: i32) -> bool {
        self.data.is_unsafe_backward(c, self.ro().is_numeric())
    }

    /// `equals(Object)` against another rule-based collator.
    fn equals_rbc(&self, o: &RuleBasedCollator) -> Result<bool> {
        if std::ptr::eq(self, o) {
            return Ok(true);
        }
        if self.ro() != o.ro() {
            return Ok(false);
        }
        if Arc::ptr_eq(&self.data, &o.data) {
            return Ok(true);
        }
        let this_is_root = self.data.base.is_none();
        let other_is_root = o.data.base.is_none();
        debug_assert!(!this_is_root || !other_is_root); // otherwise their data pointers should be ==
        if this_is_root != other_is_root {
            return Ok(false);
        }
        let these_rules = self.tailoring.get_rules()?;
        let other_rules = o.tailoring.get_rules()?;
        if (this_is_root || !these_rules.is_empty())
            && (other_is_root || !other_rules.is_empty())
            // Shortcut: If both collators have valid rule strings, then compare those.
            && these_rules == other_rules
        {
            return Ok(true);
        }
        // Different rule strings can result in the same or equivalent tailoring.
        // The rule strings are optional in ICU resource bundles, although included by default.
        // cloneBinary() drops the rule string.
        let this_tailored = self.get_tailored_set()?;
        let other_tailored = o.get_tailored_set()?;
        if this_tailored != other_tailored {
            return Ok(false);
        }
        // For completeness, we should compare all of the mappings;
        // or we should create a list of strings, sort it with one collator,
        // and check if both collators compare adjacent strings the same
        // (order & strength, down to quaternary); or similar.
        // Testing equality of collators seems unusual.
        Ok(true)
    }

    /// `equals(Object)`, with the exceptions of reading the rules.
    ///
    /// # Errors
    ///
    /// The errors of reading the rules resources.
    pub fn try_equals(&self, obj: &dyn Collator) -> Result<bool> {
        if std::ptr::addr_eq(self as *const Self, obj as *const dyn Collator) {
            return Ok(true);
        }
        match obj.as_rule_based() {
            Some(o) => self.equals_rbc(o),
            None => Ok(false),
        }
    }

    /// `hashCode()`, with the exceptions of building the tailored set.
    ///
    /// # Errors
    ///
    /// The errors of building the tailored set.
    pub fn try_hash_code(&self) -> Result<i32> {
        let mut h = self.ro().hash_code();
        if self.data.base.is_none() {
            return Ok(h);
        } // root collator
        // Do not rely on the rule string, see comments in operator==().
        let set = self.get_tailored_set()?;
        let mut iter = UnicodeSetIterator::new(&set);
        while iter.next() && iter.codepoint != IS_STRING {
            h ^= self.data.get_ce32(iter.codepoint);
        }
        Ok(h)
    }

    /// `getLocale(ULocale.Type)` for `RuleBasedCollator`.
    fn get_locale_impl(&self, ty: Type) -> Option<ULocale> {
        match ty {
            Type::ActualLocale => {
                if self.actual_locale_is_same_as_valid {
                    self.valid_locale.clone()
                } else {
                    self.tailoring.actual_locale.clone()
                }
            }
            Type::ValidLocale => self.valid_locale.clone(),
        }
    }
}

impl Collator for RuleBasedCollator {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_rule_based(&self) -> Option<&RuleBasedCollator> {
        Some(self)
    }

    fn as_rule_based_mut(&mut self) -> Option<&mut RuleBasedCollator> {
        Some(self)
    }

    fn clone_box(&self) -> Result<Box<dyn Collator>> {
        if self.is_frozen() {
            let mut c = self.clone_as_thawed_rbc();
            c.frozen = true;
            return Ok(Box::new(c));
        }
        Ok(Box::new(self.clone_as_thawed_rbc()))
    }

    /// `equals(Object)`; the unchecked exception of reading the rules
    /// propagates as a panic (see [`RuleBasedCollator::try_equals`]).
    fn equals(&self, obj: &dyn Collator) -> bool {
        self.try_equals(obj).unwrap_or_else(|e| panic!("{e}"))
    }

    /// `hashCode()`; see [`RuleBasedCollator::try_hash_code`].
    fn hash_code(&self) -> i32 {
        self.try_hash_code().unwrap_or_else(|e| panic!("{e}"))
    }

    fn set_strength(&mut self, new_strength: i32) -> Result<()> {
        self.check_not_frozen()?;
        if new_strength == self.get_strength() {
            return Ok(());
        }
        self.get_owned_settings().set_strength(new_strength)?;
        self.set_fast_latin_options();
        Ok(())
    }

    fn set_decomposition(&mut self, decomposition: i32) -> Result<()> {
        self.check_not_frozen()?;
        let flag = match decomposition {
            collator::NO_DECOMPOSITION => false,
            collator::CANONICAL_DECOMPOSITION => true,
            _ => return Err(LuceneError::illegal_argument("Wrong decomposition mode.")),
        };
        if flag == self.ro().get_flag(CollationSettings::CHECK_FCD) {
            return Ok(());
        }
        self.get_owned_settings().set_flag(CollationSettings::CHECK_FCD, flag);
        self.set_fast_latin_options();
        Ok(())
    }

    fn set_reorder_codes(&mut self, order: Option<&[i32]>) -> Result<()> {
        self.check_not_frozen()?;
        let mut length = order.map_or(0, |o| o.len());
        if length == 1 && order.expect("length 1")[0] == ReorderCodes::NONE {
            length = 0;
        }
        if if length == 0 {
            self.ro().reorder_codes.is_empty()
        } else {
            order.expect("non-empty")[..] == self.ro().reorder_codes[..]
        } {
            return Ok(());
        }
        if length == 1 && order.expect("length 1")[0] == ReorderCodes::DEFAULT {
            if !self.settings_is_default() {
                let default_settings = self.get_default_settings().clone();
                self.get_owned_settings().copy_reordering_from(&default_settings);
                self.set_fast_latin_options();
            }
            return Ok(());
        }
        let data = self.data.clone();
        let owned_settings = self.get_owned_settings();
        if length == 0 {
            owned_settings.reset_reordering();
        } else {
            owned_settings.set_reordering(&data, order.expect("non-empty"))?;
        }
        self.set_fast_latin_options();
        Ok(())
    }

    fn get_strength(&self) -> i32 {
        self.ro().get_strength()
    }

    fn get_decomposition(&self) -> i32 {
        if (self.ro().options & CollationSettings::CHECK_FCD) != 0 {
            collator::CANONICAL_DECOMPOSITION
        } else {
            collator::NO_DECOMPOSITION
        }
    }

    fn get_tailored_set(&self) -> Result<UnicodeSet> {
        let mut tailored = UnicodeSet::new();
        if self.data.base.is_some() {
            TailoredSet::new(&mut tailored).for_data(&self.data)?;
        }
        Ok(tailored)
    }

    fn compare(&self, source: &[u16], target: &[u16]) -> i32 {
        self.do_compare(source, target)
    }

    fn do_compare(&self, left: &[u16], right: &[u16]) -> i32 {
        self.do_compare_impl(left, right)
    }

    fn get_collation_key(&self, source: Option<&[u16]>) -> Option<CollationKey> {
        let source = source?;
        let mut guard = self.lock_buffer();
        let buffer = guard.as_mut().expect("created");
        let raw = buffer.raw_collation_key.take();
        let mut raw = self.get_raw_collation_key_with(source, raw, buffer);
        let key = CollationKey::from_raw(Some(source.to_vec()), &mut raw);
        buffer.raw_collation_key = Some(raw);
        Some(key)
    }

    fn get_raw_collation_key(&self, source: Option<&[u16]>, key: Option<RawCollationKey>) -> Option<RawCollationKey> {
        let source = source?;
        let mut guard = self.lock_buffer();
        let buffer = guard.as_mut().expect("created");
        Some(self.get_raw_collation_key_with(source, key, buffer))
    }

    fn set_max_variable(&mut self, mut group: i32) -> Result<()> {
        // Convert the reorder code into a MaxVariable number, or UCOL_DEFAULT=-1.
        let value = if group == ReorderCodes::DEFAULT {
            -1 // UCOL_DEFAULT
        } else if (ReorderCodes::FIRST..=ReorderCodes::CURRENCY).contains(&group) {
            group - ReorderCodes::FIRST
        } else {
            return Err(LuceneError::illegal_argument(format!("illegal max variable group {group}")));
        };
        let old_value = self.ro().get_max_variable();
        if value == old_value {
            return Ok(());
        }
        let default_settings = self.get_default_settings().clone();
        if self.settings_is_default() && value < 0 {
            // UCOL_DEFAULT
            return Ok(());
        }
        if group == ReorderCodes::DEFAULT {
            group = ReorderCodes::FIRST + default_settings.get_max_variable();
        }
        let var_top = self.data.get_last_primary_for_group(group);
        debug_assert!(var_top != 0);
        let owned_settings = self.get_owned_settings();
        owned_settings.set_max_variable(value, default_settings.options)?;
        owned_settings.variable_top = var_top;
        self.set_fast_latin_options();
        Ok(())
    }

    fn get_max_variable(&self) -> i32 {
        ReorderCodes::FIRST + self.ro().get_max_variable()
    }

    fn set_variable_top_str(&mut self, var_top: Option<&[u16]>) -> Result<i32> {
        self.check_not_frozen()?;
        let var_top = match var_top {
            Some(v) if !v.is_empty() => v,
            _ => {
                return Err(LuceneError::illegal_argument(
                    "Variable top argument string can not be null or zero in length.",
                ))
            }
        };
        let numeric = self.ro().is_numeric();
        let ce1;
        let ce2;
        if self.ro().dont_check_fcd() {
            let mut ci = UTF16CollationIterator::with_text(self.data.clone(), numeric, var_top, 0);
            ce1 = ci.next_ce();
            ce2 = ci.next_ce();
        } else {
            let mut ci = FCDUTF16CollationIterator::with_text(self.data.clone(), numeric, var_top, 0);
            ce1 = ci.next_ce();
            ce2 = ci.next_ce();
        }
        if ce1 == Collation::NO_CE || ce2 != Collation::NO_CE {
            return Err(LuceneError::illegal_argument(
                "Variable top argument string must map to exactly one collation element",
            ));
        }
        self.internal_set_variable_top(((ce1 as u64) >> 32) as i64)?;
        Ok(self.ro().variable_top as i32)
    }

    fn get_variable_top(&self) -> i32 {
        self.ro().variable_top as i32
    }

    fn set_variable_top(&mut self, var_top: i32) -> Result<()> {
        self.check_not_frozen()?;
        self.internal_set_variable_top((var_top as i64) & 0xffffffff)
    }

    fn get_version(&self) -> Result<VersionInfo> {
        let version = self.tailoring.version;
        let rt_version = VersionInfo::UCOL_RUNTIME_VERSION.get_major();
        VersionInfo::get_instance4(
            ((version as u32) >> 24) as i32 + (rt_version << 4) + (rt_version >> 4),
            (version >> 16) & 0xff,
            (version >> 8) & 0xff,
            version & 0xff,
        )
    }

    fn get_uca_version(&self) -> Result<VersionInfo> {
        let v = self.get_version()?;
        // Note: This is tied to how the current implementation encodes the UCA version
        // in the overall getVersion().
        // Alternatively, we could load the root collator and get at lower-level data from there.
        // Either way, it will reflect the input collator's UCA version only
        // if it is a known implementation.
        // (C++ comment) It would be cleaner to make this a virtual Collator method.
        // (In Java, it is virtual.)
        VersionInfo::get_instance4(v.get_minor() >> 3, v.get_minor() & 7, v.get_milli() >> 6, 0)
    }

    fn get_reorder_codes(&self) -> Result<Vec<i32>> {
        Ok(self.ro().reorder_codes.to_vec())
    }

    fn is_frozen(&self) -> bool {
        self.frozen
    }

    fn freeze(&mut self) -> Result<()> {
        if !self.is_frozen() {
            self.frozen = true;
            let mut guard = self.collation_buffer.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_none() {
                *guard = Some(Box::new(CollationBuffer::new(&self.data)));
            }
        }
        Ok(())
    }

    fn clone_as_thawed(&self) -> Result<Box<dyn Collator>> {
        Ok(Box::new(self.clone_as_thawed_rbc()))
    }

    fn get_locale(&self, ty: Type) -> Result<Option<ULocale>> {
        Ok(self.get_locale_impl(ty))
    }

    fn set_locale(&mut self, valid: Option<ULocale>, actual: Option<ULocale>) {
        // This method is called
        // by other protected functions that checks and makes sure that
        // valid and actual are not null before passing
        debug_assert!(valid.is_none() == actual.is_none());
        // Another check we could do is that the actual locale is at
        // the same level or less specific than the valid locale.
        if actual == self.tailoring.actual_locale {
            self.actual_locale_is_same_as_valid = false;
        } else {
            debug_assert!(actual == valid);
            self.actual_locale_is_same_as_valid = true;
        }
        // Do not modify tailoring.actualLocale:
        // We cannot be sure that that would be thread-safe.
        self.valid_locale = valid;
    }
}
