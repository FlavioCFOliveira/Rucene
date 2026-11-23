// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LikelySubtags`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{self, ICUResourceBundle, OpenType};
use crate::r#impl::icu_resource_bundle_reader::ReaderValue;
use crate::r#impl::locale::lsr::LSR;
use crate::r#impl::u_resource::{Table, Value};
use crate::util::bytes_trie::{BytesTrie, Result as TrieResult};
use crate::util::region::{Region, RegionType};
use crate::util::u_locale::{Minimize, ULocale};
use crate::util::u_resource_bundle::missing_resource;

const PSEUDO_ACCENTS_PREFIX: &str = "'"; // -XA, -PSACCENT
const PSEUDO_BIDI_PREFIX: &str = "+"; // -XB, -PSBIDI
const PSEUDO_CRACKED_PREFIX: &str = ","; // -XC, -PSCRACK

/// The likely-subtags data of `langInfo.res`.
///
/// Equivalent to `LikelySubtags.Data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    /// `languageAliases`.
    pub language_aliases: HashMap<String, String>,
    /// `regionAliases`.
    pub region_aliases: HashMap<String, String>,
    /// `trie`.
    pub trie: Arc<[i8]>,
    /// `lsrs`.
    pub lsrs: Vec<LSR>,
}

impl Data {
    /// `new Data(...)`.
    pub fn new(
        language_aliases: HashMap<String, String>,
        region_aliases: HashMap<String, String>,
        trie: Arc<[i8]>,
        lsrs: Vec<LSR>,
    ) -> Self {
        Self {
            language_aliases,
            region_aliases,
            trie,
            lsrs,
        }
    }

    fn get_value<'v>(table: &dyn Table, key: &str, value: &'v mut ReaderValue) -> Result<&'v mut ReaderValue> {
        let k: Vec<u16> = key.encode_utf16().collect();
        if !table.find_value(&k, value) {
            return Err(missing_resource("langInfo.res missing data", "", format!("likely/{key}")));
        }
        Ok(value)
    }

    fn pairs(value: &ReaderValue) -> Result<HashMap<String, String>> {
        let pairs = value.get_string_array()?;
        let mut m = HashMap::with_capacity(pairs.len() / 2);
        let mut i = 0;
        while i < pairs.len() {
            m.insert(String::from_utf16_lossy(&pairs[i]), String::from_utf16_lossy(&pairs[i + 1]));
            i += 2;
        }
        Ok(m)
    }

    /// `load()`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data is missing.
    pub fn load() -> Result<Data> {
        let lang_info = ICUResourceBundle::get_bundle_instance_open(
            Some(ICUData::ICU_BASE_NAME),
            "langInfo",
            icu_resource_bundle::icu_data_class_loader(),
            OpenType::Direct,
        )?;
        let mut value = lang_info.get_value_with_fallback("likely")?;
        let likely_table = value.get_table()?;

        let la: Vec<u16> = "languageAliases".encode_utf16().collect();
        let language_aliases = if likely_table.find_value(&la, &mut value) {
            Self::pairs(&value)?
        } else {
            HashMap::new()
        };

        let ra: Vec<u16> = "regionAliases".encode_utf16().collect();
        let region_aliases = if likely_table.find_value(&ra, &mut value) {
            Self::pairs(&value)?
        } else {
            HashMap::new()
        };

        let mut buffer = Self::get_value(likely_table.as_ref(), "trie", &mut value)?.get_binary()?;
        let mut trie = vec![0u8; buffer.remaining() as usize];
        buffer.get_bytes(&mut trie)?;
        let trie: Vec<i8> = trie.into_iter().map(|b| b as i8).collect();
        let m49: Vec<String> = Self::get_value(likely_table.as_ref(), "m49", &mut value)?
            .get_string_array()?
            .into_iter()
            .map(|s| String::from_utf16_lossy(&s))
            .collect();
        let nums = Self::get_value(likely_table.as_ref(), "lsrnum", &mut value)?.get_int_vector()?;
        let lsrs = LSR::decode_ints(&nums, &m49)?;
        Ok(Data::new(language_aliases, region_aliases, Arc::from(trie), lsrs))
    }
}

/// Maximizes and minimizes locales with the CLDR likely-subtags data.
///
/// Equivalent to `com.ibm.icu.impl.locale.LikelySubtags`.
#[derive(Debug)]
pub struct LikelySubtags {
    language_aliases: HashMap<String, String>,
    region_aliases: HashMap<String, String>,
    trie: BytesTrie,
    trie_und_state: i64,
    trie_und_zzzz_state: i64,
    default_lsr_index: i32,
    trie_first_letter_states: [i64; 26],
    lsrs: Vec<LSR>,
}

/// `INSTANCE`, or the message of its initializer's exception.
static INSTANCE: LazyLock<std::result::Result<Arc<LikelySubtags>, String>> =
    LazyLock::new(|| Data::load().map(|d| Arc::new(LikelySubtags::new(d))).map_err(|e| e.to_string()));

impl LikelySubtags {
    /// `SKIP_SCRIPT`.
    pub const SKIP_SCRIPT: i32 = 1;

    /// `INSTANCE`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data cannot be loaded (Java's
    /// `ExceptionInInitializerError`).
    pub fn instance() -> Result<Arc<LikelySubtags>> {
        match &*INSTANCE {
            Ok(i) => Ok(i.clone()),
            Err(m) => Err(missing_resource(m.clone(), "", "")),
        }
    }

    fn new(data: Data) -> Self {
        let mut trie = BytesTrie::new(data.trie.clone(), 0);
        // Cache the result of looking up language="und" encoded as "*", and "und-Zzzz" ("**").
        let result = trie.next('*' as i32);
        debug_assert!(result.has_next());
        let trie_und_state = trie.get_state64();
        let result = trie.next('*' as i32);
        debug_assert!(result.has_next());
        let trie_und_zzzz_state = trie.get_state64();
        let result = trie.next('*' as i32);
        debug_assert!(result.has_value());
        let default_lsr_index = trie.get_value();
        trie.reset();

        let mut trie_first_letter_states = [0i64; 26];
        for c in b'a'..=b'z' {
            let result = trie.next(c as i32);
            if result == TrieResult::NoValue {
                trie_first_letter_states[(c - b'a') as usize] = trie.get_state64();
            }
            trie.reset();
        }
        Self {
            language_aliases: data.language_aliases,
            region_aliases: data.region_aliases,
            trie,
            trie_und_state,
            trie_und_zzzz_state,
            default_lsr_index,
            trie_first_letter_states,
            lsrs: data.lsrs,
        }
    }

    /// `canonicalize(ULocale locale)`: the locale with the language and
    /// region aliases applied.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale`.
    pub fn canonicalize(&self, locale: &ULocale) -> Result<ULocale> {
        let lang = locale.get_language();
        let lang2 = self.language_aliases.get(&lang);
        let region = locale.get_country();
        let region2 = self.region_aliases.get(&region);
        if lang2.is_some() || region2.is_some() {
            return ULocale::new3(
                Some(lang2.unwrap_or(&lang)),
                Some(&locale.get_script()),
                Some(region2.unwrap_or(&region)),
            );
        }
        Ok(locale.clone())
    }

    fn get_canonical(aliases: &HashMap<String, String>, alias: &str) -> String {
        aliases.get(alias).cloned().unwrap_or_else(|| alias.to_string())
    }

    /// `makeMaximizedLsrFrom(ULocale locale, boolean returnInputIfUnmatch)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the region data.
    pub fn make_maximized_lsr_from(&self, locale: &ULocale, return_input_if_unmatch: bool) -> Result<LSR> {
        let name = locale.get_name(); // Faster than .toLanguageTag().
        if name.starts_with("@x=") {
            let tag = locale.to_language_tag();
            debug_assert!(tag.starts_with("und-x-"));
            // Private use language tag x-subtag-subtag... which CLDR changes to
            // und-x-subtag-subtag...
            return Ok(LSR::new(&tag, "", "", LSR::EXPLICIT_LSR));
        }
        let max = self.make_maximized_lsr(
            &locale.get_language(),
            &locale.get_script(),
            &locale.get_country(),
            &locale.get_variant(),
            return_input_if_unmatch,
        )?;
        if max.language.is_empty() && max.script.is_empty() && max.region.is_empty() {
            return Ok(LSR::new(
                &locale.get_language(),
                &locale.get_script(),
                &locale.get_country(),
                LSR::EXPLICIT_LSR,
            ));
        }
        Ok(max)
    }

    /// `makeMaximizedLsrFrom(Locale locale)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the region data.
    pub fn make_maximized_lsr_from_locale(&self, locale: &Locale) -> Result<LSR> {
        let tag = locale.to_language_tag();
        if tag.starts_with("x-") || tag.starts_with("und-x-") {
            // Private use language tag x-subtag-subtag... which CLDR changes to
            // und-x-subtag-subtag...
            return Ok(LSR::new(&tag, "", "", LSR::EXPLICIT_LSR));
        }
        self.make_maximized_lsr(
            locale.get_language(),
            locale.get_script(),
            locale.get_country(),
            locale.get_variant(),
            false,
        )
    }

    fn make_maximized_lsr(
        &self,
        language: &str,
        script: &str,
        region: &str,
        variant: &str,
        return_input_if_unmatch: bool,
    ) -> Result<LSR> {
        // Handle pseudolocales like en-XA, ar-XB, fr-PSCRACK.
        // They should match only themselves,
        // not other locales with what looks like the same language and script subtags.
        if !return_input_if_unmatch {
            let rb = region.as_bytes();
            if region.len() == 2 && rb[0] == b'X' {
                match rb[1] {
                    b'A' => {
                        return Ok(LSR::new(
                            &format!("{PSEUDO_ACCENTS_PREFIX}{language}"),
                            &format!("{PSEUDO_ACCENTS_PREFIX}{script}"),
                            region,
                            LSR::EXPLICIT_LSR,
                        ))
                    }
                    b'B' => {
                        return Ok(LSR::new(
                            &format!("{PSEUDO_BIDI_PREFIX}{language}"),
                            &format!("{PSEUDO_BIDI_PREFIX}{script}"),
                            region,
                            LSR::EXPLICIT_LSR,
                        ))
                    }
                    b'C' => {
                        return Ok(LSR::new(
                            &format!("{PSEUDO_CRACKED_PREFIX}{language}"),
                            &format!("{PSEUDO_CRACKED_PREFIX}{script}"),
                            region,
                            LSR::EXPLICIT_LSR,
                        ))
                    }
                    _ => {} // normal locale
                }
            }

            if variant.starts_with("PS") {
                let lsr_flags = if region.is_empty() {
                    LSR::EXPLICIT_LANGUAGE | LSR::EXPLICIT_SCRIPT
                } else {
                    LSR::EXPLICIT_LSR
                };
                let pick = |p: &str, d: &str| {
                    LSR::new(
                        &format!("{p}{language}"),
                        &format!("{p}{script}"),
                        if region.is_empty() { d } else { region },
                        lsr_flags,
                    )
                };
                match variant {
                    "PSACCENT" => return Ok(pick(PSEUDO_ACCENTS_PREFIX, "XA")),
                    "PSBIDI" => return Ok(pick(PSEUDO_BIDI_PREFIX, "XB")),
                    "PSCRACK" => return Ok(pick(PSEUDO_CRACKED_PREFIX, "XC")),
                    _ => {} // normal locale
                }
            }
        }

        let language = Self::get_canonical(&self.language_aliases, language);
        // (We have no script mappings.)
        let region = Self::get_canonical(&self.region_aliases, region);
        self.maximize(&language, script, &region, return_input_if_unmatch)
    }

    fn is_macroregion(region: &str) -> Result<bool> {
        let ty = Region::get_instance(region)?.get_type();
        Ok(ty == RegionType::World || ty == RegionType::Continent || ty == RegionType::Subcontinent)
    }

    fn first_letter_value(&self, iter: &mut BytesTrie, language: &str) -> i32 {
        let lb = language.encode_utf16().collect::<Vec<u16>>();
        if lb.len() >= 2 {
            let c0 = lb[0] as i32 - 'a' as i32;
            if (0..=25).contains(&c0) {
                let state = self.trie_first_letter_states[c0 as usize];
                if state != 0 {
                    return Self::trie_next(iter.reset_to_state64(state), &lb, 1);
                }
            }
        }
        Self::trie_next(iter, &lb, 0)
    }

    fn maximize(&self, language: &str, script: &str, region: &str, return_input_if_unmatch: bool) -> Result<LSR> {
        let mut language = if language == "und" { "" } else { language }.to_string();
        let mut script = if script == "Zzzz" { "" } else { script }.to_string();
        let mut region = if region == "ZZ" { "" } else { region }.to_string();
        if !script.is_empty() && !region.is_empty() && !language.is_empty() {
            return Ok(LSR::new(&language, &script, &region, LSR::EXPLICIT_LSR)); // already maximized
        }
        let retain_language;
        let retain_script;
        let mut retain_region = false;
        let mut iter = BytesTrie::from_other(&self.trie);
        let mut state: i64;
        let mut value = self.first_letter_value(&mut iter, &language);
        let match_language = value >= 0;
        let mut match_script = false;
        if value >= 0 {
            retain_language = !language.is_empty();
            state = iter.get_state64();
        } else {
            retain_language = true;
            iter.reset_to_state64(self.trie_und_state); // "und" ("*")
            state = 0;
        }

        if value >= 0 && !script.is_empty() {
            match_script = true;
        }
        if value > 0 {
            // Intermediate or final value from just language.
            if value == Self::SKIP_SCRIPT {
                value = 0;
            }
            retain_script = !script.is_empty();
        } else {
            let su: Vec<u16> = script.encode_utf16().collect();
            value = Self::trie_next(&mut iter, &su, 0);
            if value >= 0 {
                retain_script = !script.is_empty();
                state = iter.get_state64();
            } else {
                retain_script = true;
                if state == 0 {
                    iter.reset_to_state64(self.trie_und_zzzz_state); // "und-Zzzz" ("**")
                } else {
                    iter.reset_to_state64(state);
                    value = Self::trie_next(&mut iter, &[], 0);
                    debug_assert!(value >= 0);
                    state = iter.get_state64();
                }
            }
        }

        let mut match_region = false;
        if value > 0 {
            // Final value from just language or language+script.
            retain_region = !region.is_empty();
        } else {
            let ru: Vec<u16> = region.encode_utf16().collect();
            value = Self::trie_next(&mut iter, &ru, 0);
            if value >= 0 {
                if !region.is_empty() && !Self::is_macroregion(&region)? {
                    retain_region = true;
                    match_region = true;
                }
            } else {
                retain_region = true;
                if state == 0 {
                    value = self.default_lsr_index;
                } else {
                    iter.reset_to_state64(state);
                    value = Self::trie_next(&mut iter, &[], 0);
                    debug_assert!(value > 0);
                }
            }
        }
        let result = &self.lsrs[value as usize];

        if return_input_if_unmatch && !(match_language || match_script || (match_region && language.is_empty())) {
            return Ok(LSR::new("", "", "", LSR::EXPLICIT_LSR)); // no matching.
        }
        if language.is_empty() {
            language = "und".to_string();
        }

        if !(retain_language || retain_script || retain_region) {
            debug_assert!(result.flags == LSR::IMPLICIT_LSR);
            return Ok(result.clone());
        }
        if !retain_language {
            language = result.language.clone();
        }
        if !retain_script {
            script = result.script.clone();
        }
        if !retain_region {
            region = result.region.clone();
        }
        let retain_mask = (if retain_language { 4 } else { 0 })
            + (if retain_script { 2 } else { 0 })
            + (if retain_region { 1 } else { 0 });
        // retainOldMask flags = LSR explicit-subtag flags
        Ok(LSR::new(&language, &script, &region, retain_mask))
    }

    /// `compareLikely(LSR lsr, LSR other, int likelyInfo)`
    /// (package-private): whether `lsr` is the likely one, with cached
    /// lookup information.
    pub fn compare_likely(&self, lsr: &LSR, other: &LSR, mut likely_info: i32) -> i32 {
        // If likelyInfo >= 0:
        // likelyInfo bit 1 is set if the previous comparison with lsr
        // was for equal language and script.
        // Otherwise the scripts differed.
        if lsr.language != other.language {
            return 0xfffffffcu32 as i32; // negative, lsr not better than other
        }
        if lsr.script != other.script {
            let index;
            if likely_info >= 0 && (likely_info & 2) == 0 {
                index = likely_info >> 2;
            } else {
                index = self.get_likely_index(&lsr.language, "");
                likely_info = index << 2;
            }
            let likely = &self.lsrs[index as usize];
            return if lsr.script == likely.script {
                likely_info | 1
            } else {
                likely_info & !1
            };
        }
        if lsr.region != other.region {
            let index;
            if likely_info >= 0 && (likely_info & 2) != 0 {
                index = likely_info >> 2;
            } else {
                index = self.get_likely_index(&lsr.language, &lsr.region);
                likely_info = (index << 2) | 2;
            }
            let likely = &self.lsrs[index as usize];
            return if lsr.region == likely.region {
                likely_info | 1
            } else {
                likely_info & !1
            };
        }
        likely_info & !1 // lsr not better than other
    }

    // Subset of maximize().
    /// `getLikelyIndex(String language, String script)` (private).
    pub fn get_likely_index(&self, language: &str, script: &str) -> i32 {
        let language = if language == "und" { "" } else { language };
        let script = if script == "Zzzz" { "" } else { script };

        let mut iter = BytesTrie::from_other(&self.trie);
        let mut state: i64;
        let mut value = self.first_letter_value(&mut iter, language);
        if value >= 0 {
            state = iter.get_state64();
        } else {
            iter.reset_to_state64(self.trie_und_state); // "und" ("*")
            state = 0;
        }

        if value > 0 {
            // Intermediate or final value from just language.
            if value == Self::SKIP_SCRIPT {
                value = 0;
            }
        } else {
            let su: Vec<u16> = script.encode_utf16().collect();
            value = Self::trie_next(&mut iter, &su, 0);
            if value >= 0 {
                state = iter.get_state64();
            } else if state == 0 {
                iter.reset_to_state64(self.trie_und_zzzz_state); // "und-Zzzz" ("**")
            } else {
                iter.reset_to_state64(state);
                value = Self::trie_next(&mut iter, &[], 0);
                debug_assert!(value >= 0);
                let _ = iter.get_state64();
            }
        }
        let _ = state;

        if value > 0 {
            // Final value from just language or language+script.
        } else {
            value = Self::trie_next(&mut iter, &[], 0);
            debug_assert!(value > 0);
        }
        value
    }

    fn trie_next(iter: &mut BytesTrie, s: &[u16], mut i: usize) -> i32 {
        let result;
        if s.is_empty() {
            result = iter.next('*' as i32);
        } else {
            let end = s.len() - 1;
            loop {
                let c = s[i] as i32;
                if i < end {
                    if !iter.next(c).has_next() {
                        return -1;
                    }
                } else {
                    // last character of this subtag
                    result = iter.next(c | 0x80);
                    break;
                }
                i += 1;
            }
        }
        match result {
            TrieResult::NoMatch => -1,
            TrieResult::NoValue => 0,
            TrieResult::IntermediateValue => {
                debug_assert!(iter.get_value() == Self::SKIP_SCRIPT);
                Self::SKIP_SCRIPT
            }
            TrieResult::FinalValue => iter.get_value(),
        }
    }

    /// `minimizeSubtags(String languageIn, String scriptIn, String
    /// regionIn, ULocale.Minimize fieldToFavor)`.
    ///
    /// # Errors
    ///
    /// The errors of loading the region data.
    pub fn minimize_subtags(&self, language_in: &str, script_in: &str, region_in: &str, field_to_favor: Minimize) -> Result<LSR> {
        let max = self.maximize(language_in, script_in, region_in, true)?;
        if max.language.is_empty() && max.region.is_empty() && max.script.is_empty() {
            // Cannot match, return as is
            return Ok(LSR::new(language_in, script_in, region_in, LSR::EXPLICIT_LSR));
        }
        let test = self.maximize(&max.language, "", "", true)?;
        if test.is_equivalent_to(&max) {
            return Ok(LSR::new(&max.language, "", "", LSR::DONT_CARE_FLAGS));
        }

        if Minimize::FavorRegion == field_to_favor {
            let test = self.maximize(&max.language, "", &max.region, true)?;
            if test.is_equivalent_to(&max) {
                return Ok(LSR::new(&max.language, "", &max.region, LSR::DONT_CARE_FLAGS));
            }
            let test = self.maximize(&max.language, &max.script, "", true)?;
            if test.is_equivalent_to(&max) {
                return Ok(LSR::new(&max.language, &max.script, "", LSR::DONT_CARE_FLAGS));
            }
        } else {
            let test = self.maximize(&max.language, &max.script, "", true)?;
            if test.is_equivalent_to(&max) {
                return Ok(LSR::new(&max.language, &max.script, "", LSR::DONT_CARE_FLAGS));
            }
            let test = self.maximize(&max.language, "", &max.region, true)?;
            if test.is_equivalent_to(&max) {
                return Ok(LSR::new(&max.language, "", &max.region, LSR::DONT_CARE_FLAGS));
            }
        }
        Ok(LSR::new(&max.language, &max.script, &max.region, LSR::DONT_CARE_FLAGS))
    }

    /// `getTable()` (private): the trie's strings and their LSRs, sorted.
    fn get_table(&self) -> BTreeMap<String, LSR> {
        let mut map = BTreeMap::new();
        for entry in self.trie.iterator() {
            let mut sb = String::new();
            let length = entry.bytes_length();
            let mut i = 0;
            while i < length {
                let b = entry.byte_at(i);
                i += 1;
                if b == b'*' as i8 {
                    sb.push_str("*-");
                } else if b >= 0 {
                    sb.push(b as u8 as char);
                } else {
                    // end of subtag
                    sb.push((b as u8 & 0x7f) as char);
                    sb.push('-');
                }
            }
            debug_assert!(sb.ends_with('-'));
            sb.pop();
            map.insert(sb, self.lsrs[entry.value as usize].clone());
        }
        map
    }
}

/// `toString()`: the table, as `TreeMap.toString()` renders it
/// (`{key=value, ...}`).
impl fmt::Display for LikelySubtags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let table = self.get_table();
        f.write_str("{")?;
        let mut first = true;
        for (k, v) in &table {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            write!(f, "{k}={v}")?;
        }
        f.write_str("}")
    }
}
