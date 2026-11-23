// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CaseMap`.

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::lang::u_character::UCharacter;
use crate::r#impl::case_map_impl::{
    CaseMapImpl, OMIT_UNCHANGED_TEXT, TITLECASE_ADJUST_TO_CASED, TITLECASE_SENTENCES, TITLECASE_WHOLE_STRING,
};
use crate::r#impl::u_case_props::UCaseProps;
use crate::text::break_iterator::BreakIterator;
use crate::text::edits::Edits;

/// `TITLECASE_ITERATOR_MASK` of `CaseMapImpl` (package-private there).
const TITLECASE_ITERATOR_MASK: i32 = 0xe0;

/// `getCaseLocale(Locale locale)` (private): `Locale.getDefault()` for
/// `null`.
fn get_case_locale(locale: Option<&Locale>) -> i32 {
    match locale {
        Some(l) => UCaseProps::get_case_locale_jdk(l),
        None => UCaseProps::get_case_locale_jdk(&Locale::get_default()),
    }
}

/// Low-level case mapping options and methods. Immutable.
///
/// Equivalent to `com.ibm.icu.text.CaseMap` (abstract class with the final
/// subclasses [`Lower`], [`Upper`], [`Title`] and [`Fold`]). The shared
/// `DEFAULT`/`OMIT_UNCHANGED`/`TURKIC` instances are values (they are
/// immutable; Java never compares them by identity).
pub trait CaseMap {
    /// `internalOptions` (internal, deprecated).
    fn internal_options(&self) -> i32;

    /// `omitUnchangedText()`: an instance that behaves like this one but
    /// does not output unchanged text into an `Appendable` with `Edits`.
    fn omit_unchanged_text(&self) -> Self
    where
        Self: Sized;
}

/// `toLower()`: lowercasing object with default options.
pub fn to_lower() -> Lower {
    Lower::DEFAULT
}

/// `toUpper()`: uppercasing object with default options.
pub fn to_upper() -> Upper {
    Upper::DEFAULT
}

/// `toTitle()`: titlecasing object with default options.
pub fn to_title() -> Title {
    Title::DEFAULT
}

/// `fold()`: case folding object with default options.
pub fn fold() -> Fold {
    Fold::DEFAULT
}

/// Lowercasing options and methods. Immutable.
///
/// Equivalent to `CaseMap.Lower`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lower {
    internal_options: i32,
}

impl Lower {
    const DEFAULT: Lower = Lower { internal_options: 0 };
    const OMIT_UNCHANGED: Lower = Lower {
        internal_options: OMIT_UNCHANGED_TEXT,
    };

    /// `apply(Locale locale, CharSequence src)`: lowercases a string
    /// (`null` locale = the default locale).
    ///
    /// # Errors
    ///
    /// The errors of the case mapping.
    pub fn apply(&self, locale: Option<&Locale>, src: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::to_lower_str(get_case_locale(locale), self.internal_options, src)
    }

    /// `apply(Locale locale, CharSequence src, A dest, Edits edits)`:
    /// lowercases a string, appending to `dest` and recording edits.
    ///
    /// # Errors
    ///
    /// The errors of the case mapping.
    pub fn apply_to<'d>(
        &self,
        locale: Option<&Locale>,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        CaseMapImpl::to_lower(get_case_locale(locale), self.internal_options, src, dest, edits)
    }
}

impl CaseMap for Lower {
    fn internal_options(&self) -> i32 {
        self.internal_options
    }

    fn omit_unchanged_text(&self) -> Lower {
        Lower::OMIT_UNCHANGED
    }
}

/// Uppercasing options and methods. Immutable.
///
/// Equivalent to `CaseMap.Upper`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upper {
    internal_options: i32,
}

impl Upper {
    const DEFAULT: Upper = Upper { internal_options: 0 };
    const OMIT_UNCHANGED: Upper = Upper {
        internal_options: OMIT_UNCHANGED_TEXT,
    };

    /// `apply(Locale locale, CharSequence src)`: uppercases a string.
    ///
    /// # Errors
    ///
    /// The errors of the case mapping.
    pub fn apply(&self, locale: Option<&Locale>, src: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::to_upper_str(get_case_locale(locale), self.internal_options, src)
    }

    /// `apply(Locale locale, CharSequence src, A dest, Edits edits)`.
    ///
    /// # Errors
    ///
    /// The errors of the case mapping.
    pub fn apply_to<'d>(
        &self,
        locale: Option<&Locale>,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        CaseMapImpl::to_upper(get_case_locale(locale), self.internal_options, src, dest, edits)
    }
}

impl CaseMap for Upper {
    fn internal_options(&self) -> i32 {
        self.internal_options
    }

    fn omit_unchanged_text(&self) -> Upper {
        Upper::OMIT_UNCHANGED
    }
}

/// Titlecasing options and methods. Immutable.
///
/// Equivalent to `CaseMap.Title`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Title {
    internal_options: i32,
}

impl Title {
    const DEFAULT: Title = Title { internal_options: 0 };
    const OMIT_UNCHANGED: Title = Title {
        internal_options: OMIT_UNCHANGED_TEXT,
    };

    /// `wholeString()`: titlecase only the first character of the string.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("multiple titlecasing iterator options")`.
    pub fn whole_string(&self) -> Result<Title> {
        Ok(Title {
            internal_options: CaseMapImpl::add_title_iterator_option(self.internal_options, TITLECASE_WHOLE_STRING)?,
        })
    }

    /// `sentences()`: titlecase sentences rather than words.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("multiple titlecasing iterator options")`.
    pub fn sentences(&self) -> Result<Title> {
        Ok(Title {
            internal_options: CaseMapImpl::add_title_iterator_option(self.internal_options, TITLECASE_SENTENCES)?,
        })
    }

    /// `noLowercase()`: do not lowercase non-initial parts of words.
    pub fn no_lowercase(&self) -> Title {
        Title {
            internal_options: self.internal_options | UCharacter::TITLECASE_NO_LOWERCASE,
        }
    }

    /// `noBreakAdjustment()`: do not adjust the titlecasing break indexes.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("multiple titlecasing index adjustment
    /// options")`.
    pub fn no_break_adjustment(&self) -> Result<Title> {
        Ok(Title {
            internal_options: CaseMapImpl::add_title_adjustment_option(
                self.internal_options,
                UCharacter::TITLECASE_NO_BREAK_ADJUSTMENT,
            )?,
        })
    }

    /// `adjustToCased()`: adjust each titlecasing index to the next cased
    /// character.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("multiple titlecasing index adjustment
    /// options")`.
    pub fn adjust_to_cased(&self) -> Result<Title> {
        Ok(Title {
            internal_options: CaseMapImpl::add_title_adjustment_option(self.internal_options, TITLECASE_ADJUST_TO_CASED)?,
        })
    }

    /// `CaseMapImpl.getTitleBreakIterator(locale, internalOptions, iter)`
    /// followed by `iter.setText(src)`, over the caller's iterator or a new
    /// one, then `run` with it.
    fn with_title_iterator<R>(
        &self,
        locale: Option<&Locale>,
        iter: Option<&mut dyn BreakIterator>,
        src: &[u16],
        run: impl FnOnce(&mut dyn BreakIterator) -> Result<R>,
    ) -> Result<R> {
        match iter {
            Some(it) => {
                if (self.internal_options & TITLECASE_ITERATOR_MASK) != 0 {
                    return Err(LuceneError::illegal_argument(
                        "titlecasing iterator option together with an explicit iterator",
                    ));
                }
                it.set_text_string(src);
                run(it)
            }
            None => {
                // iter == null && locale == null -> locale = Locale.getDefault()
                let default;
                let l = match locale {
                    Some(l) => l,
                    None => {
                        default = Locale::get_default();
                        &default
                    }
                };
                let mut it = CaseMapImpl::get_title_break_iterator_locale(l, self.internal_options, None)?;
                it.set_text_string(src);
                run(it.as_mut())
            }
        }
    }

    /// `apply(Locale locale, BreakIterator iter, CharSequence src)`:
    /// titlecases a string; `None` iterator = the iterator the options
    /// select (word by default) for the locale.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an iterator option together with an
    /// explicit iterator; the errors of the case mapping.
    pub fn apply(&self, locale: Option<&Locale>, iter: Option<&mut dyn BreakIterator>, src: &[u16]) -> Result<Vec<u16>> {
        let default;
        let case_locale_arg = if iter.is_none() && locale.is_none() {
            default = Locale::get_default();
            Some(&default)
        } else {
            locale
        };
        let options = self.internal_options;
        self.with_title_iterator(case_locale_arg, iter, src, |it| {
            CaseMapImpl::to_title_str(get_case_locale(case_locale_arg), options, it, src)
        })
    }

    /// `apply(Locale locale, BreakIterator iter, CharSequence src, A dest,
    /// Edits edits)`.
    ///
    /// # Errors
    ///
    /// As [`apply`](Self::apply).
    pub fn apply_to<'d>(
        &self,
        locale: Option<&Locale>,
        iter: Option<&mut dyn BreakIterator>,
        src: &[u16],
        dest: &'d mut Vec<u16>,
        edits: Option<&mut Edits>,
    ) -> Result<&'d mut Vec<u16>> {
        let default;
        let case_locale_arg = if iter.is_none() && locale.is_none() {
            default = Locale::get_default();
            Some(&default)
        } else {
            locale
        };
        let options = self.internal_options;
        self.with_title_iterator(case_locale_arg, iter, src, |it| {
            CaseMapImpl::to_title(get_case_locale(case_locale_arg), options, it, src, dest, edits)
        })
    }
}

impl CaseMap for Title {
    fn internal_options(&self) -> i32 {
        self.internal_options
    }

    fn omit_unchanged_text(&self) -> Title {
        if self.internal_options == 0 || self.internal_options == OMIT_UNCHANGED_TEXT {
            return Title::OMIT_UNCHANGED;
        }
        Title {
            internal_options: self.internal_options | OMIT_UNCHANGED_TEXT,
        }
    }
}

/// Case folding options and methods. Immutable.
///
/// Equivalent to `CaseMap.Fold`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    internal_options: i32,
}

impl Fold {
    const DEFAULT: Fold = Fold { internal_options: 0 };
    const TURKIC: Fold = Fold {
        internal_options: UCharacter::FOLD_CASE_EXCLUDE_SPECIAL_I,
    };
    const OMIT_UNCHANGED: Fold = Fold {
        internal_options: OMIT_UNCHANGED_TEXT,
    };
    const TURKIC_OMIT_UNCHANGED: Fold = Fold {
        internal_options: UCharacter::FOLD_CASE_EXCLUDE_SPECIAL_I | OMIT_UNCHANGED_TEXT,
    };

    /// `turkic()`: use the Turkic (tr, az) mappings for dotted/dotless I.
    pub fn turkic(&self) -> Fold {
        if (self.internal_options & OMIT_UNCHANGED_TEXT) == 0 {
            Fold::TURKIC
        } else {
            Fold::TURKIC_OMIT_UNCHANGED
        }
    }

    /// `apply(CharSequence src)`: case-folds a string.
    ///
    /// # Errors
    ///
    /// The errors of the case mapping.
    pub fn apply(&self, src: &[u16]) -> Result<Vec<u16>> {
        CaseMapImpl::fold_str(self.internal_options, src)
    }

    /// `apply(CharSequence src, A dest, Edits edits)`.
    ///
    /// # Errors
    ///
    /// The errors of the case mapping.
    pub fn apply_to<'d>(&self, src: &[u16], dest: &'d mut Vec<u16>, edits: Option<&mut Edits>) -> Result<&'d mut Vec<u16>> {
        CaseMapImpl::fold(self.internal_options, src, dest, edits)
    }
}

impl CaseMap for Fold {
    fn internal_options(&self) -> i32 {
        self.internal_options
    }

    fn omit_unchanged_text(&self) -> Fold {
        if (self.internal_options & UCharacter::FOLD_CASE_EXCLUDE_SPECIAL_I) == 0 {
            Fold::OMIT_UNCHANGED
        } else {
            Fold::TURKIC_OMIT_UNCHANGED
        }
    }
}
