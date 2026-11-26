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
//! The JDK 25 locale data the emulations of `java.text.DateFormat`,
//! `SimpleDateFormat`, `DecimalFormat`, `NumberFormat` and
//! `java.util.Calendar` read, and the resolution of a locale to it.
//!
//! **Divergence.** An emulation of JDK 25, written from the locale-provider
//! sources of OpenJDK 25 (`LocaleProviderAdapter`,
//! `CLDRLocaleProviderAdapter`, `FallbackLocaleProviderAdapter`,
//! `LocaleServiceProviderPool`, `CalendarNameProviderImpl`,
//! `CalendarDataUtility`, `ResourceBundle.Control`,
//! `sun.util.resources.LocaleData`, read as specification) over data
//! `tools/translate/JdkFormatData.java` reads from a JDK 25 through its public
//! API (`src/jdk/format_data.bin`, zlib, inflated on first use). No JDK code
//! is reproduced. It extends the copy of the `rucene-analysis-common`
//! emulation (`crates/rucene-analysis-common/src/jdk/locale_data.rs`), which
//! this crate cannot depend on, with the data formatting and calendar
//! computations read.
//!
//! What a JDK resolves for a locale depends on the locale only through its
//! *class*: the adapter `LocaleProviderAdapter.getAdapter` picks for each of
//! the provider kinds, the providers the calendar-name pool asks per lookup
//! locale, the weeded candidate chain of the CLDR adapter (language aliases
//! and CLDR parent locales applied) and that of the FALLBACK adapter. The
//! time-zone names of a locale also depend on the class of each CLDR
//! candidate of it. The generator enumerated every class reachable from the
//! subtags the JDK knows (and one unseen value of each), read each class's
//! data on a representative locale, and checked on a sample of locales, with
//! and without Unicode extensions, that the JDK's own formats and calendars
//! hold the data their class predicts. [`LocaleData::class_key`] and
//! [`LocaleData::zone_key`] compute the same keys here. Two pieces of data
//! do not depend on the class and are keyed otherwise: the week data
//! (`Calendar.getFirstDayOfWeek`, `getMinimalDaysInFirstWeek`) by region,
//! and the currency symbols, which no number-instance pattern reads and which
//! are not emulated.
//!
//! The JDK computes some time-zone names (the GMT-format fallbacks) from the
//! current instant and caches derived names across locales; the data holds
//! the names a JVM computes when the locale is the first it formats, as of
//! the generation (see `tools/translate/README.md`).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::{LazyLock, OnceLock};

use rucene::error::Result;

use super::locale::Locale;

/// The compressed data.
static DATA_BYTES: &[u8] = include_bytes!("format_data.bin");

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

/// The `DecimalFormatSymbols` of a numbering system and the pattern
/// `NumberFormat.getNumberInstance` builds its `DecimalFormat` from.
#[derive(Debug, Clone)]
pub struct NumberRecord {
    /// `getZeroDigit()`.
    pub zero_digit: u16,
    /// `getGroupingSeparator()`.
    pub grouping_separator: u16,
    /// `getDecimalSeparator()`.
    pub decimal_separator: u16,
    /// `getPerMill()`.
    pub per_mill: u16,
    /// `getPercent()`.
    pub percent: u16,
    /// `getDigit()`.
    pub digit: u16,
    /// `getPatternSeparator()`.
    pub pattern_separator: u16,
    /// `getMinusSign()`.
    pub minus_sign: u16,
    /// `getMonetaryDecimalSeparator()`.
    pub monetary_decimal_separator: u16,
    /// `getMonetaryGroupingSeparator()`.
    pub monetary_grouping_separator: u16,
    /// The package-private `exponential` char.
    pub exponential: u16,
    /// `getInfinity()`.
    pub infinity: String,
    /// `getNaN()`.
    pub nan: String,
    /// `getExponentSeparator()`.
    pub exponent_separator: String,
    /// The package-private `perMillText`.
    pub per_mill_text: String,
    /// The package-private `percentText`.
    pub percent_text: String,
    /// The package-private `minusSignText`.
    pub minus_sign_text: String,
    /// `NumberFormat.getNumberInstance(locale).toPattern()`, which rebuilds
    /// the format (`new DecimalFormat(pattern, symbols)` equals it; the
    /// generator checks it).
    pub pattern: String,
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

/// A display-names map of `Calendar.getDisplayNames`: its names, as UTF-16
/// units, with their values, sorted by name; `None` for Java's `null`.
pub type NameSet = Option<Vec<(Vec<u16>, i32)>>;

/// The data of a locale class.
#[derive(Debug, Clone)]
pub struct ClassData {
    /// Per calendar type, the name-record index of the provider maps.
    pub names: [usize; 3],
    /// The name-record index of the maps `DateFormatSymbols` gives.
    pub symbols: usize,
    /// Per date style (`FULL` .. `SHORT`) and calendar type,
    /// `DateFormat.getDateInstance(style, …)`'s pattern (`None` where no
    /// locale of the class has that calendar type).
    pub patterns: [[Option<String>; 3]; 4],
    /// The default number record.
    pub number: usize,
    /// The number records of the numbering systems (`-u-nu-`) that differ.
    pub nu: HashMap<String, usize>,
    /// The array record of the `DateFormatSymbols` arrays: eras, months,
    /// short months, weekdays, short weekdays, AM/PM strings.
    pub format_symbols: usize,
    /// Per calendar type, the array record of the `Calendar.getDisplayName`
    /// names, one array per [`LocaleData::dcombos`] entry.
    pub display: [usize; 3],
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

/// The JDK locale data.
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
    /// Name sets.
    pub sets: Vec<NameSet>,
    /// The (field, style) pairs a name record holds, in order.
    pub combos: Vec<(i32, i32)>,
    /// Name records: a set index per combo.
    pub name_records: Vec<Vec<usize>>,
    /// Number records.
    pub numbers: Vec<NumberRecord>,
    /// Zone rows, decoded on first use (see [`rows`](Self::rows)).
    rows: OnceLock<Vec<ZoneRow>>,
    /// The inflated data and the offset and length of each string in it,
    /// kept to decode the zone rows on first use.
    raw: Vec<u8>,
    string_spans: Vec<(u32, u32)>,
    /// The offset of the zone-row section in `raw`.
    rows_pos: usize,
    /// Zone tables: row indices.
    pub tables: Vec<Vec<usize>>,
    /// String arrays (`None` entries for Java's `null`).
    pub arrays: Vec<Vec<Option<String>>>,
    /// The (field, style) pairs of the display-name records, in order.
    pub dcombos: Vec<(i32, i32)>,
    /// Array records: array indices.
    pub array_records: Vec<Vec<usize>>,
    /// The week data of a region the table does not hold.
    week_default: (i32, i32),
    /// Region to first day of week and minimal days in the first week.
    week_regions: HashMap<String, (i32, i32)>,
    classes: HashMap<String, ClassData>,
    zone_classes: HashMap<String, ZoneClass>,
}

/// The data, inflated and decoded on first use.
///
/// PERF: the 108 093 zone rows (four UTF-16 names each), which only a
/// pattern with a zone field reads, are decoded on their first use rather
/// than with the rest (rmp #679: the eager decode was ~80 ms per process).
pub fn data() -> &'static LocaleData {
    static DATA: LazyLock<LocaleData> = LazyLock::new(|| {
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(DATA_BYTES)
            .read_to_end(&mut bytes)
            .expect("INVARIANT: the embedded format data inflates");
        LocaleData::decode(bytes)
    });
    &DATA
}

/// `CalendarDataUtility.findRegionOverride(locale)`: `locale` with the
/// region of its `-u-rg-` keyword when the keyword is a region followed by
/// `zzzz`.
///
/// # Errors
///
/// The `IllformedLocaleException` of `Locale.Builder.setLocale` (see
/// [`Locale::with_region`]).
pub fn region_override(locale: &Locale) -> Result<Locale> {
    if let Some(rg) = locale.get_unicode_locale_type("rg") {
        if rg.encode_utf16().count() == 6 {
            let rg = rg.to_ascii_uppercase();
            let b = rg.as_bytes();
            if b[0].is_ascii_uppercase() && b[1].is_ascii_uppercase() && &rg[2..] == "ZZZZ" {
                return locale.with_region(&rg[..2]);
            }
        }
    }
    Ok(locale.clone())
}

/// Whether `Calendar.getDisplayName(s)(field, style, locale)` answers from
/// `DateFormatSymbols` rather than from the calendar-name provider: the
/// short and long format styles (except the short era) of a Gregorian
/// calendar; a Buddhist calendar asks the provider for its eras, and the
/// Japanese calendar for every name.
pub fn from_symbols(calendar_type: usize, field: i32, style: i32) -> bool {
    if calendar_type == 2 || (calendar_type == 1 && field == ERA) {
        return false;
    }
    let format = style == SHORT_FORMAT || style == LONG_FORMAT;
    format && !(field == ERA && style == SHORT_FORMAT) && field != YEAR
}

/// The string at `span` (offset, length) of the inflated data, validated
/// as UTF-8 when the data was decoded.
fn span_str(bytes: &[u8], span: (u32, u32)) -> &str {
    let (offset, len) = (span.0 as usize, span.1 as usize);
    std::str::from_utf8(&bytes[offset..offset + len]).expect("INVARIANT: the strings are UTF-8")
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
    fn decode(raw: Vec<u8>) -> LocaleData {
        let bytes = raw.as_slice();
        assert_eq!(&bytes[..4], b"JFD1", "INVARIANT: the data format");
        let mut r = Reader { bytes, pos: 4 };
        let n = r.v();
        // PERF: the strings are validated once and kept as spans of the
        // inflated data; each use copies only the string it reads.
        let mut string_spans = Vec::with_capacity(n);
        for _ in 0..n {
            let len = r.v();
            std::str::from_utf8(&bytes[r.pos..r.pos + len])
                .expect("INVARIANT: the strings are UTF-8");
            string_spans.push((r.pos as u32, len as u32));
            r.pos += len;
        }
        let string = |i: usize| -> &str { span_str(bytes, string_spans[i]) };
        let st = |r: &mut Reader<'_>| string(r.v()).to_string();
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
                sets.push(Some(
                    (0..m)
                        .map(|_| (u16s(string(r.v())), r.v() as i32))
                        .collect(),
                ));
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
            let mut c = [0u16; 11];
            for x in c.iter_mut() {
                *x = r.v() as u16;
            }
            let mut s = || st(&mut r);
            numbers.push(NumberRecord {
                zero_digit: c[0],
                grouping_separator: c[1],
                decimal_separator: c[2],
                per_mill: c[3],
                percent: c[4],
                digit: c[5],
                pattern_separator: c[6],
                minus_sign: c[7],
                monetary_decimal_separator: c[8],
                monetary_grouping_separator: c[9],
                exponential: c[10],
                infinity: s(),
                nan: s(),
                exponent_separator: s(),
                per_mill_text: s(),
                percent_text: s(),
                minus_sign_text: s(),
                pattern: s(),
            });
        }
        // The zone rows are skipped here and decoded by `rows()`.
        let rows_pos = r.pos;
        let n = r.v();
        for _ in 0..n * 7 {
            r.v();
        }
        let n = r.v();
        let tables = (0..n)
            .map(|_| {
                let m = r.v();
                (0..m).map(|_| r.v()).collect()
            })
            .collect();
        let n = r.v();
        let arrays = (0..n)
            .map(|_| {
                let m = r.v();
                (0..m)
                    .map(|_| {
                        let id = r.v();
                        if id == 0 {
                            None
                        } else {
                            Some(string(id - 1).to_string())
                        }
                    })
                    .collect()
            })
            .collect();
        let n = r.v();
        let dcombos: Vec<(i32, i32)> = (0..n).map(|_| (r.v() as i32, r.v() as i32)).collect();
        let n = r.v();
        let array_records = (0..n)
            .map(|_| {
                let m = r.v();
                (0..m).map(|_| r.v()).collect()
            })
            .collect();
        let week_default = (r.v() as i32, r.v() as i32);
        let n = r.v();
        let week_regions = (0..n)
            .map(|_| (st(&mut r), (r.v() as i32, r.v() as i32)))
            .collect();
        let n = r.v();
        let mut classes = HashMap::with_capacity(n);
        for _ in 0..n {
            let key = st(&mut r);
            let names = [r.v(), r.v(), r.v()];
            let symbols = r.v();
            let mut patterns: [[Option<String>; 3]; 4] = Default::default();
            for style in &mut patterns {
                for p in style.iter_mut() {
                    let id = r.v();
                    *p = if id == 0 {
                        None
                    } else {
                        Some(string(id - 1).to_string())
                    };
                }
            }
            let number = r.v();
            let m = r.v();
            let nu = (0..m).map(|_| (st(&mut r), r.v())).collect();
            let format_symbols = r.v();
            let display = [r.v(), r.v(), r.v()];
            classes.insert(
                key,
                ClassData {
                    names,
                    symbols,
                    patterns,
                    number,
                    nu,
                    format_symbols,
                    display,
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
            rows: OnceLock::new(),
            raw,
            string_spans,
            rows_pos,
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
            tables,
            arrays,
            dcombos,
            array_records,
            week_default,
            week_regions,
            classes,
            zone_classes,
        }
    }

    /// Zone rows (`DateFormatSymbols.getZoneStrings()` rows), decoded from
    /// the inflated data on first use.
    pub fn rows(&self) -> &[ZoneRow] {
        self.rows.get_or_init(|| {
            let bytes = self.raw.as_slice();
            let string = |i: usize| span_str(bytes, self.string_spans[i]);
            let u16s = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
            let mut r = Reader {
                bytes,
                pos: self.rows_pos,
            };
            let n = r.v();
            let mut rows = Vec::with_capacity(n);
            for _ in 0..n {
                let id = string(r.v()).to_string();
                let resolved_id = string(r.v()).to_string();
                let names = [
                    u16s(string(r.v())),
                    u16s(string(r.v())),
                    u16s(string(r.v())),
                    u16s(string(r.v())),
                ];
                let digit_mask = r.v() as u8;
                rows.push(ZoneRow {
                    id,
                    resolved_id,
                    names,
                    digit_mask,
                });
            }
            rows
        })
    }

    /// The week data of `region`: `(firstDayOfWeek,
    /// minimalDaysInFirstWeek)` as `CLDRCalendarDataProviderImpl` gives it.
    pub fn week_data(&self, region: &str) -> (i32, i32) {
        self.week_regions
            .get(region)
            .copied()
            .unwrap_or(self.week_default)
    }

    /// The `DateFormatSymbols` arrays of `locale` (`DateFormatSymbols.
    /// getInstance(locale)`, which reads the region override): eras, months,
    /// short months, weekdays, short weekdays, AM/PM strings.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the region override.
    pub fn format_symbols(&self, locale: &Locale) -> Result<[&[Option<String>]; 6]> {
        let symbols_locale = region_override(locale)?;
        let record = &self.array_records[self.class_of(&symbols_locale).format_symbols];
        Ok(std::array::from_fn(|i| self.arrays[record[i]].as_slice()))
    }

    /// `Calendar.getDisplayName(field, style, locale)` of a calendar of type
    /// `calendar_type` (an index into [`CAL_TYPES`]) whose `field` is
    /// `value`, for the (field, style) pairs of [`dcombos`](Self::dcombos);
    /// `None` for Java's `null` and for any other pair.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the region override
    /// `DateFormatSymbols` reads.
    pub fn display_name(
        &self,
        calendar_type: usize,
        field: i32,
        style: i32,
        value: i32,
        locale: &Locale,
    ) -> Result<Option<String>> {
        if from_symbols(calendar_type, field, style) {
            let symbols = self.format_symbols(locale)?;
            let long = (style & LONG_FORMAT) != 0;
            let strings = match field {
                ERA => symbols[0],
                MONTH => {
                    if long {
                        symbols[1]
                    } else {
                        symbols[2]
                    }
                }
                DAY_OF_WEEK => {
                    if long {
                        symbols[3]
                    } else {
                        symbols[4]
                    }
                }
                AM_PM => symbols[5],
                _ => return Ok(None),
            };
            return Ok(usize::try_from(value)
                .ok()
                .and_then(|v| strings.get(v))
                .cloned()
                .flatten());
        }
        let Some(index) = self.dcombos.iter().position(|&c| c == (field, style)) else {
            return Ok(None);
        };
        let record = &self.array_records[self.class_of(locale).display[calendar_type]];
        let names = &self.arrays[record[index]];
        Ok(usize::try_from(value)
            .ok()
            .and_then(|v| names.get(v))
            .cloned()
            .flatten())
    }

    /// The number record `NumberFormat.getNumberInstance(locale)` and
    /// `getIntegerInstance(locale)` build their format from: that of the
    /// region override unless `-u-nu-` is given, in the numbering system of
    /// `-u-nu-` when the class knows it.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the region override.
    pub fn number_record(&self, locale: &Locale) -> Result<&NumberRecord> {
        let number_locale = if locale.get_unicode_locale_type("nu").is_some() {
            locale.clone()
        } else {
            region_override(locale)?
        };
        let class = self.class_of(&number_locale);
        let index = number_locale
            .get_unicode_locale_type("nu")
            .and_then(|nu| class.nu.get(&nu).copied())
            .unwrap_or(class.number);
        Ok(&self.numbers[index])
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
    fn calendar_names_locale(locale: &Locale) -> std::result::Result<Locale, bool> {
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
