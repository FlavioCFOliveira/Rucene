// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 ****************************************************************************************
 * Copyright (C) 2009-2016, Google, Inc.; International Business Machines Corporation
 * and others. All Rights Reserved.
 ****************************************************************************************
 */

//! `com.ibm.icu.util.LocaleMatcher`.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result as JResult};
use rucene_queryparser::jdk::locale::Locale;

use crate::r#impl::locale::likely_subtags::LikelySubtags;
use crate::r#impl::locale::locale_distance::LocaleDistance;
use crate::r#impl::locale::lsr::LSR;
use crate::util::locale_priority_list::LocalePriorityList;
use crate::util::u_locale::{Builder as ULocaleBuilder, ULocale};

/// `UND_LSR`.
static UND_LSR: LazyLock<LSR> = LazyLock::new(|| LSR::new("und", "", "", LSR::EXPLICIT_LSR));
/// `UND_ULOCALE`: in ULocale, "und" and "" make the same object.
static UND_ULOCALE: LazyLock<ULocale> = LazyLock::new(|| ULocale::new("und").expect("\"und\" is a valid locale ID"));
/// `UND_LOCALE`: in Locale, "und" and "" make different objects.
static UND_LOCALE: LazyLock<Locale> = LazyLock::new(|| Locale::of("und", "", ""));
/// `EMPTY_LOCALE`.
static EMPTY_LOCALE: LazyLock<Locale> = LazyLock::new(|| Locale::of("", "", ""));

// (`TRACE_MATCHER` is a false constant: javac drops its blocks, which are not ported.)

/// Builder option for whether the language subtag or the script subtag is
/// most important.
///
/// Equivalent to `LocaleMatcher.FavorSubtag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FavorSubtag {
    /// `LANGUAGE`: language differences are most important, then script
    /// differences, then region differences.
    Language,
    /// `SCRIPT`: makes script differences matter relatively more than
    /// language differences.
    Script,
}

impl fmt::Display for FavorSubtag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FavorSubtag::Language => "LANGUAGE",
            FavorSubtag::Script => "SCRIPT",
        })
    }
}

/// Builder option for whether all desired locales are treated equally or
/// earlier ones are preferred.
///
/// Equivalent to `LocaleMatcher.Demotion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Demotion {
    /// `NONE`: all desired locales are treated equally.
    None,
    /// `REGION`: earlier desired locales are preferred.
    Region,
}

impl fmt::Display for Demotion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Demotion::None => "NONE",
            Demotion::Region => "REGION",
        })
    }
}

/// Builder option for whether to include or ignore one-way (fallback)
/// match data.
///
/// Equivalent to `LocaleMatcher.Direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// `WITH_ONE_WAY`: locale matching includes one-way matches.
    WithOneWay,
    /// `ONLY_TWO_WAY`: locale matching limited to two-way matches.
    OnlyTwoWay,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Direction::WithOneWay => "WITH_ONE_WAY",
            Direction::OnlyTwoWay => "ONLY_TWO_WAY",
        })
    }
}

/// Data for the best-matching pair of a desired and a supported locale.
///
/// Equivalent to `LocaleMatcher.Result`. Java's `null` references are
/// `None`.
#[derive(Debug, Clone)]
pub struct Result {
    desired_u_locale: Option<ULocale>,
    supported_u_locale: Option<ULocale>,
    desired_locale: Option<Locale>,
    supported_locale: Option<Locale>,
    desired_index: i32,
    supported_index: i32,
}

impl Result {
    /// `Result(...)` (private).
    fn new(
        udesired: Option<ULocale>,
        usupported: Option<ULocale>,
        desired: Option<Locale>,
        supported: Option<Locale>,
        des_index: i32,
        supp_index: i32,
    ) -> Self {
        Self {
            desired_u_locale: udesired,
            supported_u_locale: usupported,
            desired_locale: desired,
            supported_locale: supported,
            desired_index: des_index,
            supported_index: supp_index,
        }
    }

    /// `getDesiredULocale()`: the best-matching desired locale, `None` if
    /// the default locale was used.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn get_desired_u_locale(&self) -> JResult<Option<ULocale>> {
        if self.desired_u_locale.is_none() && self.desired_locale.is_some() {
            ULocale::for_locale(self.desired_locale.as_ref())
        } else {
            Ok(self.desired_u_locale.clone())
        }
    }

    /// `getDesiredLocale()`.
    pub fn get_desired_locale(&self) -> Option<Locale> {
        if self.desired_locale.is_none() {
            self.desired_u_locale.as_ref().map(ULocale::to_locale)
        } else {
            self.desired_locale.clone()
        }
    }

    /// `getSupportedULocale()`: the best-matching supported locale, or the
    /// default.
    pub fn get_supported_u_locale(&self) -> Option<ULocale> {
        self.supported_u_locale.clone()
    }

    /// `getSupportedLocale()`.
    pub fn get_supported_locale(&self) -> Option<Locale> {
        self.supported_locale.clone()
    }

    /// `getDesiredIndex()`: the index of the best-matching desired locale
    /// in the input iterable order, -1 if the default locale was used.
    pub fn get_desired_index(&self) -> i32 {
        self.desired_index
    }

    /// `getSupportedIndex()`: the index of the best-matching supported
    /// locale in the constructor's or builder's input order, -1 if the
    /// default locale was used.
    pub fn get_supported_index(&self) -> i32 {
        self.supported_index
    }

    /// `makeResolvedULocale()`: the supported locale with the region,
    /// variants and extensions of the desired locale.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.Builder` and `ULocale.forLocale`.
    pub fn make_resolved_u_locale(&self) -> JResult<Option<ULocale>> {
        let best_desired = self.get_desired_u_locale()?;
        let (supported, best_desired) = match (&self.supported_u_locale, best_desired) {
            (Some(s), Some(b)) if *s != b => (s, b),
            _ => return Ok(self.supported_u_locale.clone()),
        };
        let mut b = ULocaleBuilder::new();
        b.set_locale(supported)?;

        // Copy the region from bestDesired, if there is one.
        let region = best_desired.get_country();
        if !region.is_empty() {
            b.set_region(Some(&region))?;
        }

        // Copy the variants from bestDesired, if there are any.
        // Note that this will override any supportedULocale variants.
        // For example, "sco-ulster-fonipa" + "...-fonupa" => "sco-fonupa" (replacing ulster).
        let variants = best_desired.get_variant();
        if !variants.is_empty() {
            b.set_variant(Some(&variants))?;
        }

        // Copy the extensions from bestDesired, if there are any.
        // Note that this will override any supportedULocale extensions.
        // For example, "th-u-nu-latn-ca-buddhist" + "...-u-nu-native" => "th-u-nu-native"
        // (replacing calendar).
        for extension_key in best_desired.get_extension_keys() {
            let ext = best_desired.get_extension(extension_key)?;
            b.set_extension(extension_key, ext.as_deref())?;
        }
        Ok(Some(b.build()?))
    }

    /// `makeResolvedLocale()`.
    ///
    /// # Errors
    ///
    /// As [`make_resolved_u_locale`](Self::make_resolved_u_locale).
    pub fn make_resolved_locale(&self) -> JResult<Option<Locale>> {
        Ok(self.make_resolved_u_locale()?.map(|r| r.to_locale()))
    }
}

/// Immutable class that picks the best match between a user's desired
/// locales and an application's supported locales.
///
/// Equivalent to `com.ibm.icu.util.LocaleMatcher`. Java's `null` enum
/// options are `None`; the `HashMap<LSR, Integer>` of supported LSRs is a
/// `HashMap` (only looked up).
#[derive(Debug)]
pub struct LocaleMatcher {
    threshold_distance: i32,
    demotion_per_desired_locale: i32,
    favor_subtag: Option<FavorSubtag>,
    direction: Option<Direction>,

    // These are in input order.
    supported_u_locales: Vec<ULocale>,
    supported_locales: Vec<Locale>,
    // These are in preference order: 1. Default locale 2. paradigm locales 3. others.
    supported_lsr_to_index: HashMap<LSR, i32>,
    // Array versions of the supportedLsrToIndex keys and values.
    // The distance lookup loops over the supportedLSRs and returns the index of the best match.
    supported_lsrs: Vec<LSR>,
    supported_indexes: Vec<i32>,
    supported_lsrs_length: i32,
    default_u_locale: Option<ULocale>,
    default_locale: Option<Locale>,
}

/// LocaleMatcher builder.
///
/// Equivalent to `LocaleMatcher.Builder`.
#[derive(Debug, Clone)]
pub struct Builder {
    supported_locales: Option<Vec<ULocale>>,
    threshold_distance: i32,
    demotion: Option<Demotion>,
    default_locale: Option<ULocale>,
    with_default: bool,
    favor: Option<FavorSubtag>,
    direction: Option<Direction>,
    max_distance_desired: Option<ULocale>,
    max_distance_supported: Option<ULocale>,
}

impl Builder {
    /// `Builder()` (private).
    fn new() -> Self {
        Self {
            supported_locales: None,
            threshold_distance: -1,
            demotion: None,
            default_locale: None,
            with_default: true,
            favor: None,
            direction: None,
            max_distance_desired: None,
            max_distance_supported: None,
        }
    }

    /// `setSupportedLocales(String locales)`: parses an `Accept-Language`
    /// string and sets the supported locales.
    ///
    /// # Errors
    ///
    /// The errors of `LocalePriorityList.add(String)`.
    pub fn set_supported_locales_string(&mut self, locales: &str) -> JResult<&mut Self> {
        let list = LocalePriorityList::add_accept_language(locales)?.build();
        Ok(self.set_supported_u_locales(list.get_u_locales()))
    }

    /// `setSupportedULocales(Collection<ULocale> locales)`.
    pub fn set_supported_u_locales(&mut self, locales: impl IntoIterator<Item = ULocale>) -> &mut Self {
        self.supported_locales = Some(locales.into_iter().collect());
        self
    }

    /// `setSupportedLocales(Collection<Locale> locales)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn set_supported_locales(&mut self, locales: &[Locale]) -> JResult<&mut Self> {
        let mut v = Vec::with_capacity(locales.len());
        for locale in locales {
            v.push(ULocale::for_locale(Some(locale))?.expect("non-null locale"));
        }
        self.supported_locales = Some(v);
        Ok(self)
    }

    /// `addSupportedULocale(ULocale locale)`.
    pub fn add_supported_u_locale(&mut self, locale: ULocale) -> &mut Self {
        self.supported_locales.get_or_insert_with(Vec::new).push(locale);
        self
    }

    /// `addSupportedLocale(Locale locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn add_supported_locale(&mut self, locale: &Locale) -> JResult<&mut Self> {
        let l = ULocale::for_locale(Some(locale))?.expect("non-null locale");
        Ok(self.add_supported_u_locale(l))
    }

    /// `setNoDefaultLocale()`: no matching locale yields `None`.
    pub fn set_no_default_locale(&mut self) -> &mut Self {
        self.default_locale = None;
        self.with_default = false;
        self
    }

    /// `setDefaultULocale(ULocale defaultLocale)`: the default locale
    /// (`None` for `null`).
    pub fn set_default_u_locale(&mut self, default_locale: Option<ULocale>) -> &mut Self {
        self.default_locale = default_locale;
        self.with_default = true;
        self
    }

    /// `setDefaultLocale(Locale defaultLocale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn set_default_locale(&mut self, default_locale: Option<&Locale>) -> JResult<&mut Self> {
        self.default_locale = ULocale::for_locale(default_locale)?;
        self.with_default = true;
        Ok(self)
    }

    /// `setFavorSubtag(FavorSubtag subtag)`.
    pub fn set_favor_subtag(&mut self, subtag: Option<FavorSubtag>) -> &mut Self {
        self.favor = subtag;
        self
    }

    /// `setDemotionPerDesiredLocale(Demotion demotion)`.
    pub fn set_demotion_per_desired_locale(&mut self, demotion: Option<Demotion>) -> &mut Self {
        self.demotion = demotion;
        self
    }

    /// `setDirection(Direction direction)`.
    pub fn set_direction(&mut self, direction: Option<Direction>) -> &mut Self {
        self.direction = direction;
        self
    }

    /// `setMaxDistance(Locale desired, Locale supported)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("desired/supported locales must not be
    /// null")`; the errors of `ULocale.forLocale`.
    pub fn set_max_distance_locales(
        &mut self,
        desired: Option<&Locale>,
        supported: Option<&Locale>,
    ) -> JResult<&mut Self> {
        if desired.is_none() || supported.is_none() {
            return Err(LuceneError::illegal_argument("desired/supported locales must not be null"));
        }
        let d = ULocale::for_locale(desired)?;
        let s = ULocale::for_locale(supported)?;
        self.set_max_distance(d, s)
    }

    /// `setMaxDistance(ULocale desired, ULocale supported)`: the maximum
    /// distance is the one between these two locales.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("desired/supported locales must not be
    /// null")`.
    pub fn set_max_distance(&mut self, desired: Option<ULocale>, supported: Option<ULocale>) -> JResult<&mut Self> {
        if desired.is_none() || supported.is_none() {
            return Err(LuceneError::illegal_argument("desired/supported locales must not be null"));
        }
        self.max_distance_desired = desired;
        self.max_distance_supported = supported;
        Ok(self)
    }

    /// `internalSetThresholdDistance(int thresholdDistance)` (internal,
    /// deprecated).
    pub fn internal_set_threshold_distance(&mut self, mut threshold_distance: i32) -> &mut Self {
        if threshold_distance > 100 {
            threshold_distance = 100;
        }
        self.threshold_distance = threshold_distance;
        self
    }

    /// `build()`.
    ///
    /// # Errors
    ///
    /// The errors of loading the likely-subtags and distance data.
    pub fn build(&self) -> JResult<LocaleMatcher> {
        LocaleMatcher::from_builder(self)
    }
}

/// `toString()`.
impl fmt::Display for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::from("{LocaleMatcher.Builder");
        if let Some(sl) = self.supported_locales.as_ref().filter(|sl| !sl.is_empty()) {
            s.push_str(" supported={[");
            for (i, l) in sl.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&l.to_string());
            }
            s.push_str("]}");
        }
        if let Some(d) = &self.default_locale {
            s.push_str(" default=");
            s.push_str(&d.to_string());
        }
        if let Some(fav) = self.favor {
            s.push_str(" distance=");
            s.push_str(&fav.to_string());
        }
        if self.threshold_distance >= 0 {
            s.push_str(&format!(" threshold={}", self.threshold_distance));
        }
        if let Some(d) = self.demotion {
            s.push_str(" demotion=");
            s.push_str(&d.to_string());
        }
        s.push('}');
        f.write_str(&s)
    }
}

/// `ULocaleLsrIterator`/`LocaleLsrIterator` (subclasses of the private
/// `LsrIterator`): the maximized LSRs of the desired locales, remembering
/// the current locale when asked.
struct LsrIterator<T: Clone, I: Iterator<Item = T>> {
    locales: std::iter::Peekable<I>,
    current: Option<T>,
    remembered: Option<T>,
    best_desired_index: i32,
    to_lsr: fn(&T) -> JResult<LSR>,
}

impl<T: Clone, I: Iterator<Item = T>> LsrIterator<T, I> {
    fn new(locales: std::iter::Peekable<I>, to_lsr: fn(&T) -> JResult<LSR>) -> Self {
        Self {
            locales,
            current: None,
            remembered: None,
            best_desired_index: -1,
            to_lsr,
        }
    }
}

/// The operations `getBestSuppIndex` uses on an `LsrIterator`.
trait RemainingLsrs {
    fn has_next(&mut self) -> bool;
    fn next_lsr(&mut self) -> JResult<LSR>;
    fn remember_current(&mut self, desired_index: i32);
}

impl<T: Clone, I: Iterator<Item = T>> RemainingLsrs for LsrIterator<T, I> {
    fn has_next(&mut self) -> bool {
        self.locales.peek().is_some()
    }

    fn next_lsr(&mut self) -> JResult<LSR> {
        let current = self.locales.next().expect("hasNext() checked");
        let lsr = (self.to_lsr)(&current);
        self.current = Some(current);
        lsr
    }

    fn remember_current(&mut self, desired_index: i32) {
        self.best_desired_index = desired_index;
        self.remembered = self.current.clone();
    }
}

fn get_maximal_lsr_or_und(locale: &ULocale) -> JResult<LSR> {
    if *locale == *UND_ULOCALE {
        Ok(UND_LSR.clone())
    } else {
        LikelySubtags::instance()?.make_maximized_lsr_from(locale, false)
    }
}

fn get_maximal_lsr_or_und_locale(locale: &Locale) -> JResult<LSR> {
    if *locale == *UND_LOCALE || *locale == *EMPTY_LOCALE {
        Ok(UND_LSR.clone())
    } else {
        LikelySubtags::instance()?.make_maximized_lsr_from_locale(locale)
    }
}

impl LocaleMatcher {
    /// `builder()`.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// `LocaleMatcher(LocalePriorityList supportedLocales)`.
    ///
    /// # Errors
    ///
    /// As [`Builder::build`].
    pub fn from_priority_list(supported_locales: &LocalePriorityList) -> JResult<LocaleMatcher> {
        Self::from_builder(Self::builder().set_supported_u_locales(supported_locales.get_u_locales()))
    }

    /// `LocaleMatcher(String supportedLocales)`.
    ///
    /// # Errors
    ///
    /// The errors of parsing the list and of [`Builder::build`].
    pub fn from_string(supported_locales: &str) -> JResult<LocaleMatcher> {
        Self::from_builder(Self::builder().set_supported_locales_string(supported_locales)?)
    }

    /// `LocaleMatcher(Builder builder)` (private).
    fn from_builder(builder: &Builder) -> JResult<LocaleMatcher> {
        let distance = LocaleDistance::instance()?;
        let mut udef = builder.default_locale.clone();
        let mut def: Option<Locale> = None;
        let mut def_lsr: Option<LSR> = None;
        if let Some(u) = &udef {
            def = Some(u.to_locale());
            def_lsr = Some(get_maximal_lsr_or_und(u)?);
        }
        // Store the supported locales in input order,
        // so that when different types are used (e.g., java.util.Locale)
        // we can return those by parallel index.
        let supported_locales_length = builder.supported_locales.as_ref().map_or(0, Vec::len);
        let mut supported_u_locales = Vec::with_capacity(supported_locales_length);
        let mut supported_locales = Vec::with_capacity(supported_locales_length);
        // Supported LRSs in input order.
        let mut lsrs = Vec::with_capacity(supported_locales_length);
        if let Some(sl) = &builder.supported_locales {
            for locale in sl {
                supported_u_locales.push(locale.clone());
                supported_locales.push(locale.to_locale());
                lsrs.push(get_maximal_lsr_or_und(locale)?);
            }
        }

        // We need an unordered map from LSR to first supported locale with that LSR,
        // and an ordered list of (LSR, supported index) for
        // the supported locales in the following order:
        // 1. Default locale, if it is supported.
        // 2. Priority locales (aka "paradigm locales") in builder order.
        // 3. Remaining locales in builder order.
        let mut m = LocaleMatcher {
            threshold_distance: 0,
            demotion_per_desired_locale: 0,
            favor_subtag: None,
            direction: None,
            supported_u_locales,
            supported_locales,
            supported_lsr_to_index: HashMap::with_capacity(supported_locales_length),
            supported_lsrs: vec![UND_LSR.clone(); supported_locales_length],
            supported_indexes: vec![0; supported_locales_length],
            supported_lsrs_length: 0,
            default_u_locale: None,
            default_locale: None,
        };
        let mut supp_length = 0;
        // Determine insertion order.
        // Add locales immediately that are equivalent to the default.
        let mut order = vec![0u8; supported_locales_length];
        let mut num_paradigms = 0;
        for i in 0..supported_locales_length {
            let lsr = &lsrs[i];
            if def_lsr.is_none() && builder.with_default {
                // Implicit default locale = first supported locale, if not turned off.
                debug_assert!(i == 0);
                udef = Some(m.supported_u_locales[i].clone());
                def = Some(m.supported_locales[0].clone());
                def_lsr = Some(lsr.clone());
                supp_length = m.put_if_absent(lsr, 0, supp_length);
            } else if def_lsr.as_ref().is_some_and(|d| lsr.is_equivalent_to(d)) {
                supp_length = m.put_if_absent(lsr, i as i32, supp_length);
            } else if distance.is_paradigm_lsr(lsr) {
                order[i] = 2;
                num_paradigms += 1;
            } else {
                order[i] = 3;
            }
        }
        // Add supported paradigm locales.
        let paradigm_limit = supp_length + num_paradigms;
        let mut i = 0;
        while i < supported_locales_length && supp_length < paradigm_limit {
            if order[i] == 2 {
                supp_length = m.put_if_absent(&lsrs[i], i as i32, supp_length);
            }
            i += 1;
        }
        // Add remaining supported locales.
        for i in 0..supported_locales_length {
            if order[i] == 3 {
                supp_length = m.put_if_absent(&lsrs[i], i as i32, supp_length);
            }
        }
        m.supported_lsrs_length = supp_length;
        // If supportedLSRsLength < supportedLocalesLength then
        // we waste as many array slots as there are duplicate supported LSRs,
        // but the amount of wasted space is small as long as there are few duplicates.

        m.default_u_locale = udef;
        m.default_locale = def;
        m.demotion_per_desired_locale = if builder.demotion == Some(Demotion::None) {
            0
        } else {
            distance.get_default_demotion_per_desired_locale() // null or REGION
        };
        m.favor_subtag = builder.favor;
        m.direction = builder.direction;

        let threshold;
        if builder.threshold_distance >= 0 {
            threshold = builder.threshold_distance;
        } else if let Some(max_desired) = &builder.max_distance_desired {
            let max_supported = builder.max_distance_supported.as_ref().expect("set together with maxDistanceDesired");
            let index_and_distance = distance.get_best_index_and_distance(
                &get_maximal_lsr_or_und(max_desired)?,
                &[get_maximal_lsr_or_und(max_supported)?],
                1,
                LocaleDistance::shift_distance(100),
                m.favor_subtag,
                m.direction,
            )?;
            // +1 for an exclusive threshold from an inclusive max.
            threshold = LocaleDistance::get_distance_floor(index_and_distance) + 1;
        } else {
            threshold = distance.get_default_script_distance();
        }
        m.threshold_distance = threshold;
        Ok(m)
    }

    fn put_if_absent(&mut self, lsr: &LSR, i: i32, mut supp_length: i32) -> i32 {
        if !self.supported_lsr_to_index.contains_key(lsr) {
            self.supported_lsr_to_index.insert(lsr.clone(), i);
            self.supported_lsrs[supp_length as usize] = lsr.clone();
            self.supported_indexes[supp_length as usize] = i;
            supp_length += 1;
        }
        supp_length
    }

    /// `getBestMatch(ULocale desiredLocale)`: the best-matching supported
    /// locale, or the default locale (`None` when there is none).
    ///
    /// # Errors
    ///
    /// The errors of the likely-subtags and distance lookups.
    pub fn get_best_match(&self, desired_locale: &ULocale) -> JResult<Option<ULocale>> {
        let desired_lsr = get_maximal_lsr_or_und(desired_locale)?;
        let supp_index = self.get_best_supp_index(desired_lsr, None)?;
        Ok(if supp_index >= 0 {
            Some(self.supported_u_locales[supp_index as usize].clone())
        } else {
            self.default_u_locale.clone()
        })
    }

    /// `getBestMatch(Iterable<ULocale> desiredLocales)`: the best match for
    /// a list of desired locales in preference order.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_match_iter(&self, desired_locales: impl IntoIterator<Item = ULocale>) -> JResult<Option<ULocale>> {
        let mut desired_iter = desired_locales.into_iter().peekable();
        if desired_iter.peek().is_none() {
            return Ok(self.default_u_locale.clone());
        }
        let mut lsr_iter = LsrIterator::new(desired_iter, get_maximal_lsr_or_und);
        let desired_lsr = lsr_iter.next_lsr()?;
        let supp_index = self.get_best_supp_index(desired_lsr, Some(&mut lsr_iter))?;
        Ok(if supp_index >= 0 {
            Some(self.supported_u_locales[supp_index as usize].clone())
        } else {
            self.default_u_locale.clone()
        })
    }

    /// `getBestMatch(String desiredLocaleList)`: the best match for an
    /// `Accept-Language` string.
    ///
    /// # Errors
    ///
    /// The errors of `LocalePriorityList.add(String)` and of the lookups.
    pub fn get_best_match_string(&self, desired_locale_list: &str) -> JResult<Option<ULocale>> {
        let list = LocalePriorityList::add_accept_language(desired_locale_list)?.build();
        self.get_best_match_iter(list.get_u_locales())
    }

    /// `getBestLocale(Locale desiredLocale)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_locale(&self, desired_locale: &Locale) -> JResult<Option<Locale>> {
        let desired_lsr = get_maximal_lsr_or_und_locale(desired_locale)?;
        let supp_index = self.get_best_supp_index(desired_lsr, None)?;
        Ok(if supp_index >= 0 {
            Some(self.supported_locales[supp_index as usize].clone())
        } else {
            self.default_locale.clone()
        })
    }

    /// `getBestLocale(Iterable<Locale> desiredLocales)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_locale_iter(&self, desired_locales: impl IntoIterator<Item = Locale>) -> JResult<Option<Locale>> {
        let mut desired_iter = desired_locales.into_iter().peekable();
        if desired_iter.peek().is_none() {
            return Ok(self.default_locale.clone());
        }
        let mut lsr_iter = LsrIterator::new(desired_iter, get_maximal_lsr_or_und_locale);
        let desired_lsr = lsr_iter.next_lsr()?;
        let supp_index = self.get_best_supp_index(desired_lsr, Some(&mut lsr_iter))?;
        Ok(if supp_index >= 0 {
            Some(self.supported_locales[supp_index as usize].clone())
        } else {
            self.default_locale.clone()
        })
    }

    fn default_result(&self) -> Result {
        Result::new(None, self.default_u_locale.clone(), None, self.default_locale.clone(), -1, -1)
    }

    fn make_result_u(
        &self,
        desired_locale: Option<&ULocale>,
        lsr_iter: Option<(&Option<ULocale>, i32)>,
        supp_index: i32,
    ) -> Result {
        if supp_index < 0 {
            self.default_result()
        } else if let Some(d) = desired_locale {
            Result::new(
                Some(d.clone()),
                Some(self.supported_u_locales[supp_index as usize].clone()),
                None,
                Some(self.supported_locales[supp_index as usize].clone()),
                0,
                supp_index,
            )
        } else {
            let (remembered, best_desired_index) = lsr_iter.expect("an iterator when there is no single desired locale");
            Result::new(
                remembered.clone(),
                Some(self.supported_u_locales[supp_index as usize].clone()),
                None,
                Some(self.supported_locales[supp_index as usize].clone()),
                best_desired_index,
                supp_index,
            )
        }
    }

    fn make_result_l(
        &self,
        desired_locale: Option<&Locale>,
        lsr_iter: Option<(&Option<Locale>, i32)>,
        supp_index: i32,
    ) -> Result {
        if supp_index < 0 {
            self.default_result()
        } else if let Some(d) = desired_locale {
            Result::new(
                None,
                Some(self.supported_u_locales[supp_index as usize].clone()),
                Some(d.clone()),
                Some(self.supported_locales[supp_index as usize].clone()),
                0,
                supp_index,
            )
        } else {
            let (remembered, best_desired_index) = lsr_iter.expect("an iterator when there is no single desired locale");
            Result::new(
                None,
                Some(self.supported_u_locales[supp_index as usize].clone()),
                remembered.clone(),
                Some(self.supported_locales[supp_index as usize].clone()),
                best_desired_index,
                supp_index,
            )
        }
    }

    /// `getBestMatchResult(ULocale desiredLocale)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_match_result(&self, desired_locale: &ULocale) -> JResult<Result> {
        let desired_lsr = get_maximal_lsr_or_und(desired_locale)?;
        let supp_index = self.get_best_supp_index(desired_lsr, None)?;
        Ok(self.make_result_u(Some(desired_locale), None, supp_index))
    }

    /// `getBestMatchResult(Iterable<ULocale> desiredLocales)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_match_result_iter(&self, desired_locales: impl IntoIterator<Item = ULocale>) -> JResult<Result> {
        let mut desired_iter = desired_locales.into_iter().peekable();
        if desired_iter.peek().is_none() {
            return Ok(self.default_result());
        }
        let mut lsr_iter = LsrIterator::new(desired_iter, get_maximal_lsr_or_und);
        let desired_lsr = lsr_iter.next_lsr()?;
        let supp_index = self.get_best_supp_index(desired_lsr, Some(&mut lsr_iter))?;
        Ok(self.make_result_u(None, Some((&lsr_iter.remembered, lsr_iter.best_desired_index)), supp_index))
    }

    /// `getBestLocaleResult(Locale desiredLocale)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_locale_result(&self, desired_locale: &Locale) -> JResult<Result> {
        let desired_lsr = get_maximal_lsr_or_und_locale(desired_locale)?;
        let supp_index = self.get_best_supp_index(desired_lsr, None)?;
        Ok(self.make_result_l(Some(desired_locale), None, supp_index))
    }

    /// `getBestLocaleResult(Iterable<Locale> desiredLocales)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn get_best_locale_result_iter(&self, desired_locales: impl IntoIterator<Item = Locale>) -> JResult<Result> {
        let mut desired_iter = desired_locales.into_iter().peekable();
        if desired_iter.peek().is_none() {
            return Ok(self.default_result());
        }
        let mut lsr_iter = LsrIterator::new(desired_iter, get_maximal_lsr_or_und_locale);
        let desired_lsr = lsr_iter.next_lsr()?;
        let supp_index = self.get_best_supp_index(desired_lsr, Some(&mut lsr_iter))?;
        Ok(self.make_result_l(None, Some((&lsr_iter.remembered, lsr_iter.best_desired_index)), supp_index))
    }

    fn get_best_supp_index(&self, mut desired_lsr: LSR, mut remaining_iter: Option<&mut dyn RemainingLsrs>) -> JResult<i32> {
        let distance = LocaleDistance::instance()?;
        let mut desired_index = 0;
        let mut best_supported_lsr_index = -1;
        let mut best_shifted_distance = LocaleDistance::shift_distance(self.threshold_distance);
        loop {
            // Quick check for exact maximized LSR.
            if let Some(&index) = self.supported_lsr_to_index.get(&desired_lsr) {
                let supp_index = index;
                if let Some(r) = remaining_iter.as_deref_mut() {
                    r.remember_current(desired_index);
                }
                return Ok(supp_index);
            }
            let best_index_and_distance = distance.get_best_index_and_distance(
                &desired_lsr,
                &self.supported_lsrs,
                self.supported_lsrs_length,
                best_shifted_distance,
                self.favor_subtag,
                self.direction,
            )?;
            if best_index_and_distance >= 0 {
                best_shifted_distance = LocaleDistance::get_shifted_distance(best_index_and_distance);
                if let Some(r) = remaining_iter.as_deref_mut() {
                    r.remember_current(desired_index);
                }
                best_supported_lsr_index = LocaleDistance::get_index(best_index_and_distance);
            }
            best_shifted_distance -= LocaleDistance::shift_distance(self.demotion_per_desired_locale);
            if best_shifted_distance <= 0 {
                break;
            }
            let r = match remaining_iter.as_deref_mut() {
                Some(r) => r,
                None => break,
            };
            if !r.has_next() {
                break;
            }
            desired_lsr = r.next_lsr()?;
            desired_index += 1;
        }
        if best_supported_lsr_index < 0 {
            return Ok(-1);
        }
        Ok(self.supported_indexes[best_supported_lsr_index as usize])
    }

    /// `isMatch(Locale desired, Locale supported)`: whether the pair is a
    /// match below the threshold.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn is_match_locales(&self, desired: &Locale, supported: &Locale) -> JResult<bool> {
        let index_and_distance = LocaleDistance::instance()?.get_best_index_and_distance(
            &get_maximal_lsr_or_und_locale(desired)?,
            &[get_maximal_lsr_or_und_locale(supported)?],
            1,
            LocaleDistance::shift_distance(self.threshold_distance),
            self.favor_subtag,
            self.direction,
        )?;
        Ok(index_and_distance >= 0)
    }

    /// `isMatch(ULocale desired, ULocale supported)`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn is_match(&self, desired: &ULocale, supported: &ULocale) -> JResult<bool> {
        let index_and_distance = LocaleDistance::instance()?.get_best_index_and_distance(
            &get_maximal_lsr_or_und(desired)?,
            &[get_maximal_lsr_or_und(supported)?],
            1,
            LocaleDistance::shift_distance(self.threshold_distance),
            self.favor_subtag,
            self.direction,
        )?;
        Ok(index_and_distance >= 0)
    }

    /// `match(ULocale desired, ULocale desiredMax, ULocale supported,
    /// ULocale supportedMax)` (deprecated): `1 - distance/100`.
    ///
    /// # Errors
    ///
    /// As [`get_best_match`](Self::get_best_match).
    pub fn r#match(
        &self,
        desired: &ULocale,
        _desired_max: Option<&ULocale>,
        supported: &ULocale,
        _supported_max: Option<&ULocale>,
    ) -> JResult<f64> {
        // Returns the inverse of the distance: That is, 1-distance(desired, supported).
        let index_and_distance = LocaleDistance::instance()?.get_best_index_and_distance(
            &get_maximal_lsr_or_und(desired)?,
            &[get_maximal_lsr_or_und(supported)?],
            1,
            LocaleDistance::shift_distance(self.threshold_distance),
            self.favor_subtag,
            self.direction,
        )?;
        let distance = LocaleDistance::get_distance_double(index_and_distance);
        Ok((100.0 - distance) / 100.0)
    }

    /// `canonicalize(ULocale locale)`: the locale with the likely-subtags
    /// language and region aliases applied.
    ///
    /// # Errors
    ///
    /// The errors of `LikelySubtags.canonicalize`.
    pub fn canonicalize(&self, locale: &ULocale) -> JResult<ULocale> {
        LikelySubtags::instance()?.canonicalize(locale)
    }
}

/// `toString()`.
impl fmt::Display for LocaleMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::from("{LocaleMatcher");
        // Supported languages in the order that we try to match them.
        if self.supported_lsrs_length > 0 {
            s.push_str(" supportedLSRs={");
            s.push_str(&self.supported_lsrs[0].to_string());
            for i in 1..self.supported_lsrs_length as usize {
                s.push_str(", ");
                s.push_str(&self.supported_lsrs[i].to_string());
            }
            s.push('}');
        }
        s.push_str(" default=");
        match &self.default_u_locale {
            Some(d) => s.push_str(&d.to_string()),
            None => s.push_str("null"),
        }
        if let Some(fav) = self.favor_subtag {
            s.push_str(" favor=");
            s.push_str(&fav.to_string());
        }
        if let Some(d) = self.direction {
            s.push_str(" direction=");
            s.push_str(&d.to_string());
        }
        if self.threshold_distance >= 0 {
            s.push_str(&format!(" threshold={}", self.threshold_distance));
        }
        s.push_str(&format!(" demotion={}", self.demotion_per_desired_locale));
        s.push('}');
        f.write_str(&s)
    }
}
