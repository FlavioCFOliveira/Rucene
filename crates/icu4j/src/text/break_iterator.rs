// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.BreakIterator`.
//!
//! Ported by rmp task #645 for `CaseMapImpl` title casing and
//! `UnicodeSet.closeOver` (owner #647, whose `BreakIteratorFactory` the
//! shim resolves).

use std::any::Any;
use std::fmt::Debug;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, StringCharacterIterator};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::runtime_exception;
use crate::r#impl::cs_character_iterator::CSCharacterIterator;
use crate::r#impl::icu_debug::ICUDebug;
use crate::r#impl::icu_service::Factory;
use crate::util::u_locale::{Type as ULocaleType, ULocale, ACTUAL_LOCALE};

/// `DEBUG`.
static DEBUG: LazyLock<bool> = LazyLock::new(|| ICUDebug::enabled_arg("breakiterator"));

/// `DONE`: the value returned by iteration functions at the end of the text.
pub const DONE: i32 = -1;

/// `WORD_NONE`: tag value for "words" that do not fit into any of the
/// other categories.
pub const WORD_NONE: i32 = 0;
/// `WORD_NONE_LIMIT`.
pub const WORD_NONE_LIMIT: i32 = 100;
/// `WORD_NUMBER`.
pub const WORD_NUMBER: i32 = 100;
/// `WORD_NUMBER_LIMIT`.
pub const WORD_NUMBER_LIMIT: i32 = 200;
/// `WORD_LETTER`.
pub const WORD_LETTER: i32 = 200;
/// `WORD_LETTER_LIMIT`.
pub const WORD_LETTER_LIMIT: i32 = 300;
/// `WORD_KANA`.
pub const WORD_KANA: i32 = 300;
/// `WORD_KANA_LIMIT`.
pub const WORD_KANA_LIMIT: i32 = 400;
/// `WORD_IDEO`.
pub const WORD_IDEO: i32 = 400;
/// `WORD_IDEO_LIMIT`.
pub const WORD_IDEO_LIMIT: i32 = 500;

/// `KIND_CHARACTER`.
pub const KIND_CHARACTER: i32 = 0;
/// `KIND_WORD`.
pub const KIND_WORD: i32 = 1;
/// `KIND_LINE`.
pub const KIND_LINE: i32 = 2;
/// `KIND_SENTENCE`.
pub const KIND_SENTENCE: i32 = 3;
/// `KIND_TITLE` (deprecated).
pub const KIND_TITLE: i32 = 4;
const KIND_COUNT: usize = 5;

/// The valid and actual locales of a break iterator (`validLocale`,
/// `actualLocale`), set by the factory.
#[derive(Debug, Clone, Default)]
pub struct BreakIteratorLocales {
    valid_locale: Option<ULocale>,
    actual_locale: Option<ULocale>,
}

/// A class that locates boundaries in text.
///
/// Equivalent to the abstract class `com.ibm.icu.text.BreakIterator`.
/// The abstract methods are required; the concrete ones are provided.
/// Java's `clone()` is [`clone_box`](Self::clone_box); the private locale
/// fields are reached through [`locales`](Self::locales).
pub trait BreakIterator: Debug + Send + Sync {
    /// `clone()`.
    fn clone_box(&self) -> Box<dyn BreakIterator>;

    /// The iterator as `Any`, for Java's casts to a concrete class.
    fn as_any(&self) -> &dyn Any;

    /// `first()`: the first boundary.
    fn first(&mut self) -> i32;

    /// `last()`: the last boundary.
    fn last(&mut self) -> i32;

    /// `next(int n)`: moves `n` boundaries.
    fn next_n(&mut self, n: i32) -> i32;

    /// `next()`: the next boundary, `DONE` at the end.
    fn next(&mut self) -> i32;

    /// `previous()`: the previous boundary, `DONE` at the start.
    fn previous(&mut self) -> i32;

    /// `following(int offset)`: the first boundary after `offset`.
    fn following(&mut self, offset: i32) -> i32;

    /// `preceding(int offset)`: the last boundary before `offset`.
    fn preceding(&mut self, offset: i32) -> i32 {
        // NOTE:  This implementation is here solely because we can't add new
        // abstract methods to an existing class.  There is almost ALWAYS a
        // better, faster way to do this.
        let mut pos = self.following(offset);
        while pos >= offset && pos != DONE {
            pos = self.previous();
        }
        pos
    }

    /// `isBoundary(int offset)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an implementation for an offset
    /// outside the text (the default implementation throws none).
    fn is_boundary(&mut self, offset: i32) -> Result<bool> {
        // Again, this is the default implementation, which is provided solely because
        // we couldn't add a new abstract method to an existing class.  The real
        // implementations will usually need to do a little more work.
        if offset == 0 {
            Ok(true)
        } else {
            Ok(self.following(offset - 1) == offset)
        }
    }

    /// `current()`: the current boundary.
    fn current(&self) -> i32;

    /// `getRuleStatus()`: the status tag of the rule that determined the
    /// boundary at the current position (0 by default).
    fn get_rule_status(&self) -> i32 {
        0
    }

    /// `getRuleStatusVec(int[] fillInArray)`: the status tags; returns
    /// their number (1 by default, with value 0).
    fn get_rule_status_vec(&self, fill_in_array: Option<&mut [i32]>) -> i32 {
        if let Some(f) = fill_in_array {
            if !f.is_empty() {
                f[0] = 0;
            }
        }
        1
    }

    /// `getText()`: the iterator over the text being analyzed (Java's
    /// shared reference; `None` where Java returns `null`).
    fn get_text(&self) -> Option<&dyn CharacterIterator>;

    /// `setText(String newText)`.
    fn set_text_string(&mut self, new_text: &[u16]) {
        self.set_text(Some(Box::new(StringCharacterIterator::new(
            new_text.to_vec(),
        ))));
    }

    /// `setText(CharSequence newText)`.
    fn set_text_char_sequence(&mut self, new_text: Arc<[u16]>) {
        self.set_text(Some(Box::new(CSCharacterIterator::new(new_text))));
    }

    /// `setText(CharacterIterator newText)` (`None` for Java's `null`).
    fn set_text(&mut self, new_text: Option<Box<dyn CharacterIterator>>);

    /// The locale fields of the base class.
    fn locales(&self) -> &BreakIteratorLocales;

    /// The locale fields of the base class, mutable.
    fn locales_mut(&mut self) -> &mut BreakIteratorLocales;

    /// `getLocale(ULocale.Type type)`: the actual or valid locale.
    fn get_locale(&self, ty: ULocaleType) -> Option<ULocale> {
        if ty == ACTUAL_LOCALE {
            self.locales().actual_locale.clone()
        } else {
            self.locales().valid_locale.clone()
        }
    }

    /// `setLocale(ULocale valid, ULocale actual)` (package-private).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when exactly one of them is `None`.
    fn set_locale(&mut self, valid: Option<ULocale>, actual: Option<ULocale>) -> Result<()> {
        // Change the following to an assertion later
        if valid.is_none() != actual.is_none() {
            return Err(crate::jdk::illegal_argument_no_message());
        }
        // Another check we could do is that the actual locale is at
        // the same level or less specific than the valid locale.
        let l = self.locales_mut();
        l.valid_locale = valid;
        l.actual_locale = actual;
        Ok(())
    }
}

impl Clone for Box<dyn BreakIterator> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// The service behind the factory methods.
///
/// Equivalent to `BreakIterator.BreakIteratorServiceShim` (package-private
/// abstract class).
pub trait BreakIteratorServiceShim: Send + Sync {
    /// `registerInstance(BreakIterator iter, ULocale l, int k)`: the
    /// registry key (the service's factory).
    ///
    /// # Errors
    ///
    /// The errors of the service.
    fn register_instance(
        &self,
        iter: Box<dyn BreakIterator>,
        l: &ULocale,
        k: i32,
    ) -> Result<Arc<dyn Factory>>;
    /// `unregister(Object key)`.
    fn unregister(&self, key: &Arc<dyn Factory>) -> bool;
    /// `getAvailableLocales()`.
    ///
    /// # Errors
    ///
    /// The errors of the service.
    fn get_available_locales(&self) -> Result<Vec<Locale>>;
    /// `getAvailableULocales()`.
    ///
    /// # Errors
    ///
    /// The errors of the service.
    fn get_available_u_locales(&self) -> Result<Vec<ULocale>>;
    /// `createBreakIterator(ULocale l, int k)`.
    ///
    /// # Errors
    ///
    /// The errors of the service.
    fn create_break_iterator(&self, l: &ULocale, k: i32) -> Result<Box<dyn BreakIterator>>;
}

/// `BreakIteratorCache` (private): a locale and a prototype iterator.
#[derive(Debug)]
struct BreakIteratorCache {
    iter: Box<dyn BreakIterator>,
    where_: ULocale,
}

impl BreakIteratorCache {
    fn new(where_: ULocale, iter: &dyn BreakIterator) -> Self {
        Self {
            where_,
            iter: iter.clone_box(),
        }
    }

    fn get_locale(&self) -> &ULocale {
        &self.where_
    }

    fn create_break_instance(&self) -> Box<dyn BreakIterator> {
        self.iter.clone_box()
    }
}

/// `iterCache`: the last iterator created per kind (a `CacheValue` whose
/// soft reference this crate never clears).
static ITER_CACHE: Mutex<[Option<BreakIteratorCache>; KIND_COUNT]> =
    Mutex::new([None, None, None, None, None]);

/// `shim`, created on first use.
static SHIM: Mutex<Option<Arc<dyn BreakIteratorServiceShim>>> = Mutex::new(None);

/// `Class.forName("com.ibm.icu.text.BreakIteratorFactory").newInstance()`:
/// the constructor of the factory class.
#[allow(clippy::type_complexity)] // type mirrors the Java signature
fn break_iterator_factory_class() -> Option<fn() -> Result<Arc<dyn BreakIteratorServiceShim>>> {
    Some(|| {
        Ok(
            Arc::new(crate::text::break_iterator_factory::BreakIteratorFactory::new()?)
                as Arc<dyn BreakIteratorServiceShim>,
        )
    })
}

fn get_shim() -> Result<Arc<dyn BreakIteratorServiceShim>> {
    // Note: this instantiation is safe on loose static variable
    let mut shim = SHIM.lock().unwrap_or_else(|e| e.into_inner());
    if shim.is_none() {
        match break_iterator_factory_class() {
            Some(new_instance) => match new_instance() {
                Ok(s) => *shim = Some(s),
                Err(e) if crate::util::u_resource_bundle::is_missing_resource(&e) => return Err(e),
                Err(e) => {
                    if *DEBUG {
                        eprintln!("{e}");
                    }
                    // `new RuntimeException(e.getMessage())`
                    return Err(runtime_exception(e.original_message().map(str::to_string)));
                }
            },
            None => {
                let e = "com.ibm.icu.text.BreakIteratorFactory";
                if *DEBUG {
                    eprintln!("java.lang.ClassNotFoundException: {e}");
                }
                return Err(runtime_exception(Some(e.to_string())));
            }
        }
    }
    Ok(shim.as_ref().expect("set above").clone())
}

/// `getWordInstance()`: for the default locale.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_word_instance() -> Result<Box<dyn BreakIterator>> {
    get_word_instance_ulocale(&ULocale::get_default())
}

/// `getWordInstance(Locale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_word_instance_locale(where_: &Locale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(&for_locale(where_)?, KIND_WORD)
}

/// `getWordInstance(ULocale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_word_instance_ulocale(where_: &ULocale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(where_, KIND_WORD)
}

/// `getLineInstance()`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_line_instance() -> Result<Box<dyn BreakIterator>> {
    get_line_instance_ulocale(&ULocale::get_default())
}

/// `getLineInstance(Locale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_line_instance_locale(where_: &Locale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(&for_locale(where_)?, KIND_LINE)
}

/// `getLineInstance(ULocale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_line_instance_ulocale(where_: &ULocale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(where_, KIND_LINE)
}

/// `getCharacterInstance()`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_character_instance() -> Result<Box<dyn BreakIterator>> {
    get_character_instance_ulocale(&ULocale::get_default())
}

/// `getCharacterInstance(Locale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_character_instance_locale(where_: &Locale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(&for_locale(where_)?, KIND_CHARACTER)
}

/// `getCharacterInstance(ULocale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_character_instance_ulocale(where_: &ULocale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(where_, KIND_CHARACTER)
}

/// `getSentenceInstance()`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_sentence_instance() -> Result<Box<dyn BreakIterator>> {
    get_sentence_instance_ulocale(&ULocale::get_default())
}

/// `getSentenceInstance(Locale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_sentence_instance_locale(where_: &Locale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(&for_locale(where_)?, KIND_SENTENCE)
}

/// `getSentenceInstance(ULocale where)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn get_sentence_instance_ulocale(where_: &ULocale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(where_, KIND_SENTENCE)
}

/// `getTitleInstance()` (deprecated).
///
/// # Errors
///
/// The errors of the factory.
pub fn get_title_instance() -> Result<Box<dyn BreakIterator>> {
    get_title_instance_ulocale(&ULocale::get_default())
}

/// `getTitleInstance(Locale where)` (deprecated).
///
/// # Errors
///
/// The errors of the factory.
pub fn get_title_instance_locale(where_: &Locale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(&for_locale(where_)?, KIND_TITLE)
}

/// `getTitleInstance(ULocale where)` (deprecated).
///
/// # Errors
///
/// The errors of the factory.
pub fn get_title_instance_ulocale(where_: &ULocale) -> Result<Box<dyn BreakIterator>> {
    get_break_instance(where_, KIND_TITLE)
}

fn for_locale(where_: &Locale) -> Result<ULocale> {
    Ok(ULocale::for_locale(Some(where_))?.expect("a non-null Locale maps to a ULocale"))
}

/// `registerInstance(BreakIterator iter, Locale locale, int kind)`.
///
/// # Errors
///
/// The errors of the factory.
pub fn register_instance_locale(
    iter: Box<dyn BreakIterator>,
    locale: &Locale,
    kind: i32,
) -> Result<Arc<dyn Factory>> {
    register_instance(iter, &for_locale(locale)?, kind)
}

/// `registerInstance(BreakIterator iter, ULocale locale, int kind)`.
///
/// # Errors
///
/// The errors of the factory.
///
/// # Panics
///
/// `ArrayIndexOutOfBoundsException` for a kind outside `0..5`.
pub fn register_instance(
    iter: Box<dyn BreakIterator>,
    locale: &ULocale,
    kind: i32,
) -> Result<Arc<dyn Factory>> {
    // If the registered object matches the one in the cache, then
    // flush the cached object.
    {
        let mut cache = ITER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = &cache[kind as usize] {
            if c.get_locale() == locale {
                cache[kind as usize] = None;
            }
        }
    }
    get_shim()?.register_instance(iter, locale, kind)
}

/// `unregister(Object key)`.
///
/// # Errors
///
/// `IllegalArgumentException("registry key must not be null")` for a
/// `None` key.
pub fn unregister(key: Option<&Arc<dyn Factory>>) -> Result<bool> {
    let Some(key) = key else {
        return Err(LuceneError::illegal_argument(
            "registry key must not be null",
        ));
    };
    let shim = SHIM.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(shim) = shim {
        // Unfortunately, we don't know what is being unregistered
        // -- what `kind' and what locale -- so we flush all
        // caches.  This is safe but inefficient if people are
        // actively registering and unregistering.
        let mut cache = ITER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        for c in cache.iter_mut() {
            *c = None;
        }
        drop(cache);
        return Ok(shim.unregister(key));
    }
    Ok(false)
}

/// `getBreakInstance(ULocale where, int kind)` (deprecated internal).
///
/// # Errors
///
/// The errors of the factory.
///
/// # Panics
///
/// `ArrayIndexOutOfBoundsException` for a kind outside `0..5`.
pub fn get_break_instance(where_: &ULocale, kind: i32) -> Result<Box<dyn BreakIterator>> {
    {
        let cache = ITER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = &cache[kind as usize] {
            if c.get_locale() == where_ {
                return Ok(c.create_break_instance());
            }
        }
    }

    // sigh, all to avoid linking in ICULocaleData...
    let result = get_shim()?.create_break_iterator(where_, kind)?;

    let cache = BreakIteratorCache::new(where_.clone(), result.as_ref());
    ITER_CACHE.lock().unwrap_or_else(|e| e.into_inner())[kind as usize] = Some(cache);
    Ok(result)
}

/// `getAvailableLocales()` (synchronized).
///
/// # Errors
///
/// The errors of the factory.
pub fn get_available_locales() -> Result<Vec<Locale>> {
    // to avoid linking ICULocaleData
    get_shim()?.get_available_locales()
}

/// `getAvailableULocales()` (synchronized).
///
/// # Errors
///
/// The errors of the factory.
pub fn get_available_u_locales() -> Result<Vec<ULocale>> {
    // to avoid linking ICULocaleData
    get_shim()?.get_available_u_locales()
}
