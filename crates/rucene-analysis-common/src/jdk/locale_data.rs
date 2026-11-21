/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! The JDK 25 locale data `SimpleDateFormat` parsing reads, and the
//! resolution of a locale to it.
//!
//! **Divergence.** An emulation of JDK 25, written from the locale-provider
//! sources of OpenJDK `jdk-25-ga` (`LocaleProviderAdapter`,
//! `CLDRLocaleProviderAdapter`, `FallbackLocaleProviderAdapter`,
//! `LocaleServiceProviderPool`, `CalendarNameProviderImpl`,
//! `ResourceBundle.Control`, `sun.util.resources.LocaleData`, read as
//! specification) over data `tools/translate/JdkDateParseData.java` reads
//! from a JDK 25 through its public API (`src/jdk/date_parse_data.bin`,
//! zlib, inflated on first use). No JDK code is reproduced.
//!
//! What a JDK resolves for a locale depends on the locale only through its
//! *class*: the adapter `LocaleProviderAdapter.getAdapter` picks for each of
//! the provider kinds parsing uses, the providers the calendar-name pool
//! asks per lookup locale, the weeded candidate chain of the CLDR adapter
//! (language aliases and CLDR parent locales applied) and that of the
//! FALLBACK adapter. The time-zone names of a locale also depend on the
//! class of each CLDR candidate of it. The generator enumerated every class
//! reachable from the subtags the JDK knows (and one unseen value of each),
//! read each class's data on a representative locale, and checked on a
//! sample of locales, with and without Unicode extensions, that the JDK's
//! own formats hold the data their class predicts. [`LocaleData::class_key`]
//! and [`LocaleData::zone_key`] compute the same keys here.
//!
//! The JDK computes some time-zone names (the GMT-format fallbacks) from the
//! current instant and caches derived names across locales; the data holds
//! the names a JVM computes when the locale is the first it formats, as of
//! the generation (see `tools/translate/README.md`).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::LazyLock;

use super::locale::Locale;

/// The compressed data.
static DATA_BYTES: &[u8] = include_bytes!("date_parse_data.bin");

/// The calendar types the data is indexed by: `Calendar.getAvailableCalendarTypes()`.
pub const CAL_TYPES: [&str; 3] = ["gregory", "buddhist", "japanese"];

/// `Calendar.ERA`.
pub const ERA: i32 = 0;
/// `Calendar.YEAR`.
pub const YEAR: i32 = 1;
/// `Calendar.MONTH`.
pub const MONTH: i32 = 2;
/// `Calendar.DAY_OF_WEEK`.
pub const DAY_OF_WEEK: i32 = 7;
/// `Calendar.AM_PM`.
pub const AM_PM: i32 = 9;
/// `Calendar.SHORT_FORMAT` (`SHORT`).
pub const SHORT_FORMAT: i32 = 1;
/// `Calendar.LONG_FORMAT` (`LONG`).
pub const LONG_FORMAT: i32 = 2;
/// `Calendar.SHORT_STANDALONE`.
pub const SHORT_STANDALONE: i32 = 0x8001;
/// `Calendar.LONG_STANDALONE`.
pub const LONG_STANDALONE: i32 = 0x8002;

/// The parse-relevant state of the integer `NumberFormat` (a
/// `DecimalFormat` with grouping off) `SimpleDateFormat` uses.
#[derive(Debug, Clone)]
pub struct NumberRecord {
    /// `DecimalFormatSymbols.getZeroDigit()`.
    pub zero_digit: u16,
    /// `DecimalFormatSymbols.getDecimalSeparator()`.
    pub decimal_separator: u16,
    /// `DecimalFormatSymbols.getMinusSign()`.
    pub minus_sign: u16,
    /// `DecimalFormatSymbols.getMinusSignText()`.
    pub minus_sign_text: Vec<u16>,
    /// `DecimalFormatSymbols.getExponentSeparator()`.
    pub exponent_separator: Vec<u16>,
    /// `DecimalFormatSymbols.getNaN()`.
    pub nan: Vec<u16>,
    /// `DecimalFormatSymbols.getInfinity()`.
    pub infinity: Vec<u16>,
    /// `DecimalFormat.getPositivePrefix()`.
    pub positive_prefix: Vec<u16>,
    /// `DecimalFormat.getPositiveSuffix()`.
    pub positive_suffix: Vec<u16>,
    /// `DecimalFormat.getNegativePrefix()`.
    pub negative_prefix: Vec<u16>,
    /// `DecimalFormat.getNegativeSuffix()`.
    pub negative_suffix: Vec<u16>,
    /// `SimpleDateFormat.checkNegativeNumberExpression`'s result on the
    /// format's pattern.
    pub has_following_minus_sign: bool,
}

/// One row of `DateFormatSymbols.getZoneStrings()`.
#[derive(Debug, Clone)]
pub struct ZoneRow {
    /// The zone ID (column 0).
    pub id: String,
    /// `TimeZone.getTimeZone(id).getID()`.
    pub resolved_id: String,
    /// Columns 1 to 4: long and short standard, long and short daylight
    /// names.
    pub names: [Vec<u16>; 4],
    /// Bit `i` set: name `i` is a GMT format whose digits are the locale's
    /// (stored with Latin digits when the zone class says so).
    pub digit_mask: u8,
}

/// The data of a locale class.
#[derive(Debug, Clone)]
pub struct ClassData {
    /// Per calendar type, the name-record index of the provider maps.
    pub names: [usize; 3],
    /// The name-record index of the maps `DateFormatSymbols` gives.
    pub symbols: usize,
    /// Per calendar type, `DateFormat.getDateInstance(DEFAULT, …)`'s pattern
    /// (`None` where no locale of the class has that calendar type).
    pub patterns: [Option<String>; 3],
    /// The default number record.
    pub number: usize,
    /// The number records of the numbering systems (`-u-nu-`) that differ.
    pub nu: HashMap<String, usize>,
}

/// The time-zone names of a zone class.
#[derive(Debug, Clone, Copy)]
pub struct ZoneClass {
    /// The table index.
    pub table: usize,
    /// Whether the masked names hold Latin digits, to be transliterated to
    /// the locale's zero digit.
    pub latin: bool,
}

/// The JDK locale data parsing reads.
#[derive(Debug)]
pub struct LocaleData {
    langtags: HashSet<String>,
    fallback_tags: HashSet<String>,
    fallback_zone_tags: HashSet<String>,
    aliases: HashMap<String, String>,
    /// CLDR parent locales: (parent tag, sorted child tags).
    parents: Vec<(String, Vec<String>)>,
    /// Likely scripts: (script, " lang1 lang2 ").
    likely: Vec<(String, String)>,
    nonlikely_script: bool,
    /// BCP 47 `tz` IDs and old IDs to canonical IDs.
    pub tz_canonical: HashMap<String, String>,
    /// The IDs `TimeZone.getTimeZone` knows, to the ID of the zone it returns.
    pub zone_ids: HashMap<String, String>,
    /// `isWeekDateSupported()` per calendar type.
    pub week_date: [bool; 3],
    /// `ZoneId.SHORT_IDS` keys.
    pub short_ids: HashSet<String>,
    /// Name sets (`None` for Java's `null` map).
    pub sets: Vec<Option<Vec<Vec<u16>>>>,
    /// The (field, style) pairs a name record holds, in order.
    pub combos: Vec<(i32, i32)>,
    /// Name records: a set index per combo.
    pub name_records: Vec<Vec<usize>>,
    /// Number records.
    pub numbers: Vec<NumberRecord>,
    /// Zone rows.
    pub rows: Vec<ZoneRow>,
    /// Zone tables: row indices.
    pub tables: Vec<Vec<usize>>,
    classes: HashMap<String, ClassData>,
    zone_classes: HashMap<String, ZoneClass>,
}

/// The data, inflated and decoded on first use.
pub fn data() -> &'static LocaleData {
    static DATA: LazyLock<LocaleData> = LazyLock::new(|| {
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(DATA_BYTES)
            .read_to_end(&mut bytes)
            .expect("INVARIANT: the embedded date data inflates");
        LocaleData::decode(&bytes)
    });
    &DATA
}

/// A reader of the generator's varints.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn v(&mut self) -> usize {
        let mut value = 0usize;
        let mut shift = 0;
        loop {
            let b = self.bytes[self.pos];
            self.pos += 1;
            value |= usize::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }
}

impl LocaleData {
    fn decode(bytes: &[u8]) -> LocaleData {
        assert_eq!(&bytes[..4], b"JDP1", "INVARIANT: the data format");
        let mut r = Reader { bytes, pos: 4 };
        let n = r.v();
        let mut strings = Vec::with_capacity(n);
        for _ in 0..n {
            let len = r.v();
            let s = std::str::from_utf8(&bytes[r.pos..r.pos + len])
                .expect("INVARIANT: the strings are UTF-8")
                .to_string();
            r.pos += len;
            strings.push(s);
        }
        let st = |r: &mut Reader<'_>| strings[r.v()].clone();
        let u16s = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
        let set_of = |r: &mut Reader<'_>| -> HashSet<String> {
            let n = r.v();
            (0..n).map(|_| st(r)).collect()
        };
        let map_of = |r: &mut Reader<'_>| -> HashMap<String, String> {
            let n = r.v();
            (0..n).map(|_| (st(r), st(r))).collect()
        };
        let langtags = set_of(&mut r);
        let fallback_tags = set_of(&mut r);
        let fallback_zone_tags = set_of(&mut r);
        let aliases = map_of(&mut r);
        let n = r.v();
        let mut parents = Vec::with_capacity(n);
        for _ in 0..n {
            let parent = st(&mut r);
            let m = r.v();
            let children = (0..m).map(|_| st(&mut r)).collect();
            parents.push((parent, children));
        }
        let n = r.v();
        let likely = (0..n).map(|_| (st(&mut r), st(&mut r))).collect();
        let nonlikely_script = r.v() != 0;
        let tz_canonical = map_of(&mut r);
        let zone_ids = map_of(&mut r);
        let week_date = [r.v() != 0, r.v() != 0, r.v() != 0];
        let short_ids = set_of(&mut r);
        let n = r.v();
        let mut sets = Vec::with_capacity(n);
        for _ in 0..n {
            if r.v() == 0 {
                sets.push(None);
            } else {
                let m = r.v();
                sets.push(Some((0..m).map(|_| u16s(&st(&mut r))).collect()));
            }
        }
        let n = r.v();
        let combos: Vec<(i32, i32)> = (0..n).map(|_| (r.v() as i32, r.v() as i32)).collect();
        let n = r.v();
        let name_records = (0..n)
            .map(|_| (0..combos.len()).map(|_| r.v()).collect())
            .collect();
        let n = r.v();
        let mut numbers = Vec::with_capacity(n);
        for _ in 0..n {
            let zero_digit = r.v() as u16;
            let decimal_separator = r.v() as u16;
            let minus_sign = r.v() as u16;
            let mut s = || u16s(&st(&mut r));
            let minus_sign_text = s();
            let exponent_separator = s();
            let nan = s();
            let infinity = s();
            let positive_prefix = s();
            let positive_suffix = s();
            let negative_prefix = s();
            let negative_suffix = s();
            let has_following_minus_sign = r.v() != 0;
            numbers.push(NumberRecord {
                zero_digit,
                decimal_separator,
                minus_sign,
                minus_sign_text,
                exponent_separator,
                nan,
                infinity,
                positive_prefix,
                positive_suffix,
                negative_prefix,
                negative_suffix,
                has_following_minus_sign,
            });
        }
        let n = r.v();
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            let id = st(&mut r);
            let resolved_id = st(&mut r);
            let names = [
                u16s(&st(&mut r)),
                u16s(&st(&mut r)),
                u16s(&st(&mut r)),
                u16s(&st(&mut r)),
            ];
            let digit_mask = r.v() as u8;
            rows.push(ZoneRow {
                id,
                resolved_id,
                names,
                digit_mask,
            });
        }
        let n = r.v();
        let tables = (0..n)
            .map(|_| {
                let m = r.v();
                (0..m).map(|_| r.v()).collect()
            })
            .collect();
        let n = r.v();
        let mut classes = HashMap::with_capacity(n);
        for _ in 0..n {
            let key = st(&mut r);
            let names = [r.v(), r.v(), r.v()];
            let symbols = r.v();
            let mut patterns: [Option<String>; 3] = [None, None, None];
            for p in &mut patterns {
                let id = r.v();
                *p = if id == 0 {
                    None
                } else {
                    Some(strings[id - 1].clone())
                };
            }
            let number = r.v();
            let m = r.v();
            let nu = (0..m).map(|_| (st(&mut r), r.v())).collect();
            classes.insert(
                key,
                ClassData {
                    names,
                    symbols,
                    patterns,
                    number,
                    nu,
                },
            );
        }
        let n = r.v();
        let mut zone_classes = HashMap::with_capacity(n);
        for _ in 0..n {
            let key = st(&mut r);
            let table = r.v();
            let latin = r.v() != 0;
            zone_classes.insert(key, ZoneClass { table, latin });
        }
        LocaleData {
            langtags,
            fallback_tags,
            fallback_zone_tags,
            aliases,
            parents,
            likely,
            nonlikely_script,
            tz_canonical,
            zone_ids,
            week_date,
            short_ids,
            sets,
            combos,
            name_records,
            numbers,
            rows,
            tables,
            classes,
            zone_classes,
        }
    }

    // ------------------------------------------------------------- resolution

    /// `ResourceBundle.Control.getCandidateLocales(baseName, locale)` (the
    /// default list, with the Norwegian and Chinese special cases), over the
    /// base locale of `locale`.
    pub fn control_candidates(locale: &Locale) -> Vec<Locale> {
        let language = locale.get_language();
        let mut script = locale.get_script().to_string();
        let region = locale.get_country();
        let mut variant = locale.get_variant().to_string();

        let mut is_norwegian_bokmal = false;
        let mut is_norwegian_nynorsk = false;
        if language == "no" {
            if region == "NO" && variant == "NY" {
                variant.clear();
                is_norwegian_nynorsk = true;
            } else {
                is_norwegian_bokmal = true;
            }
        }
        if language == "nb" || is_norwegian_bokmal {
            let tmp = Self::default_list("nb", &script, region, &variant);
            let mut bokmal = Vec::new();
            for l_nb in tmp {
                let is_root = l_nb.get_language().is_empty();
                let l_no = Locale::of_parts(
                    if is_root { "" } else { "no" },
                    l_nb.get_script(),
                    l_nb.get_country(),
                    l_nb.get_variant(),
                );
                if is_norwegian_bokmal {
                    bokmal.push(l_no);
                } else {
                    bokmal.push(l_nb.clone());
                }
                if is_root {
                    break;
                }
                if is_norwegian_bokmal {
                    bokmal.push(l_nb);
                } else {
                    let l_no = Locale::of_parts(
                        "no",
                        l_nb.get_script(),
                        l_nb.get_country(),
                        l_nb.get_variant(),
                    );
                    bokmal.push(l_no);
                }
            }
            return bokmal;
        } else if language == "nn" || is_norwegian_nynorsk {
            let mut nynorsk = Self::default_list("nn", &script, region, &variant);
            let mut idx = nynorsk.len() - 1;
            nynorsk.insert(idx, Locale::of_parts("no", "", "NO", "NY"));
            idx += 1;
            nynorsk.insert(idx, Locale::of_parts("no", "", "NO", ""));
            idx += 1;
            nynorsk.insert(idx, Locale::of_parts("no", "", "", ""));
            return nynorsk;
        } else if language == "zh" && script.is_empty() && !region.is_empty() {
            match region {
                "TW" | "HK" | "MO" => script = "Hant".to_string(),
                "CN" | "SG" => script = "Hans".to_string(),
                _ => {}
            }
        }
        Self::default_list(language, &script, region, &variant)
    }

    /// `ResourceBundle.Control.CandidateListCache.getDefaultList`.
    fn default_list(language: &str, script: &str, region: &str, variant: &str) -> Vec<Locale> {
        let mut variants: Option<Vec<String>> = None;
        if !variant.is_empty() {
            // `idx = variant.lastIndexOf('_', --idx)` from the variant's
            // length down; a variant is ASCII.
            let mut vs = Vec::new();
            let mut idx = variant.len() as isize;
            while idx != -1 {
                vs.push(variant[..idx as usize].to_string());
                idx -= 1;
                idx = if idx < 0 {
                    -1
                } else {
                    variant[..=idx as usize]
                        .rfind('_')
                        .map_or(-1, |p| p as isize)
                };
            }
            variants = Some(vs);
        }
        let mut region = region.to_string();
        let mut list = Vec::new();
        if let Some(vs) = &variants {
            for v in vs {
                list.push(Locale::of_parts(language, script, &region, v));
            }
        }
        if !region.is_empty() {
            list.push(Locale::of_parts(language, script, &region, ""));
        }
        if !script.is_empty() {
            list.push(Locale::of_parts(language, script, "", ""));
            if language == "zh" && region.is_empty() {
                match script {
                    "Hans" => region = "CN".to_string(),
                    "Hant" => region = "TW".to_string(),
                    _ => {}
                }
            }
            if let Some(vs) = &variants {
                for v in vs {
                    list.push(Locale::of_parts(language, "", &region, v));
                }
            }
            if !region.is_empty() {
                list.push(Locale::of_parts(language, "", &region, ""));
            }
        }
        if !language.is_empty() {
            list.push(Locale::of_parts(language, "", "", ""));
        }
        list.push(Locale::default());
        list
    }

    /// `CLDRLocaleProviderAdapter.applyAliases(Locale)`.
    fn apply_aliases(&self, locale: &Locale) -> Locale {
        match self.aliases.get(&locale.to_language_tag()) {
            Some(alias) => Locale::builder_from_language_tag(alias)
                .expect("INVARIANT: the CLDR aliases are well-formed tags"),
            None => locale.clone(),
        }
    }

    /// `CLDRLocaleProviderAdapter.getParentLocale(Locale)`.
    fn parent_locale(&self, locale: &Locale) -> Option<Locale> {
        // The adapter assumes these locales have no irregular parent.
        if locale.is_root() || *locale == Locale::english() || *locale == Locale::us() {
            return None;
        }
        let tag = locale.to_language_tag();
        for (parent, children) in &self.parents {
            if children.binary_search(&tag).is_ok() {
                let parent = Locale::builder_from_language_tag(parent)
                    .expect("INVARIANT: the CLDR parents are well-formed tags");
                return if parent == *locale {
                    None
                } else {
                    Some(parent)
                };
            }
        }
        if self.nonlikely_script && locale.get_country().is_empty() {
            let lang = format!(" {} ", locale.get_language());
            let script = locale.get_script();
            if !script.is_empty() {
                if let Some((likely, _)) =
                    self.likely.iter().find(|(_, langs)| langs.contains(&lang))
                {
                    return if likely == script {
                        None
                    } else {
                        Some(Locale::default())
                    };
                }
            }
        }
        None
    }

    /// `CLDRLocaleProviderAdapter.applyParentLocales(String, List)`.
    fn apply_parent_locales(&self, candidates: Vec<Locale>) -> Vec<Locale> {
        for i in 0..candidates.len() {
            let l = &candidates[i];
            if l.is_root() {
                continue;
            }
            if let Some(p) = self.parent_locale(l) {
                if candidates.get(i + 1) != Some(&p) {
                    let mut applied: Vec<Locale> = candidates[..=i].to_vec();
                    let nb = Locale::of_parts("nb", "", "", "");
                    let no = Locale::of_parts("no", "", "", "");
                    if p == nb || p == no {
                        applied.push(no);
                        applied.push(Locale::default());
                    } else {
                        applied.extend(self.apply_parent_locales(Self::control_candidates(&p)));
                    }
                    return applied;
                }
            }
        }
        candidates
    }

    /// `CLDRLocaleProviderAdapter.getCandidateLocales("", locale)`.
    pub fn cldr_candidates(&self, locale: &Locale) -> Vec<Locale> {
        self.apply_parent_locales(Self::control_candidates(&self.apply_aliases(locale)))
    }

    /// `CLDRLocaleProviderAdapter.isSupportedProviderLocale(locale,
    /// AvailableLocales)`.
    fn cldr_supports(&self, locale: &Locale) -> bool {
        if locale.is_root()
            || self
                .langtags
                .contains(&locale.strip_extensions().to_language_tag())
        {
            return true;
        }
        let equivalent = match locale.to_string().as_str() {
            "no" | "no_NO" => Locale::of_parts("nb", "", "", ""),
            _ => self.apply_aliases(locale),
        };
        self.langtags.contains(&equivalent.to_language_tag())
    }

    /// `FallbackLocaleProviderAdapter.isSupportedProviderLocale(locale, tags)`.
    fn fallback_supports(locale: &Locale, tags: &HashSet<String>) -> bool {
        locale.is_root() || tags.contains(&locale.strip_extensions().to_language_tag())
    }

    /// `LocaleProviderAdapter.getAdapter(providerClass, locale)`: `'C'` for
    /// CLDR, `'F'` for FALLBACK, the providers asking `tags` as the FALLBACK
    /// adapter's language tags.
    fn adapter(&self, locale: &Locale, tags: &HashSet<String>) -> char {
        let find = |l: &Locale| -> Option<char> {
            if self.cldr_supports(l) {
                Some('C')
            } else if Self::fallback_supports(l, tags) {
                Some('F')
            } else {
                None
            }
        };
        if let Some(a) = find(locale) {
            return a;
        }
        for c in Self::control_candidates(locale) {
            if c == *locale {
                continue;
            }
            if let Some(a) = find(&c) {
                return a;
            }
        }
        'F'
    }

    /// The calendar-type check both calendar-name providers' `isSupportedLocale`
    /// start with: `None` when the locale is the root (supported) or names an
    /// unknown `-u-ca-` type (unsupported), else the locale without its
    /// extensions.
    fn calendar_names_locale(locale: &Locale) -> Result<Locale, bool> {
        if locale.is_root() {
            return Err(true);
        }
        let mut locale = locale.clone();
        if locale.has_extensions() {
            if let Some(calendar_type) = locale.get_unicode_locale_type("ca") {
                if !matches!(
                    calendar_type.as_str(),
                    "buddhist" | "japanese" | "gregory" | "islamic" | "roc"
                ) {
                    return Err(false);
                }
            }
            locale = locale.strip_extensions();
        }
        Ok(locale)
    }

    /// `CalendarNameProviderImpl.isSupportedLocale(locale)` of the FALLBACK
    /// adapter, whose language tags are `tags`.
    fn calendar_names_supports(locale: &Locale, tags: &HashSet<String>) -> bool {
        match Self::calendar_names_locale(locale) {
            Err(supported) => supported,
            Ok(locale) => {
                tags.contains(&locale.to_language_tag())
                    || tags.contains(&locale.to_string().replace('_', "-"))
            }
        }
    }

    /// `CLDRCalendarNameProviderImpl.isSupportedLocale(locale)`: the same
    /// calendar-type check, then the CLDR adapter's
    /// `isSupportedProviderLocale(locale, langtags)`, which also accepts a
    /// locale whose equivalent (language alias, `no`/`no_NO` as `nb`) is
    /// available (`tl` as `fil`).
    fn cldr_calendar_names_supports(&self, locale: &Locale) -> bool {
        match Self::calendar_names_locale(locale) {
            Err(supported) => supported,
            Ok(locale) => self.cldr_supports(&locale),
        }
    }

    /// The class key of `locale`: what the JDK's resolution depends on (see
    /// the module documentation).
    pub fn class_key(&self, locale: &Locale) -> String {
        let mut key = String::new();
        for _ in 0..4 {
            key.push(self.adapter(locale, &self.fallback_tags));
        }
        key.push(self.adapter(locale, &self.fallback_zone_tags));
        key.push(';');
        // the calendar-name pool: the providers supporting each lookup locale
        for c in Self::control_candidates(locale) {
            if self.cldr_calendar_names_supports(&c) {
                key.push('C');
            }
            if Self::calendar_names_supports(&c, &self.fallback_tags) {
                key.push('F');
            }
            key.push(',');
        }
        key.push(';');
        let cldr: Vec<String> = self
            .cldr_candidates(locale)
            .into_iter()
            .filter(|c| self.cldr_supports(c))
            .map(|c| c.to_string())
            .collect();
        key.push_str(&cldr.join("|"));
        key.push(';');
        let fallback: Vec<String> = Self::control_candidates(locale)
            .into_iter()
            .filter(|c| Self::fallback_supports(c, &self.fallback_tags))
            .map(|c| c.to_string())
            .collect();
        key.push_str(&fallback.join("|"));
        key
    }

    /// The zone key of `locale`: its class key, then the class key of each
    /// other CLDR candidate.
    pub fn zone_key(&self, locale: &Locale) -> String {
        let mut key = self.class_key(locale);
        for c in self.cldr_candidates(locale).iter().skip(1) {
            key.push_str("||");
            key.push_str(&self.class_key(c));
        }
        key
    }

    /// The data of the class of `locale`.
    ///
    /// # Panics
    ///
    /// When the class is not in the data, which the generator's enumeration
    /// of every reachable class excludes.
    pub fn class_of(&self, locale: &Locale) -> &ClassData {
        let key = self.class_key(locale);
        self.classes
            .get(&key)
            .unwrap_or_else(|| panic!("no JDK 25 locale data for the class of {locale}: {key}"))
    }

    /// The time-zone names of the zone class of `locale`.
    ///
    /// # Panics
    ///
    /// As [`class_of`](Self::class_of).
    pub fn zone_class_of(&self, locale: &Locale) -> ZoneClass {
        let key = self.zone_key(locale);
        *self
            .zone_classes
            .get(&key)
            .unwrap_or_else(|| panic!("no JDK 25 zone data for the class of {locale}: {key}"))
    }
}
