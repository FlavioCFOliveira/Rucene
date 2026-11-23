// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Transliterator`.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, RwLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{index_of, runtime_exception, to_rust_string, units};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::utility::Utility;
use crate::text::any_transliterator::AnyTransliterator;
use crate::text::break_transliterator::BreakTransliterator;
use crate::text::case_fold_transliterator::CaseFoldTransliterator;
use crate::text::compound_transliterator::CompoundTransliterator;
use crate::text::escape_transliterator::EscapeTransliterator;
use crate::text::lowercase_transliterator::LowercaseTransliterator;
use crate::text::name_unicode_transliterator::NameUnicodeTransliterator;
use crate::text::normalization_transliterator::NormalizationTransliterator;
use crate::text::null_transliterator::NullTransliterator;
use crate::text::remove_transliterator::RemoveTransliterator;
use crate::text::replaceable::Replaceable;
use crate::text::replaceable_string::ReplaceableString;
use crate::text::rule_based_transliterator::RuleBasedTransliterator;
use crate::text::titlecase_transliterator::TitlecaseTransliterator;
use crate::text::transform::Transform;
use crate::text::transliterator_id_parser::TransliteratorIDParser;
use crate::text::transliterator_parser::TransliteratorParser;
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unescape_transliterator::UnescapeTransliterator;
use crate::text::unicode_filter::UnicodeFilter;
use crate::text::unicode_name_transliterator::UnicodeNameTransliterator;
use crate::text::unicode_set::{self, UnicodeSet};
use crate::text::uppercase_transliterator::UppercaseTransliterator;
use crate::text::utf16::UTF16;
use crate::util::case_insensitive_string::CaseInsensitiveString;
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::{is_missing_resource, UResourceBundle};

/// `FORWARD`: direction constant indicating the forward direction in a
/// transliterator, e.g., the forward rules of a rule-based transliterator.
pub const FORWARD: i32 = 0;

/// `REVERSE`: direction constant indicating the reverse direction in a
/// transliterator, e.g., the reverse rules of a rule-based transliterator.
pub const REVERSE: i32 = 1;

/// `ID_DELIM`: delimiter between elements in a compound ID.
pub(crate) const ID_DELIM: u16 = b';' as u16;

/// `ID_SEP`: delimiter before target in an ID.
pub(crate) const ID_SEP: u16 = b'-' as u16;

/// `VARIANT_SEP`: delimiter before variant in an ID.
pub(crate) const VARIANT_SEP: u16 = b'/' as u16;

/// `RB_DISPLAY_NAME_PREFIX`: prefix for resource bundle key for the display
/// name for a transliterator.
const RB_DISPLAY_NAME_PREFIX: &str = "%Translit%%";

/// `RB_SCRIPT_DISPLAY_NAME_PREFIX`: prefix for resource bundle key for the
/// display name for a transliterator script.
const RB_SCRIPT_DISPLAY_NAME_PREFIX: &str = "%Translit%";

/// `RB_DISPLAY_NAME_PATTERN`: resource bundle key for display name pattern.
const RB_DISPLAY_NAME_PATTERN: &str = "TransliteratorNamePattern";

/// `ROOT`.
const ROOT: &str = "root";

/// `RB_RULE_BASED_IDS`.
const RB_RULE_BASED_IDS: &str = "RuleBasedTransliteratorIDs";

/// Position structure for incremental transliteration. This data structure
/// defines two substrings of the text being transliterated. The first
/// region, `[context_start, context_limit)`, defines what characters the
/// transliterator will read as context. The second region, `[start,
/// limit)`, defines what characters will actually be transliterated. The
/// second region should be a subset of the first.
///
/// Equivalent to the nested class `Transliterator.Position`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Position {
    /// Beginning index, inclusive, of the context to be considered for a
    /// transliteration operation.
    pub context_start: i32,
    /// Ending index, exclusive, of the context to be considered for a
    /// transliteration operation.
    pub context_limit: i32,
    /// Beginning index, inclusive, of the text to be transliterated.
    pub start: i32,
    /// Ending index, exclusive, of the text to be transliterated.
    pub limit: i32,
}

impl Position {
    /// `Position()`: all indices zero.
    pub fn new() -> Self {
        Self::with_all(0, 0, 0, 0)
    }

    /// `Position(int contextStart, int contextLimit, int start)`: `limit`
    /// is `context_limit`.
    pub fn with_context_and_start(context_start: i32, context_limit: i32, start: i32) -> Self {
        Self::with_all(context_start, context_limit, start, context_limit)
    }

    /// `Position(int contextStart, int contextLimit, int start, int
    /// limit)`.
    pub fn with_all(context_start: i32, context_limit: i32, start: i32, limit: i32) -> Self {
        Self {
            context_start,
            context_limit,
            start,
            limit,
        }
    }

    /// `Position(Position pos)`: a copy.
    pub fn from_position(pos: &Position) -> Self {
        *pos
    }

    /// `set(Position pos)`: copies the given position into this one.
    pub fn set(&mut self, pos: &Position) {
        self.context_start = pos.context_start;
        self.context_limit = pos.context_limit;
        self.start = pos.start;
        self.limit = pos.limit;
    }

    /// `hashCode()`: `Objects.hash(contextStart, contextLimit, start,
    /// limit)`.
    pub fn hash_code(&self) -> i32 {
        let mut result = 1i32;
        for v in [self.context_start, self.context_limit, self.start, self.limit] {
            result = result.wrapping_mul(31).wrapping_add(v);
        }
        result
    }

    /// `validate(int length)`: checks that this position is consistent and
    /// within the text of the given length.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if any indices are out of range.
    pub fn validate(&self, length: i32) -> Result<()> {
        if self.context_start < 0
            || self.start < self.context_start
            || self.limit < self.start
            || self.context_limit < self.limit
            || length < self.context_limit
        {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid Position {{cs={}, s={}, l={}, cl={}}}, len={}",
                self.context_start, self.start, self.limit, self.context_limit, length
            )));
        }
        Ok(())
    }
}

impl fmt::Display for Position {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[cs={}, s={}, l={}, cl={}]", self.context_start, self.start, self.limit, self.context_limit)
    }
}

/// The fields every transliterator has: its ID, its filter and its maximum
/// context length.
///
/// Equivalent to the instance fields of the abstract class `Transliterator`
/// (`ID`, `filter`, `maximumContextLength`) and the accessors that read and
/// write them.
///
/// **Divergence.** Java's fields (`Transliterator.java:673-683`) are plain
/// fields of an object shared by reference (the registry hands back the instance it holds, and
/// `getInstance` then calls `setID`/`setFilter` on it); here the object is
/// shared through an `Arc`, so the fields sit behind locks and the setters
/// take `&self`.
#[derive(Debug)]
pub struct TransliteratorBase {
    /// Programmatic name, e.g., "Latin-Arabic".
    id: RwLock<Vec<u16>>,
    /// This transliterator's filter. Any character for which
    /// `filter.contains()` returns false will not be altered by this
    /// transliterator. If filter is null then no filtering is applied.
    filter: RwLock<Option<Arc<UnicodeSet>>>,
    maximum_context_length: AtomicI32,
}

impl TransliteratorBase {
    /// `Transliterator(String ID, UnicodeFilter filter)`: the fields of a
    /// transliterator with the given ID and filter (Java's `null` ID check
    /// is the non-optional parameter).
    ///
    /// # Errors
    ///
    /// The errors of [`set_filter`](Self::set_filter).
    pub fn new(id: Vec<u16>, filter: Option<&dyn UnicodeFilter>) -> Result<Self> {
        let base = Self::with_id(id);
        base.set_filter(filter)?;
        Ok(base)
    }

    /// `Transliterator(String ID, null)`.
    pub fn with_id(id: Vec<u16>) -> Self {
        Self {
            id: RwLock::new(id),
            filter: RwLock::new(None),
            maximum_context_length: AtomicI32::new(0),
        }
    }

    /// `getID()`.
    pub fn get_id(&self) -> Vec<u16> {
        self.id.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// `setID(String id)`.
    pub fn set_id(&self, id: Vec<u16>) {
        *self.id.write().unwrap_or_else(|e| e.into_inner()) = id;
    }

    /// `getFilter()`: the filter, always a frozen `UnicodeSet`.
    pub fn get_filter(&self) -> Option<Arc<UnicodeSet>> {
        self.filter.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// `Transliterator.setFilter(UnicodeFilter filter)`: stores a frozen copy
    /// of the filter (a `UnicodeSet` copy, or the match set of another
    /// filter).
    ///
    /// # Errors
    ///
    /// The errors of `addMatchSetTo` of a filter that is not a `UnicodeSet`.
    pub fn set_filter(&self, filter: Option<&dyn UnicodeFilter>) -> Result<()> {
        let new = match filter {
            None => None,
            Some(filter) => match filter.as_any().downcast_ref::<UnicodeSet>() {
                Some(set) => {
                    let mut copy = UnicodeSet::from_set(set);
                    copy.freeze();
                    Some(Arc::new(copy))
                }
                None => {
                    // catch (Exception e): not a UnicodeSet
                    let mut copy = UnicodeSet::new();
                    filter.add_match_set_to(&mut copy)?;
                    copy.freeze();
                    Some(Arc::new(copy))
                }
            },
        };
        *self.filter.write().unwrap_or_else(|e| e.into_inner()) = new;
        Ok(())
    }

    /// `getMaximumContextLength()`.
    pub fn get_maximum_context_length(&self) -> i32 {
        self.maximum_context_length.load(Ordering::Relaxed)
    }

    /// `setMaximumContextLength(int a)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a negative length.
    pub fn set_maximum_context_length(&self, a: i32) -> Result<()> {
        if a < 0 {
            return Err(LuceneError::illegal_argument(format!("Invalid context length {a}")));
        }
        self.maximum_context_length.store(a, Ordering::Relaxed);
        Ok(())
    }
}

/// Implements the [`Transliterator`] accessors of a type whose
/// [`TransliteratorBase`] is the field `base`.
macro_rules! transliterator_boilerplate {
    () => {
        fn base(&self) -> &$crate::text::transliterator::TransliteratorBase {
            &self.base
        }

        fn as_dyn(&self) -> &dyn $crate::text::transliterator::Transliterator {
            self
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    };
}
pub(crate) use transliterator_boilerplate;

/// `Transliterator` is an abstract class that transliterates text from one
/// format to another. The most common kind of transliterator is a script,
/// or alphabet, transliterator. For example, a Russian to Latin
/// transliterator changes Russian text written in Cyrillic characters to
/// phonetically equivalent Latin characters.
///
/// Equivalent to the abstract class `com.ibm.icu.text.Transliterator`.
/// Its abstract method `handleTransliterate` and its overridable methods
/// are the methods of this trait; its final methods are inherent methods of
/// `dyn Transliterator`; its static methods are the free functions of this
/// module. Instances are shared as `Arc<dyn Transliterator>`, as Java
/// shares them by reference.
pub trait Transliterator: fmt::Debug + Send + Sync + Any {
    /// The fields of the transliterator.
    fn base(&self) -> &TransliteratorBase;

    /// The transliterator as a trait object.
    fn as_dyn(&self) -> &dyn Transliterator;

    /// The transliterator as `Any`, for Java's `instanceof` tests and casts.
    fn as_any(&self) -> &dyn Any;

    /// `handleTransliterate(Replaceable text, Position pos, boolean
    /// incremental)`: abstract method that concrete subclasses define to
    /// implement their transliteration algorithm. This method handles both
    /// incremental and non-incremental transliteration. Let `originalStart`
    /// refer to the value of `pos.start` upon entry.
    ///
    /// * If `incremental` is false, then this method should transliterate
    ///   all characters between `pos.start` and `pos.limit`. Upon return
    ///   `pos.start` must == `pos.limit`.
    /// * If `incremental` is true, then this method should transliterate
    ///   all characters between `pos.start` and `pos.limit` that can be
    ///   unambiguously transliterated, regardless of future insertions of
    ///   text at `pos.limit`. Upon return, `pos.start` should be in the
    ///   range [`originalStart`, `pos.limit`).
    ///
    /// # Errors
    ///
    /// The errors of the transliteration.
    fn handle_transliterate(&self, text: &dyn Replaceable, pos: &mut Position, incremental: bool) -> Result<()>;

    /// `filteredTransliterate(Replaceable text, Position index, boolean
    /// incremental)`: transliterate a substring of text, as specified by
    /// index, taking filters into account. This method is for subclasses
    /// that need to delegate to another transliterator, such as
    /// `CompoundTransliterator`.
    ///
    /// # Errors
    ///
    /// The errors of the transliteration.
    fn filtered_transliterate(&self, text: &dyn Replaceable, index: &mut Position, incremental: bool) -> Result<()> {
        self.as_dyn().filtered_transliterate_full(text, index, incremental, false)
    }

    /// `setFilter(UnicodeFilter filter)`: changes the filter used by this
    /// transliterator. If the filter is set to `None` then no filtering
    /// will occur.
    ///
    /// # Errors
    ///
    /// The errors of [`TransliteratorBase::set_filter`].
    fn set_filter(&self, filter: Option<&dyn UnicodeFilter>) -> Result<()> {
        self.base().set_filter(filter)
    }

    /// `toRules(boolean escapeUnprintable)`: a rule string for this
    /// transliterator.
    ///
    /// # Errors
    ///
    /// The errors of the rule rendering.
    fn to_rules(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        self.as_dyn().base_to_rules(escape_unprintable)
    }

    /// `handleGetSourceSet()`: framework method that returns the set of all
    /// characters that may be modified in the input text by this
    /// transliterator, ignoring the effect of this object's filter.
    fn handle_get_source_set(&self) -> UnicodeSet {
        UnicodeSet::new()
    }

    /// `getTargetSet()`: the set of all characters that may be generated as
    /// replacement text by this transliterator.
    ///
    /// # Errors
    ///
    /// The errors of [`add_source_target_set`](Self::add_source_target_set).
    fn get_target_set(&self) -> Result<UnicodeSet> {
        let mut result = UnicodeSet::new();
        let filter = self.get_filter_as_unicode_set(&unicode_set::ALL_CODE_POINTS)?;
        self.add_source_target_set(&filter, &mut UnicodeSet::new(), &mut result)?;
        Ok(result)
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`: unions into `source_set` the characters this
    /// transliterator may modify under `input_filter`, and into
    /// `target_set` those it may produce. The default transliterates every
    /// item of [`handle_get_source_set`](Self::handle_get_source_set).
    ///
    /// # Errors
    ///
    /// The errors of the transliteration and of the `UnicodeSet`
    /// operations.
    fn add_source_target_set(&self, input_filter: &UnicodeSet, source_set: &mut UnicodeSet, target_set: &mut UnicodeSet) -> Result<()> {
        let my_filter = self.get_filter_as_unicode_set(input_filter)?;
        let mut temp = UnicodeSet::from_set(&self.handle_get_source_set());
        temp.retain_all(&my_filter)?;
        // use old method, if we don't have anything better
        source_set.add_all(&temp)?;
        // TODO
        for s in temp.iter() {
            let t = self.as_dyn().transliterate(&s)?;
            if s != t {
                target_set.add_all_str(&t)?;
            }
        }
        Ok(())
    }

    /// `getFilterAsUnicodeSet(UnicodeSet externalFilter)`: the intersection
    /// of this instance's filter intersected with an external filter.
    ///
    /// **Divergence.** The result is a copy: Java returns `externalFilter`
    /// itself when there is no filter (`Transliterator.java:1915-1917`); no
    /// caller modifies the result, so the copy is observably the same.
    ///
    /// # Errors
    ///
    /// The errors of the `UnicodeSet` operations.
    fn get_filter_as_unicode_set(&self, external_filter: &UnicodeSet) -> Result<UnicodeSet> {
        let Some(filter) = self.base().get_filter() else {
            return Ok(external_filter.clone());
        };
        let mut filter_set = UnicodeSet::from_set(external_filter);
        // Most, but not all filters will be UnicodeSets.  Optimize for
        // the high-runner case. (The stored filter is always a UnicodeSet.)
        filter_set.retain_all(&filter)?;
        filter_set.freeze();
        Ok(filter_set)
    }
}

impl dyn Transliterator {
    /// `transliterate(Replaceable text, int start, int limit)`:
    /// transliterates a segment of a string, with optional filtering;
    /// returns the new limit index, or -1 if `start` or `limit` are out of
    /// bounds.
    ///
    /// # Errors
    ///
    /// The errors of the transliteration.
    pub fn transliterate_range(&self, text: &dyn Replaceable, start: i32, limit: i32) -> Result<i32> {
        if start < 0 || limit < start || text.length() < limit {
            return Ok(-1);
        }

        let mut pos = Position::with_context_and_start(start, limit, start);
        self.filtered_transliterate_full(text, &mut pos, false, true)?;
        Ok(pos.limit)
    }

    /// `transliterate(Replaceable text)`: transliterates an entire string
    /// in place.
    ///
    /// # Errors
    ///
    /// The errors of the transliteration.
    pub fn transliterate_replaceable(&self, text: &dyn Replaceable) -> Result<()> {
        self.transliterate_range(text, 0, text.length())?;
        Ok(())
    }

    /// `transliterate(String text)`: transliterates an entire string and
    /// returns the result.
    ///
    /// # Errors
    ///
    /// The errors of the transliteration.
    pub fn transliterate(&self, text: &[u16]) -> Result<Vec<u16>> {
        let result = ReplaceableString::from_str_units(text);
        self.transliterate_replaceable(&result)?;
        Ok(result.to_units())
    }

    /// `transliterate(Replaceable text, Position index, String insertion)`:
    /// transliterates the portion of the text buffer that can be
    /// transliterated unambiguously after new text has been inserted,
    /// typically as a result of a keyboard event.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if `index` is invalid, and the errors of
    /// the transliteration.
    pub fn transliterate_position_str(&self, text: &dyn Replaceable, index: &mut Position, insertion: Option<&[u16]>) -> Result<()> {
        index.validate(text.length())?;

        if let Some(insertion) = insertion {
            text.replace(index.limit, index.limit, insertion);
            index.limit += insertion.len() as i32;
            index.context_limit += insertion.len() as i32;
        }

        if index.limit > 0 && UTF16::is_lead_surrogate(text.char_at(index.limit - 1) as i32) {
            // Oops, there is a dangling lead surrogate in the buffer.
            // This will break most transliterators, since they will
            // assume it is part of a pair.  Don't transliterate until
            // more text comes in.
            return Ok(());
        }

        self.filtered_transliterate_full(text, index, true, true)
    }

    /// `transliterate(Replaceable text, Position index, int insertion)`:
    /// as [`transliterate_position_str`](Self::transliterate_position_str)
    /// with a single code point.
    ///
    /// # Errors
    ///
    /// As [`transliterate_position_str`](Self::transliterate_position_str),
    /// and `IllegalArgumentException` for an invalid code point.
    pub fn transliterate_position_cp(&self, text: &dyn Replaceable, index: &mut Position, insertion: i32) -> Result<()> {
        let insertion = UTF16::value_of(insertion)?;
        self.transliterate_position_str(text, index, Some(&insertion))
    }

    /// `transliterate(Replaceable text, Position index)`: transliterates
    /// the portion of the text buffer that can be transliterated
    /// unambiguously, without inserting text.
    ///
    /// # Errors
    ///
    /// As [`transliterate_position_str`](Self::transliterate_position_str).
    pub fn transliterate_position(&self, text: &dyn Replaceable, index: &mut Position) -> Result<()> {
        self.transliterate_position_str(text, index, None)
    }

    /// `finishTransliteration(Replaceable text, Position index)`: finishes
    /// any pending transliterations that were waiting for more characters.
    ///
    /// # Errors
    ///
    /// As [`transliterate_position_str`](Self::transliterate_position_str).
    pub fn finish_transliteration(&self, text: &dyn Replaceable, index: &mut Position) -> Result<()> {
        index.validate(text.length())?;
        self.filtered_transliterate_full(text, index, false, true)
    }

    /// The private `filteredTransliterate(Replaceable text, Position index,
    /// boolean incremental, boolean rollback)`: top-level transliteration
    /// method, handling filtering, incremental and non-incremental
    /// transliteration, and rollback. All transliteration public API
    /// methods eventually call this method with a rollback argument of
    /// true. Other entities may call this method but rollback should be
    /// false.
    ///
    /// The `if (DEBUG)` logging of Java is not ported: `DEBUG` is a
    /// `static final false` constant, so javac compiles it out.
    fn filtered_transliterate_full(&self, text: &dyn Replaceable, index: &mut Position, incremental: bool, rollback: bool) -> Result<()> {
        let filter = self.base().get_filter();

        // Short circuit path for transliterators with no filter in
        // non-incremental mode.
        if filter.is_none() && !rollback {
            return self.handle_transliterate(text, index, incremental);
        }

        //----------------------------------------------------------------------
        // This method processes text in two groupings:
        //
        // RUNS -- A run is a contiguous group of characters which are contained
        // in the filter for this transliterator (filter.contains(ch) == true).
        // Text outside of runs may appear as context but it is not modified.
        // The start and limit Position values are narrowed to each run.
        //
        // PASSES (incremental only) -- To make incremental mode work correctly,
        // each run is broken up into n passes, where n is the length (in code
        // points) of the run.  Each pass contains the first n characters.  If a
        // pass is completely transliterated, it is committed, and further passes
        // include characters after the committed text.  If a pass is blocked,
        // and does not transliterate completely, then this method rolls back
        // the changes made during the pass, extends the pass by one code point,
        // and tries again.
        //----------------------------------------------------------------------

        // globalLimit is the limit value for the entire operation.  We
        // set index.limit to the end of each unfiltered run before
        // calling handleTransliterate(), so we need to maintain the real
        // value of index.limit here.  After each transliteration, we
        // update globalLimit for insertions or deletions that have
        // happened.
        let mut global_limit = index.limit;

        loop {
            if let Some(filter) = &filter {
                // Narrow the range to be transliterated to the first run
                // of unfiltered characters at or after index.start.

                // Advance past filtered chars
                let mut c;
                while index.start < global_limit && {
                    c = text.char32_at(index.start);
                    !filter.contains_valid(c)
                } {
                    index.start += UTF16::get_char_count(c);
                }

                // Find the end of this run of unfiltered chars
                index.limit = index.start;
                while index.limit < global_limit && {
                    c = text.char32_at(index.limit);
                    filter.contains_valid(c)
                } {
                    index.limit += UTF16::get_char_count(c);
                }
            }

            // Check to see if the unfiltered run is empty.  This only
            // happens at the end of the string when all the remaining
            // characters are filtered.
            if index.start == index.limit {
                break;
            }

            // Is this run incremental?  If there is additional
            // filtered text (if limit < globalLimit) then we pass in
            // an incremental value of false to force the subclass to
            // complete the transliteration for this run.
            let is_incremental_run = if index.limit < global_limit { false } else { incremental };

            let delta;

            // Implement rollback.  To understand the need for rollback,
            // consider the following transliterator:
            //
            //  "t" is "a > A;"
            //  "u" is "A > b;"
            //  "v" is a compound of "t; NFD; u" with a filter [:Ll:]
            //
            // Now apply "v" to the input text "a".  The result is "b".  But if
            // the transliteration is done incrementally, then the NFD holds
            // things up after "t" has already transformed "a" to "A".  When
            // finishTransliterate() is called, "A" is _not_ processed because
            // it gets excluded by the [:Ll:] filter, and the end result is "A"
            // -- incorrect.  The problem is that the filter is applied to a
            // partially-transliterated result, when we only want it to apply to
            // input text.  Although this example describes a compound
            // transliterator containing NFD and a specific filter, it can
            // happen with any transliterator which does a partial
            // transformation in incremental mode into characters outside its
            // filter.
            //
            // To handle this, when in incremental mode we supply characters to
            // handleTransliterate() in several passes.  Each pass adds one more
            // input character to the input text.  That is, for input "ABCD", we
            // first try "A", then "AB", then "ABC", and finally "ABCD".  If at
            // any point we block (upon return, start < limit) then we roll
            // back.  If at any point we complete the run (upon return start ==
            // limit) then we commit that run.

            if rollback && is_incremental_run {
                let run_start = index.start;
                let mut run_limit = index.limit;
                let run_length = run_limit - run_start;

                // Make a rollback copy at the end of the string
                let mut rollback_origin = text.length();
                text.copy(run_start, run_limit, rollback_origin);

                // Variables reflecting the commitment of completely
                // transliterated text.  passStart is the runStart, advanced
                // past committed text.  rollbackStart is the rollbackOrigin,
                // advanced past rollback text that corresponds to committed
                // text.
                let mut pass_start = run_start;
                let mut rollback_start = rollback_origin;

                // The limit for each pass; we advance by one code point with
                // each iteration.
                let mut pass_limit = index.start;

                // Total length, in 16-bit code units, of uncommitted text.
                // This is the length to be rolled back.
                let mut uncommitted_length = 0;

                // Total delta (change in length) for all passes
                let mut total_delta = 0;

                // PASS MAIN LOOP -- Start with a single character, and extend
                // the text by one character at a time.  Roll back partial
                // transliterations and commit complete transliterations.
                loop {
                    // Length of additional code point, either one or two
                    let char_length = UTF16::get_char_count(text.char32_at(pass_limit));
                    pass_limit += char_length;
                    if pass_limit > run_limit {
                        break;
                    }
                    uncommitted_length += char_length;

                    index.limit = pass_limit;

                    // Delegate to subclass for actual transliteration.  Upon
                    // return, start will be updated to point after the
                    // transliterated text, and limit and contextLimit will be
                    // adjusted for length changes.
                    self.handle_transliterate(text, index, true)?;

                    let delta = index.limit - pass_limit; // change in length

                    // We failed to completely transliterate this pass.
                    // Roll back the text.  Indices remain unchanged; reset
                    // them where necessary.
                    if index.start != index.limit {
                        // Find the rollbackStart, adjusted for length changes
                        // and the deletion of partially transliterated text.
                        let rs = rollback_start + delta - (index.limit - pass_start);

                        // Delete the partially transliterated text
                        text.replace(pass_start, index.limit, &[]);

                        // Copy the rollback text back
                        text.copy(rs, rs + uncommitted_length, pass_start);

                        // Restore indices to their original values
                        index.start = pass_start;
                        index.limit = pass_limit;
                        index.context_limit -= delta;
                    }
                    // We did completely transliterate this pass.  Update the
                    // commit indices to record how far we got.  Adjust indices
                    // for length change.
                    else {
                        // Move the pass indices past the committed text.
                        pass_start = index.start;
                        pass_limit = index.start;

                        // Adjust the rollbackStart for length changes and move
                        // it past the committed text.  All characters we've
                        // processed to this point are committed now, so zero
                        // out the uncommittedLength.
                        rollback_start += delta + uncommitted_length;
                        uncommitted_length = 0;

                        // Adjust indices for length changes.
                        run_limit += delta;
                        total_delta += delta;
                    }
                }

                // Adjust overall limit and rollbackOrigin for insertions and
                // deletions.  Don't need to worry about contextLimit because
                // handleTransliterate() maintains that.
                rollback_origin += total_delta;
                global_limit += total_delta;

                // Delete the rollback copy
                text.replace(rollback_origin, rollback_origin + run_length, &[]);

                // Move start past committed text
                index.start = pass_start;
            } else {
                // Delegate to subclass for actual transliteration.
                let limit = index.limit;
                self.handle_transliterate(text, index, is_incremental_run)?;
                delta = index.limit - limit; // change in length

                // In a properly written transliterator, start == limit after
                // handleTransliterate() returns when incremental is false.
                // Catch cases where the subclass doesn't do this, and throw
                // an exception.  (Just pinning start to limit is a bad idea,
                // because what's probably happening is that the subclass
                // isn't transliterating all the way to the end, and it should
                // in non-incremental mode.)
                if !is_incremental_run && index.start != index.limit {
                    return Err(runtime_exception(Some(format!(
                        "ERROR: Incomplete non-incremental transliteration by {}",
                        to_rust_string(&self.get_id())
                    ))));
                }

                // Adjust overall limit for insertions/deletions.  Don't need
                // to worry about contextLimit because handleTransliterate()
                // maintains that.
                global_limit += delta;
            }

            if filter.is_none() || is_incremental_run {
                break;
            }

            // If we did completely transliterate this
            // run, then repeat with the next unfiltered run.
        }

        // Start is valid where it is.  Limit needs to be put back where
        // it was, modulo adjustments for deletions/insertions.
        index.limit = global_limit;
        Ok(())
    }

    /// `getMaximumContextLength()`: the length of the longest context
    /// required by this transliterator.
    pub fn get_maximum_context_length(&self) -> i32 {
        self.base().get_maximum_context_length()
    }

    /// `setMaximumContextLength(int a)` (protected).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a negative length.
    pub fn set_maximum_context_length(&self, a: i32) -> Result<()> {
        self.base().set_maximum_context_length(a)
    }

    /// `getID()`: a programmatic identifier for this transliterator.
    pub fn get_id(&self) -> Vec<u16> {
        self.base().get_id()
    }

    /// `setID(String id)` (protected).
    pub fn set_id(&self, id: Vec<u16>) {
        self.base().set_id(id);
    }

    /// `getFilter()`: the filter used by this transliterator, or `None` if
    /// this transliterator uses no filter.
    pub fn get_filter(&self) -> Option<Arc<UnicodeSet>> {
        self.base().get_filter()
    }

    /// `baseToRules(boolean escapeUnprintable)` (protected): `"::" + ID +
    /// ";"`, the ID escaped when asked.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` from `UTF16.append` (does not occur for
    /// code points read from the ID).
    pub fn base_to_rules(&self, escape_unprintable: bool) -> Result<Vec<u16>> {
        // The base class implementation of toRules munges the ID into
        // the correct format.  That is: foo => ::foo
        // KEEP in sync with rbt_pars
        let id = self.get_id();
        if escape_unprintable {
            let mut rules_source = Vec::new();
            let mut i = 0;
            while i < id.len() as i32 {
                let c = UTF16::char_at(&id, i);
                if !Utility::escape_unprintable(&mut rules_source, c) {
                    UTF16::append(&mut rules_source, c)?;
                }
                i += UTF16::get_char_count(c);
            }
            rules_source.splice(0..0, [b':' as u16, b':' as u16]);
            rules_source.push(ID_DELIM);
            return Ok(rules_source);
        }
        let mut s = vec![b':' as u16, b':' as u16];
        s.extend_from_slice(&id);
        s.push(ID_DELIM);
        Ok(s)
    }

    /// `getSourceSet()`: the set of all characters that may be modified in
    /// the input text by this transliterator. This incorporates this
    /// object's current filter.
    ///
    /// # Errors
    ///
    /// The errors of `addSourceTargetSet`.
    pub fn get_source_set(&self) -> Result<UnicodeSet> {
        let mut result = UnicodeSet::new();
        let filter = self.get_filter_as_unicode_set(&unicode_set::ALL_CODE_POINTS)?;
        self.add_source_target_set(&filter, &mut result, &mut UnicodeSet::new())?;
        Ok(result)
    }

    /// `getInverse()`: this transliterator's inverse, `getInstance(ID,
    /// REVERSE)`.
    ///
    /// # Errors
    ///
    /// As [`get_instance_dir`].
    pub fn get_inverse(&self) -> Result<Arc<dyn Transliterator>> {
        get_instance_dir(&self.get_id(), REVERSE)
    }
}

impl<'a> Transform<&'a [u16], Result<Vec<u16>>> for dyn Transliterator {
    /// `transform(String source)`: [`transliterate`](dyn Transliterator::transliterate).
    ///
    /// **Divergence.** Java's `Transliterator` implements
    /// `StringTransform` (`Transform<String,String>`,
    /// `Transliterator.java:2269-2271`); transliteration
    /// reports Java's runtime exceptions as `Result` here (crate
    /// convention), so the destination type is `Result<Vec<u16>>` and the
    /// crate's `StringTransform` (infallible) is not implemented.
    fn transform(&self, source: &'a [u16]) -> Result<Vec<u16>> {
        self.transliterate(source)
    }
}

/// A factory for transliterators, registered under an ID.
///
/// Equivalent to the interface `Transliterator.Factory`. Closures are
/// factories, as Java's anonymous classes are.
pub trait Factory: Send + Sync {
    /// `getInstance(String ID)`: a transliterator for the given ID, or
    /// `None`.
    ///
    /// # Errors
    ///
    /// The errors of the transliterator's construction.
    fn get_instance(&self, id: &[u16]) -> Result<Option<Arc<dyn Transliterator>>>;
}

impl<F> Factory for F
where
    F: Fn(&[u16]) -> Result<Option<Arc<dyn Transliterator>>> + Send + Sync,
{
    fn get_instance(&self, id: &[u16]) -> Result<Option<Arc<dyn Transliterator>>> {
        self(id)
    }
}

/// A [`Factory`] from a closure (Java's anonymous `Transliterator.Factory`
/// classes).
pub(crate) fn factory<F>(f: F) -> Arc<dyn Factory>
where
    F: Fn(&[u16]) -> Result<Option<Arc<dyn Transliterator>>> + Send + Sync + 'static,
{
    Arc::new(f)
}

/// `Class<? extends Transliterator>` of `registerClass`: the class's public
/// no-argument constructor (Java instantiates it with `newInstance()`).
pub type TransliteratorClass = fn() -> Arc<dyn Transliterator>;

// ---------------------------------------------------------------------------
// Static state
// ---------------------------------------------------------------------------

/// `registry`: the registry, built by Java's static initializer.
static REGISTRY: LazyLock<TransliteratorRegistry> = LazyLock::new(|| match init_registry() {
    Ok(registry) => registry,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

/// `displayNameCache`: a `Collections.synchronizedMap` of IDs to display
/// names, for transliterators registered with `registerClass`.
static DISPLAY_NAME_CACHE: LazyLock<Mutex<HashMap<CaseInsensitiveString, Vec<u16>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn display_name_cache() -> MutexGuard<'static, HashMap<CaseInsensitiveString, Vec<u16>>> {
    DISPLAY_NAME_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The registry of system transliterators.
pub(crate) fn registry() -> &'static TransliteratorRegistry {
    &REGISTRY
}

/// The body of Java's `static { ... }` block: registers the rule-based
/// transliterators listed in the root `translit` bundle, then the built-in
/// ones. The `register()` methods receive the registry under construction,
/// which Java's static calls reach through the half-initialized class.
fn init_registry() -> Result<TransliteratorRegistry> {
    let registry = TransliteratorRegistry::new();

    // Read the index file and populate the registry.
    let bundle = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_TRANSLIT_BASE_NAME), ROOT)?;
    let trans_ids = bundle.get(RB_RULE_BASED_IDS)?;

    let max_rows = trans_ids.get_size();
    for row in 0..max_rows {
        let col_bund = trans_ids.get_at(row)?;
        let id = col_bund.get_key().unwrap_or_default();
        if id.contains("-t-") {
            continue;
        }
        let res = col_bund.get_at(0)?;
        let ty = res.get_key().unwrap_or_default();
        if ty == "file" || ty == "internal" {
            // Rest of line is <resource>:<encoding>:<direction>
            //                   pos       colon      c2
            let res_string = res.get_string_by_key("resource")?;
            let direction = res.get_string_by_key("direction")?;
            let dir = match direction[0] {
                0x46 /* 'F' */ => FORWARD,
                0x52 /* 'R' */ => REVERSE,
                _ => {
                    return Err(runtime_exception(Some(format!("Can't parse direction: {}", to_rust_string(&direction)))));
                }
            };
            registry.put_resource(&units(&id), res_string, dir, ty != "internal")?;
        } else if ty == "alias" {
            // 'alias'; row[2]=createInstance argument
            let res_string = res.get_string()?;
            registry.put_alias(&units(&id), res_string, true)?;
        } else {
            // Unknown type
            return Err(runtime_exception(Some(format!("Unknown type: {ty}"))));
        }
    }

    register_special_inverse(&units(NullTransliterator::SHORT_ID), &units(NullTransliterator::SHORT_ID), false);

    // Register non-rule-based transliterators
    register_class_in(&registry, &units(NullTransliterator::ID), || Arc::new(NullTransliterator::new()), None)?;
    RemoveTransliterator::register(&registry)?;
    EscapeTransliterator::register(&registry)?;
    UnescapeTransliterator::register(&registry)?;
    LowercaseTransliterator::register(&registry)?;
    UppercaseTransliterator::register(&registry)?;
    TitlecaseTransliterator::register(&registry)?;
    CaseFoldTransliterator::register(&registry)?;
    UnicodeNameTransliterator::register(&registry)?;
    NameUnicodeTransliterator::register(&registry)?;
    NormalizationTransliterator::register(&registry)?;
    BreakTransliterator::register(&registry)?;
    AnyTransliterator::register(&registry)?; // do this last!
    Ok(registry)
}

// ---------------------------------------------------------------------------
// Static methods
// ---------------------------------------------------------------------------

/// `getDisplayName(String ID)`: a name for this transliterator that is
/// appropriate for display to the user in the default `DISPLAY` locale.
///
/// # Errors
///
/// As [`get_display_name_ulocale`].
pub fn get_display_name(id: &[u16]) -> Result<Vec<u16>> {
    get_display_name_ulocale(id, &ULocale::get_default_category(Category::Display))
}

/// `getDisplayName(String id, Locale inLocale)`.
///
/// # Errors
///
/// As [`get_display_name_ulocale`].
pub fn get_display_name_locale(id: &[u16], in_locale: &Locale) -> Result<Vec<u16>> {
    let locale = ULocale::for_locale(Some(in_locale))?;
    match locale {
        Some(locale) => get_display_name_ulocale(id, &locale),
        None => Err(crate::jdk::null_pointer("inLocale")),
    }
}

/// `getDisplayName(String id, ULocale inLocale)`: a name for this
/// transliterator that is appropriate for display to the user in the given
/// locale. This name is taken from the locale resource data in the
/// standard manner of the `java.text` package.
///
/// # Errors
///
/// `MissingResourceException` when the translit bundle cannot be opened;
/// `RuntimeException` (no message) when neither a display name nor the
/// name pattern is found; `IllegalArgumentException` for a malformed name
/// pattern.
pub fn get_display_name_ulocale(id: &[u16], in_locale: &ULocale) -> Result<Vec<u16>> {
    // Resource bundle containing display name keys and the
    // RB_RULE_BASED_IDS array.
    //
    //If we ever integrate this with the Sun JDK, the resource bundle
    // root will change to sun.text.resources.LocaleElements

    let bundle = UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_TRANSLIT_BASE_NAME), Some(in_locale))?;

    // Normalize the ID
    let stv = TransliteratorIDParser::id_to_stv(id);
    // (IDtoSTV never returns null in this version)
    let mut id_n = stv.0.clone();
    id_n.push(b'-' as u16);
    id_n.extend_from_slice(&stv.1);
    if !stv.2.is_empty() {
        id_n.push(b'/' as u16);
        id_n.extend_from_slice(&stv.2);
    }

    // Use the registered display name, if any
    if let Some(n) = display_name_cache().get(&CaseInsensitiveString::new(id_n.clone())) {
        return Ok(n.clone());
    }

    // Use display name for the entire transliterator, if it
    // exists.
    match bundle.get_string_by_key(&format!("{RB_DISPLAY_NAME_PREFIX}{}", to_rust_string(&id_n))) {
        Ok(s) => return Ok(s),
        Err(e) if is_missing_resource(&e) => {}
        Err(e) => return Err(e),
    }

    // Construct the formatter first; if getString() fails
    // we'll exit the try block
    match bundle.get_string_by_key(RB_DISPLAY_NAME_PATTERN) {
        Ok(pattern) => {
            let format = java_message_format::MessageFormat::new(&pattern)?;
            // Construct the argument array
            let mut args = [
                java_message_format::Arg::Integer(2),
                java_message_format::Arg::Str(stv.0.clone()),
                java_message_format::Arg::Str(stv.1.clone()),
            ];

            // Use display names for the scripts, if they exist
            for arg in args.iter_mut().skip(1) {
                let java_message_format::Arg::Str(s) = &*arg else { continue };
                match bundle.get_string_by_key(&format!("{RB_SCRIPT_DISPLAY_NAME_PREFIX}{}", to_rust_string(s))) {
                    Ok(name) => *arg = java_message_format::Arg::Str(name),
                    Err(e) if is_missing_resource(&e) => {}
                    Err(e) => return Err(e),
                }
            }

            // Format it using the pattern in the resource
            let mut result = format.format(&args)?;
            if !stv.2.is_empty() {
                result.push(b'/' as u16);
                result.extend_from_slice(&stv.2);
            }
            Ok(result)
        }
        Err(e) if is_missing_resource(&e) => {
            // Should not reach this point unless there is something
            // wrong with the build or the RB_DISPLAY_NAME_PATTERN has
            // been deleted from the root RB_LOCALE_ELEMENTS resource.
            Err(runtime_exception(None))
        }
        Err(e) => Err(e),
    }
}

/// `getInstance(String ID)`: a transliterator object given its ID, in the
/// forward direction.
///
/// # Errors
///
/// As [`get_instance_dir`].
pub fn get_instance(id: &[u16]) -> Result<Arc<dyn Transliterator>> {
    get_instance_dir(id, FORWARD)
}

/// `getInstance(String ID, int dir)`: a transliterator object given its ID
/// (a compound ID, possibly with filters) and direction.
///
/// # Errors
///
/// `IllegalArgumentException` if the given ID is invalid, and the errors of
/// the instantiation of its elements.
pub fn get_instance_dir(id: &[u16], dir: i32) -> Result<Arc<dyn Transliterator>> {
    let mut canon_id = Vec::new();
    let mut list = Vec::new();
    let mut global_filter: Option<UnicodeSet> = None;
    if !TransliteratorIDParser::parse_compound_id(id, dir, &mut canon_id, &mut list, &mut global_filter)? {
        return Err(LuceneError::illegal_argument(format!("Invalid ID {}", to_rust_string(id))));
    }

    let translits = TransliteratorIDParser::instantiate_list(&list)?;

    // assert(list.size() > 0);
    let t: Arc<dyn Transliterator> = if list.len() > 1 || index_of(&canon_id, &[b';' as u16], 0) >= 0 {
        // [NOTE: If it's a compoundID, we instantiate a CompoundTransliterator even if it only
        // has one child transliterator.  This is so that toRules() will return the right thing
        // (without any inactive ID), but our main ID still comes out correct.  That is, if we
        // instantiate "(Lower);Latin-Greek;", we want the rules to come out as "::Latin-Greek;"
        // even though the ID is "(Lower);Latin-Greek;".
        Arc::new(CompoundTransliterator::new(translits)?)
    } else {
        translits[0].clone()
    };

    t.set_id(canon_id);
    if let Some(global_filter) = global_filter {
        t.set_filter(Some(&global_filter))?;
    }
    Ok(t)
}

/// `getBasicInstance(String id, String canonID)`: create a transliterator
/// from a basic ID. This is an ID containing only the forward direction
/// source, target, and variant.
///
/// # Errors
///
/// The errors of the registry's instantiation.
pub(crate) fn get_basic_instance(id: &[u16], canon_id: Option<&[u16]>) -> Result<Option<Arc<dyn Transliterator>>> {
    let mut s = Vec::new();
    let mut t = registry().get(id, &mut s)?;
    if !s.is_empty() {
        // assert(t==0);
        // Instantiate an alias
        t = Some(get_instance_dir(&s, FORWARD)?);
    }
    if let (Some(t), Some(canon_id)) = (&t, canon_id) {
        t.set_id(canon_id.to_vec());
    }
    Ok(t)
}

/// `createFromRules(String ID, String rules, int dir)`: a transliterator
/// from the given rule string.
///
/// # Errors
///
/// `IllegalArgumentException` for malformed rules, and the errors of the
/// instantiation of the IDs they name.
pub fn create_from_rules(id: &[u16], rules: &[u16], dir: i32) -> Result<Arc<dyn Transliterator>> {
    let mut parser = TransliteratorParser::new();
    parser.parse(rules, dir)?;

    // NOTE: The logic here matches that in TransliteratorRegistry.
    let t: Arc<dyn Transliterator> = if parser.id_block_vector.is_empty() && parser.data_vector.is_empty() {
        Arc::new(NullTransliterator::new())
    } else if parser.id_block_vector.is_empty() && parser.data_vector.len() == 1 {
        Arc::new(RuleBasedTransliterator::new(
            id.to_vec(),
            parser.data_vector[0].clone(),
            parser.compound_filter.as_ref().map(|f| f as &dyn UnicodeFilter),
        )?)
    } else if parser.id_block_vector.len() == 1 && parser.data_vector.is_empty() {
        // [NOTE: The logic here matches that in TransliteratorRegistry.]
        let t = match &parser.compound_filter {
            Some(compound_filter) => {
                let mut s = compound_filter.to_pattern(false);
                s.push(b';' as u16);
                s.extend_from_slice(&parser.id_block_vector[0]);
                get_instance(&s)?
            }
            None => get_instance(&parser.id_block_vector[0])?,
        };
        t.set_id(id.to_vec());
        t
    } else {
        let mut transliterators: Vec<Arc<dyn Transliterator>> = Vec::new();
        let mut pass_number = 1;

        let limit = parser.id_block_vector.len().max(parser.data_vector.len());
        for i in 0..limit {
            if i < parser.id_block_vector.len() {
                let id_block = &parser.id_block_vector[i];
                if !id_block.is_empty() {
                    let temp = get_instance(id_block)?;
                    if !temp.as_any().is::<NullTransliterator>() {
                        transliterators.push(get_instance(id_block)?);
                    }
                }
            }
            if i < parser.data_vector.len() {
                let data = parser.data_vector[i].clone();
                transliterators.push(Arc::new(RuleBasedTransliterator::new(units(&format!("%Pass{pass_number}")), data, None)?));
                pass_number += 1;
            }
        }

        let t: Arc<dyn Transliterator> = Arc::new(CompoundTransliterator::with_anonymous_rbts(transliterators, pass_number - 1)?);
        t.set_id(id.to_vec());
        if let Some(compound_filter) = &parser.compound_filter {
            t.set_filter(Some(compound_filter))?;
        }
        t
    };

    Ok(t)
}

/// `getElements()`: the elements that make up this transliterator: the
/// children of a `CompoundTransliterator`, or the transliterator itself.
pub fn get_elements(this: &Arc<dyn Transliterator>) -> Vec<Arc<dyn Transliterator>> {
    if let Some(cpd) = this.as_any().downcast_ref::<CompoundTransliterator>() {
        let mut result = Vec::with_capacity(cpd.get_count() as usize);
        for i in 0..cpd.get_count() {
            result.push(cpd.get_transliterator(i));
        }
        result
    } else {
        vec![this.clone()]
    }
}

/// `registerClass(String ID, Class transClass, String displayName)` on the
/// given registry.
pub(crate) fn register_class_in(
    registry: &TransliteratorRegistry,
    id: &[u16],
    trans_class: TransliteratorClass,
    display_name: Option<&[u16]>,
) -> Result<()> {
    registry.put_class(id, trans_class, true)?;
    if let Some(display_name) = display_name {
        display_name_cache().insert(CaseInsensitiveString::new(id.to_vec()), display_name.to_vec());
    }
    Ok(())
}

/// `registerClass(String ID, Class<? extends Transliterator> transClass,
/// String displayName)`: registers a subclass of `Transliterator` with the
/// system. This subclass must have a public constructor taking no
/// arguments (here: the constructor function).
///
/// # Errors
///
/// The errors of the ID parsing (`UnicodeSet` patterns in the ID).
pub fn register_class(id: &[u16], trans_class: TransliteratorClass, display_name: Option<&[u16]>) -> Result<()> {
    register_class_in(registry(), id, trans_class, display_name)
}

/// `registerFactory(String ID, Factory factory)` on the given registry.
pub(crate) fn register_factory_in(registry: &TransliteratorRegistry, id: &[u16], factory: Arc<dyn Factory>) -> Result<()> {
    registry.put_factory(id, factory, true)
}

/// `registerFactory(String ID, Factory factory)`: register a factory
/// object with the given ID.
///
/// # Errors
///
/// As [`register_class`].
pub fn register_factory(id: &[u16], factory: Arc<dyn Factory>) -> Result<()> {
    register_factory_in(registry(), id, factory)
}

/// `registerInstance(Transliterator trans)`: register a transliterator
/// object with its ID.
///
/// # Errors
///
/// As [`register_class`].
pub fn register_instance(trans: Arc<dyn Transliterator>) -> Result<()> {
    register_instance_in(registry(), trans, true)
}

/// `registerInstance(Transliterator trans, boolean visible)` on the given
/// registry.
pub(crate) fn register_instance_in(registry: &TransliteratorRegistry, trans: Arc<dyn Transliterator>, visible: bool) -> Result<()> {
    let id = trans.get_id();
    registry.put_transliterator(&id, trans, visible)
}

/// `registerInstance(Transliterator trans, boolean visible)`: register a
/// transliterator object, visible or not.
///
/// # Errors
///
/// As [`register_class`].
pub(crate) fn register_instance_visible(trans: Arc<dyn Transliterator>, visible: bool) -> Result<()> {
    register_instance_in(registry(), trans, visible)
}

/// `registerAlias(String aliasID, String realID)`: register an ID as an
/// alias of another ID.
///
/// # Errors
///
/// As [`register_class`].
pub fn register_alias(alias_id: &[u16], real_id: &[u16]) -> Result<()> {
    registry().put_alias(alias_id, real_id.to_vec(), true)
}

/// `registerSpecialInverse(String target, String inverseTarget, boolean
/// bidirectional)`: register two targets as being inverses of one another.
pub(crate) fn register_special_inverse(target: &[u16], inverse_target: &[u16], bidirectional: bool) {
    TransliteratorIDParser::register_special_inverse(target, inverse_target, bidirectional);
}

/// `unregister(String ID)`: unregisters a transliterator or class.
///
/// # Errors
///
/// As [`register_class`].
pub fn unregister(id: &[u16]) -> Result<()> {
    display_name_cache().remove(&CaseInsensitiveString::new(id.to_vec()));
    registry().remove(id)
}

/// `getAvailableIDs()`: the programmatic identifiers of the currently
/// registered (visible) transliterators, in registration order.
pub fn get_available_ids() -> Vec<Vec<u16>> {
    registry().get_available_ids()
}

/// `getAvailableSources()`: all available source names.
pub fn get_available_sources() -> Vec<Vec<u16>> {
    registry().get_available_sources()
}

/// `getAvailableTargets(String source)`: all available target names for
/// the given source.
pub fn get_available_targets(source: &[u16]) -> Vec<Vec<u16>> {
    registry().get_available_targets(source)
}

/// `getAvailableVariants(String source, String target)`: all available
/// variant names for the given source and target.
pub fn get_available_variants(source: &[u16], target: &[u16]) -> Vec<Vec<u16>> {
    registry().get_available_variants(source, target)
}

/// `registerAny()`: register the script-based "Any" transliterators:
/// Any-Latin, Any-Greek, etc.
///
/// # Errors
///
/// As [`register_class`].
pub fn register_any() -> Result<()> {
    AnyTransliterator::register(registry())
}

// ---------------------------------------------------------------------------
// java.text.MessageFormat, as getDisplayName uses it
// ---------------------------------------------------------------------------

/// The part of `java.text.MessageFormat` (JDK 25) that
/// `Transliterator.getDisplayName` uses: `new MessageFormat(pattern)` and
/// `format(Object[])` over the arguments `{Integer, String, String}`.
///
/// **Divergence.** The JDK class (`Transliterator.java:1507-1523` uses it)
/// is not part of the workspace. This module
/// reproduces its pattern syntax (`MessageFormat.applyPatternImpl`,
/// `setFormatFromPattern`), the `choice` format type (`ChoiceFormat`
/// `applyPatternImpl` and `format(double)`, including the nested
/// `MessageFormat` of a choice result containing `{`) and the plain
/// argument formatting of `subformat`. The other format types (`number`,
/// `date`, `time`, `list`, the `DateTimeFormatter` types) are rejected
/// with `UnsupportedOperationException`, and an `Integer` argument without
/// a format type is rendered by `Integer.toString` where Java uses
/// `NumberFormat.getInstance(locale)` (identical for the constant `2`
/// `getDisplayName` passes, in every locale of the embedded data). The
/// `translit` bundles' `TransliteratorNamePattern` values use only
/// `choice` and plain arguments.
mod java_message_format {
    use rucene::error::{LuceneError, Result};

    use crate::jdk::{parse_double, to_rust_string, units};

    /// An argument of `format(Object[])`.
    #[derive(Debug, Clone)]
    pub(super) enum Arg {
        /// An `Integer`.
        Integer(i32),
        /// A `String`.
        Str(Vec<u16>),
    }

    /// `ChoiceFormat`: `choiceLimits` and `choiceFormats`.
    #[derive(Debug, Clone)]
    struct ChoiceFormat {
        choice_limits: Vec<f64>,
        choice_formats: Vec<Vec<u16>>,
    }

    impl ChoiceFormat {
        /// `new ChoiceFormat(String newPattern)` (`applyPatternImpl`).
        fn new(new_pattern: &[u16]) -> Result<Self> {
            let mut limits: Vec<f64> = Vec::new();
            let mut formats: Vec<Vec<u16>> = Vec::new();
            let mut segments: [Vec<u16>; 2] = [Vec::new(), Vec::new()];
            let mut part = 0usize;
            let mut limit = 0f64;
            let mut in_quote = false;
            let mut i = 0usize;
            while i < new_pattern.len() {
                let ch = new_pattern[i];
                match ch {
                    0x27 /* '\'' */ => {
                        if (i + 1) < new_pattern.len() && new_pattern[i + 1] == ch {
                            segments[part].push(ch);
                            i += 1;
                        } else {
                            in_quote = !in_quote;
                        }
                    }
                    0x3C /* '<' */ | 0x23 /* '#' */ | 0x2264 /* '≤' */ => {
                        if in_quote || part == 1 {
                            segments[part].push(ch);
                        } else {
                            if segments[0].is_empty() {
                                return Err(LuceneError::illegal_argument("Each interval must contain a number before a format"));
                            }
                            limit = string_to_num(&segments[0])?;
                            if ch == 0x3C && limit.is_finite() {
                                limit = limit.next_up();
                            }
                            if let Some(&last) = limits.last() {
                                if limit <= last {
                                    return Err(LuceneError::illegal_argument(
                                        "Incorrect order of intervals, must be in ascending order",
                                    ));
                                }
                            }
                            segments[0].clear();
                            part = 1;
                        }
                    }
                    0x7C /* '|' */ => {
                        if in_quote {
                            segments[part].push(ch);
                        } else if part == 1 {
                            limits.push(limit);
                            formats.push(segments[1].clone());
                            segments[1].clear();
                            part = 0;
                        }
                    }
                    _ => segments[part].push(ch),
                }
                i += 1;
            }
            if part == 1 {
                limits.push(limit);
                formats.push(segments[1].clone());
            }
            Ok(Self {
                choice_limits: limits,
                choice_formats: formats,
            })
        }

        /// `format(double number, ...)`.
        ///
        /// # Panics
        ///
        /// Java's `ArrayIndexOutOfBoundsException` for a pattern without
        /// choices.
        fn format(&self, number: f64) -> Vec<u16> {
            let mut i = 0;
            while i < self.choice_limits.len() {
                if !(number >= self.choice_limits[i]) {
                    // same as number < choiceLimits, except catches NaN
                    break;
                }
                i += 1;
            }
            let i = i.saturating_sub(1);
            match self.choice_formats.get(i) {
                Some(f) => f.clone(),
                None => panic!("java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0"),
            }
        }
    }

    /// `ChoiceFormat.stringToNum(String str)`.
    fn string_to_num(s: &[u16]) -> Result<f64> {
        if s == [0x221E] {
            Ok(f64::INFINITY)
        } else if s == [b'-' as u16, 0x221E] {
            Ok(f64::NEG_INFINITY)
        } else {
            parse_double(&to_rust_string(s))
        }
    }

    /// A subformat of a `MessageFormat`.
    #[derive(Debug, Clone)]
    enum SubFormat {
        Choice(ChoiceFormat),
    }

    /// `MessageFormat`: the literal text, and per argument its offset,
    /// argument number and format.
    #[derive(Debug, Clone)]
    pub(super) struct MessageFormat {
        pattern: Vec<u16>,
        offsets: Vec<i32>,
        argument_numbers: Vec<i32>,
        formats: Vec<Option<SubFormat>>,
    }

    const SEG_RAW: usize = 0;
    const SEG_INDEX: usize = 1;
    const SEG_TYPE: usize = 2;
    const SEG_MODIFIER: usize = 3;
    const MAX_ARGUMENT_INDEX: i32 = 10000;

    impl MessageFormat {
        /// `new MessageFormat(String pattern)` (`applyPatternImpl`).
        pub(super) fn new(pattern: &[u16]) -> Result<Self> {
            let mut this = Self {
                pattern: Vec::new(),
                offsets: Vec::new(),
                argument_numbers: Vec::new(),
                formats: Vec::new(),
            };
            let mut segments: [Option<Vec<u16>>; 4] = [Some(Vec::new()), None, None, None];
            let mut part = SEG_RAW;
            let mut format_number = 0usize;
            let mut in_quote = false;
            let mut brace_stack = 0;
            let mut i = 0usize;
            while i < pattern.len() {
                let ch = pattern[i];
                if part == SEG_RAW {
                    if ch == b'\'' as u16 {
                        if i + 1 < pattern.len() && pattern[i + 1] == b'\'' as u16 {
                            segments[part].get_or_insert_with(Vec::new).push(ch); // handle doubles
                            i += 1;
                        } else {
                            in_quote = !in_quote;
                        }
                    } else if ch == b'{' as u16 && !in_quote {
                        part = SEG_INDEX;
                        if segments[SEG_INDEX].is_none() {
                            segments[SEG_INDEX] = Some(Vec::new());
                        }
                    } else {
                        segments[part].get_or_insert_with(Vec::new).push(ch);
                    }
                } else if in_quote {
                    // just copy quotes in parts
                    segments[part].get_or_insert_with(Vec::new).push(ch);
                    if ch == b'\'' as u16 {
                        in_quote = false;
                    }
                } else {
                    match ch {
                        0x2C /* ',' */ => {
                            if part < SEG_MODIFIER {
                                part += 1;
                                if segments[part].is_none() {
                                    segments[part] = Some(Vec::new());
                                }
                            } else {
                                segments[part].get_or_insert_with(Vec::new).push(ch);
                            }
                        }
                        0x7B /* '{' */ => {
                            brace_stack += 1;
                            segments[part].get_or_insert_with(Vec::new).push(ch);
                        }
                        0x7D /* '}' */ => {
                            if brace_stack == 0 {
                                part = SEG_RAW;
                                this.set_format_from_pattern(format_number, &segments)?;
                                format_number += 1;
                                // throw away other segments
                                segments[SEG_INDEX] = None;
                                segments[SEG_TYPE] = None;
                                segments[SEG_MODIFIER] = None;
                            } else {
                                brace_stack -= 1;
                                segments[part].get_or_insert_with(Vec::new).push(ch);
                            }
                        }
                        0x20 /* ' ' */ => {
                            // Skip any leading space chars for SEG_TYPE.
                            if part != SEG_TYPE || segments[SEG_TYPE].as_ref().is_some_and(|s| !s.is_empty()) {
                                segments[part].get_or_insert_with(Vec::new).push(ch);
                            }
                        }
                        0x27 /* '\'' */ => {
                            in_quote = true;
                            segments[part].get_or_insert_with(Vec::new).push(ch);
                        }
                        _ => segments[part].get_or_insert_with(Vec::new).push(ch),
                    }
                }
                i += 1;
            }
            if brace_stack == 0 && part != 0 {
                return Err(LuceneError::illegal_argument("Unmatched braces in the pattern."));
            }
            this.pattern = segments[0].take().unwrap_or_default();
            Ok(this)
        }

        /// `setFormatFromPattern(int position, int offsetNumber,
        /// StringBuilder[] textSegments)`.
        fn set_format_from_pattern(&mut self, offset_number: usize, text_segments: &[Option<Vec<u16>>; 4]) -> Result<()> {
            let segments: Vec<Vec<u16>> = text_segments.iter().map(|s| s.clone().unwrap_or_default()).collect();

            // get the argument number
            let index = to_rust_string(&segments[SEG_INDEX]);
            let argument_number = match crate::jdk::parse_int(&segments[SEG_INDEX], 10) {
                Ok(n) => n,
                Err(e) => {
                    return Err(LuceneError::illegal_argument_with_cause(
                        format!("can't parse argument number: {index}"),
                        std::sync::Arc::new(e),
                    ));
                }
            };
            if argument_number < 0 {
                return Err(LuceneError::illegal_argument(format!("negative argument number: {argument_number}")));
            }
            if argument_number >= MAX_ARGUMENT_INDEX {
                return Err(LuceneError::illegal_argument(format!(
                    "{argument_number} exceeds the ArgumentIndex implementation limit"
                )));
            }

            let format = if !segments[SEG_TYPE].is_empty() {
                Some(Self::format_from_pattern(&segments[SEG_TYPE], &segments[SEG_MODIFIER])?)
            } else {
                None
            };
            // now get the format
            self.offsets.truncate(offset_number);
            self.argument_numbers.truncate(offset_number);
            self.formats.truncate(offset_number);
            self.offsets.push(segments[SEG_RAW].len() as i32);
            self.argument_numbers.push(argument_number);
            self.formats.push(format);
            Ok(())
        }

        /// `formatFromPattern(String type, String style)`.
        fn format_from_pattern(ty: &[u16], style: &[u16]) -> Result<SubFormat> {
            let ty_s = to_rust_string(ty);
            let upper = ty_s.trim_matches(|c: char| c <= '\u{20}').to_ascii_uppercase();
            const TYPES: [&str; 23] = [
                "NUMBER",
                "DATE",
                "TIME",
                "DTF_DATE",
                "DTF_TIME",
                "DTF_DATETIME",
                "CHOICE",
                "LIST",
                "BASIC_ISO_DATE",
                "ISO_LOCAL_DATE",
                "ISO_OFFSET_DATE",
                "ISO_DATE",
                "ISO_LOCAL_TIME",
                "ISO_OFFSET_TIME",
                "ISO_TIME",
                "ISO_LOCAL_DATE_TIME",
                "ISO_OFFSET_DATE_TIME",
                "ISO_ZONED_DATE_TIME",
                "ISO_DATE_TIME",
                "ISO_ORDINAL_DATE",
                "ISO_WEEK_DATE",
                "ISO_INSTANT",
                "RFC_1123_DATE_TIME",
            ];
            if !TYPES.contains(&upper.as_str()) {
                return Err(LuceneError::illegal_argument(format!("unknown format type: {ty_s}")));
            }
            if upper != "CHOICE" {
                return Err(LuceneError::unsupported_operation(format!(
                    "MessageFormat format type {ty_s} is not available in this port"
                )));
            }
            // case CHOICE -> formatFromSubformatPattern(fType, style)
            match ChoiceFormat::new(style) {
                Ok(c) => Ok(SubFormat::Choice(c)),
                Err(e) if matches!(e.kind(), rucene::error::LuceneErrorKind::IllegalArgument { .. }) => Err(e),
                Err(e) => Err(LuceneError::illegal_argument_with_cause(
                    format!("Choice pattern incorrect: {}", to_rust_string(style)),
                    std::sync::Arc::new(e),
                )),
            }
        }

        /// `format(Object[] arguments)` (`subformat`).
        pub(super) fn format(&self, arguments: &[Arg]) -> Result<Vec<u16>> {
            let mut result = Vec::new();
            let mut last_offset = 0usize;
            for i in 0..self.offsets.len() {
                let offset = self.offsets[i] as usize;
                result.extend_from_slice(&self.pattern[last_offset..offset]);
                last_offset = offset;
                let argument_number = self.argument_numbers[i];
                if argument_number as usize >= arguments.len() {
                    result.push(b'{' as u16);
                    result.extend_from_slice(&units(&argument_number.to_string()));
                    result.push(b'}' as u16);
                    continue;
                }

                let obj = &arguments[argument_number as usize];
                let arg = match &self.formats[i] {
                    Some(SubFormat::Choice(choice)) => {
                        let number = match obj {
                            Arg::Integer(n) => *n as f64,
                            Arg::Str(_) => {
                                return Err(LuceneError::illegal_argument("Cannot format given Object as a Number"));
                            }
                        };
                        let arg = choice.format(number);
                        if arg.contains(&(b'{' as u16)) {
                            MessageFormat::new(&arg)?.format(arguments)?
                        } else {
                            arg
                        }
                    }
                    None => match obj {
                        Arg::Integer(n) => units(&n.to_string()),
                        Arg::Str(s) => s.clone(),
                    },
                };
                result.extend_from_slice(&arg);
            }
            result.extend_from_slice(&self.pattern[last_offset..]);
            Ok(result)
        }
    }
}
