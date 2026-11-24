// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.LocaleDistance`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::error::Result;

use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::{self, ICUResourceBundle, OpenType};
use crate::r#impl::icu_resource_bundle_reader::ReaderValue;
use crate::r#impl::locale::likely_subtags::LikelySubtags;
use crate::r#impl::locale::lsr::LSR;
use crate::r#impl::u_resource::{Table, Value};
use crate::util::bytes_trie::{BytesTrie, Result as TrieResult};
use crate::util::locale_matcher::{Direction, FavorSubtag};
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::missing_resource;

/// `END_OF_SUBTAG`: bit flag used on the last character of a subtag in
/// the trie. Must be set consistently by the builder and the lookup code.
pub const END_OF_SUBTAG: i32 = 0x80;
/// `DISTANCE_SKIP_SCRIPT`: distance value bit flag, set by the builder.
pub const DISTANCE_SKIP_SCRIPT: i32 = 0x80;
/// Distance value bit flag, set by trieNext().
const DISTANCE_IS_FINAL: i32 = 0x100;
const DISTANCE_IS_FINAL_OR_SKIP_SCRIPT: i32 = DISTANCE_IS_FINAL | DISTANCE_SKIP_SCRIPT;
// The distance is shifted left to gain some fraction bits.
const DISTANCE_SHIFT: i32 = 3;
const DISTANCE_FRACTION_MASK: i32 = 7;
// 7 bits for 0..100
const DISTANCE_INT_SHIFT: i32 = 7;
const INDEX_SHIFT: i32 = DISTANCE_INT_SHIFT + DISTANCE_SHIFT;
const DISTANCE_MASK: i32 = 0x3ff;
// vate static final int MAX_INDEX = 0x1fffff;  // avoids sign bit
const INDEX_NEG_1: i32 = 0xfffffc00u32 as i32;

/// `IX_DEF_LANG_DISTANCE`: index into the array of distances.
pub const IX_DEF_LANG_DISTANCE: usize = 0;
/// `IX_DEF_SCRIPT_DISTANCE`.
pub const IX_DEF_SCRIPT_DISTANCE: usize = 1;
/// `IX_DEF_REGION_DISTANCE`.
pub const IX_DEF_REGION_DISTANCE: usize = 2;
/// `IX_MIN_REGION_DISTANCE`.
pub const IX_MIN_REGION_DISTANCE: usize = 3;
/// `IX_LIMIT`.
pub const IX_LIMIT: usize = 4;
const ABOVE_THRESHOLD: i32 = 100;

/// Offline-built data for LocaleMatcher, mostly but not only the
/// data for mapping locales to their maximized forms.
///
/// Equivalent to `LocaleDistance.Data` (VisibleForTesting). `byte[]`
/// arrays are `Vec<i8>`; the `String[]` partition arrays keep their UTF-16
/// units; the `LinkedHashSet<LSR>` of paradigm LSRs is a vector of unique
/// LSRs in insertion order.
#[derive(Debug, Clone)]
pub struct Data {
    /// `trie`.
    pub trie: Vec<i8>,
    /// `regionToPartitionsIndex`.
    pub region_to_partitions_index: Vec<i8>,
    /// `partitionArrays`.
    pub partition_arrays: Vec<Vec<u16>>,
    /// `paradigmLSRs`.
    pub paradigm_lsrs: Vec<LSR>,
    /// `distances`.
    pub distances: Vec<i32>,
}

impl Data {
    /// `new Data(byte[] trie, byte[] regionToPartitionsIndex, String[]
    /// partitionArrays, Set<LSR> paradigmLSRs, int[] distances)`.
    pub fn new(
        trie: Vec<i8>,
        region_to_partitions_index: Vec<i8>,
        partition_arrays: Vec<Vec<u16>>,
        paradigm_lsrs: Vec<LSR>,
        distances: Vec<i32>,
    ) -> Self {
        Self {
            trie,
            region_to_partitions_index,
            partition_arrays,
            paradigm_lsrs,
            distances,
        }
    }

    fn get_value<'v>(
        table: &dyn Table,
        key: &str,
        value: &'v mut ReaderValue,
    ) -> Result<&'v mut ReaderValue> {
        let k: Vec<u16> = key.encode_utf16().collect();
        if !table.find_value(&k, value) {
            return Err(missing_resource(
                "langInfo.res missing data",
                "",
                format!("match/{key}"),
            ));
        }
        Ok(value)
    }

    fn binary(value: &ReaderValue) -> Result<Vec<i8>> {
        let mut buffer = value.get_binary()?;
        let mut bytes = vec![0u8; buffer.remaining() as usize];
        buffer.get_bytes(&mut bytes)?;
        Ok(bytes.into_iter().map(|b| b as i8).collect())
    }

    /// `load()` (VisibleForTesting): the `match` data of `langInfo.res`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data is missing or too short.
    pub fn load() -> Result<Data> {
        let lang_info = ICUResourceBundle::get_bundle_instance_open(
            Some(ICUData::ICU_BASE_NAME),
            "langInfo",
            icu_resource_bundle::icu_data_class_loader(),
            OpenType::Direct,
        )?;
        let mut value = lang_info.get_value_with_fallback("match")?;
        let match_table = value.get_table()?;

        let trie = Self::binary(Self::get_value(match_table.as_ref(), "trie", &mut value)?)?;
        let region_to_partitions = Self::binary(Self::get_value(
            match_table.as_ref(),
            "regionToPartitions",
            &mut value,
        )?)?;
        if (region_to_partitions.len() as i32) < LSR::REGION_INDEX_LIMIT {
            return Err(missing_resource(
                "langInfo.res binary data too short",
                "",
                "match/regionToPartitions",
            ));
        }

        let partitions =
            Self::get_value(match_table.as_ref(), "partitions", &mut value)?.get_string_array()?;

        let paradigmnum: Vec<u16> = "paradigmnum".encode_utf16().collect();
        let paradigm_lsrs = if match_table.find_value(&paradigmnum, &mut value) {
            let mut likely_value = lang_info.get_value_with_fallback("likely")?;
            let likely_table = likely_value.get_table()?;
            let m49: Vec<String> =
                Self::get_value(likely_table.as_ref(), "m49", &mut likely_value)?
                    .get_string_array()?
                    .into_iter()
                    .map(|s| String::from_utf16_lossy(&s))
                    .collect();
            let nums = Self::get_value(match_table.as_ref(), "paradigmnum", &mut value)?
                .get_int_vector()?;
            let paradigms = LSR::decode_ints(&nums, &m49)?;
            // LinkedHashSet for stable order; otherwise a unit test is flaky.
            let mut set: Vec<LSR> = Vec::with_capacity(paradigms.len());
            for p in paradigms {
                if !set.contains(&p) {
                    set.push(p);
                }
            }
            set
        } else {
            Vec::new()
        };

        let distances =
            Self::get_value(match_table.as_ref(), "distances", &mut value)?.get_int_vector()?;
        if distances.len() < IX_LIMIT {
            return Err(missing_resource(
                "langInfo.res intvector too short",
                "",
                "match/distances",
            ));
        }

        Ok(Data::new(
            trie,
            region_to_partitions,
            partitions,
            paradigm_lsrs,
            distances,
        ))
    }
}

/// `equals(Object other)`: equal arrays, equal paradigm sets (in any
/// order). (`hashCode()` is the constant 1, "unused; silence ErrorProne".)
impl PartialEq for Data {
    fn eq(&self, od: &Self) -> bool {
        self.trie == od.trie
            && self.region_to_partitions_index == od.region_to_partitions_index
            && self.partition_arrays == od.partition_arrays
            && self.paradigm_lsrs.len() == od.paradigm_lsrs.len()
            && self
                .paradigm_lsrs
                .iter()
                .all(|l| od.paradigm_lsrs.contains(l))
            && self.distances == od.distances
    }
}

/// Offline-built data for LocaleMatcher: the trie of language, script and
/// region distances (CLDR `languageMatching`).
///
/// Equivalent to `com.ibm.icu.impl.locale.LocaleDistance`.
#[derive(Debug)]
pub struct LocaleDistance {
    // The trie maps each dlang+slang+dscript+sscript+dregion+sregion
    // (encoded in ASCII with bit 7 set on the last character of each subtag) to a distance.
    // There is also a trie value for each subsequence of whole subtags.
    // One '*' is used for a (desired, supported) pair of "und", "Zzzz"/"", or "ZZ"/"".
    trie: BytesTrie,
    /// Maps each region to zero or more single-character partitions.
    region_to_partitions_index: Vec<i8>,
    partition_arrays: Vec<Vec<u16>>,
    /// Used to get the paradigm region for a cluster, if there is one.
    paradigm_lsrs: Vec<LSR>,
    default_language_distance: i32,
    default_script_distance: i32,
    default_region_distance: i32,
    min_region_distance: i32,
    default_demotion_per_desired_locale: i32,
}

/// `INSTANCE`, or the message of its initializer's exception.
static INSTANCE: LazyLock<std::result::Result<Arc<LocaleDistance>, String>> = LazyLock::new(|| {
    Data::load()
        .and_then(LocaleDistance::new)
        .map(Arc::new)
        .map_err(|e| e.to_string())
});

impl LocaleDistance {
    /// `shiftDistance(int distance)`.
    pub const fn shift_distance(distance: i32) -> i32 {
        distance << DISTANCE_SHIFT
    }

    /// `getShiftedDistance(int indexAndDistance)`.
    pub const fn get_shifted_distance(index_and_distance: i32) -> i32 {
        index_and_distance & DISTANCE_MASK
    }

    /// `getDistanceDouble(int indexAndDistance)`.
    pub fn get_distance_double(index_and_distance: i32) -> f64 {
        let shifted_distance = Self::get_shifted_distance(index_and_distance) as f64;
        shifted_distance / (1 << DISTANCE_SHIFT) as f64
    }

    /// `getDistanceFloor(int indexAndDistance)`.
    pub const fn get_distance_floor(index_and_distance: i32) -> i32 {
        (index_and_distance & DISTANCE_MASK) >> DISTANCE_SHIFT
    }

    /// `getIndex(int indexAndDistance)`.
    pub fn get_index(index_and_distance: i32) -> i32 {
        debug_assert!(index_and_distance >= 0);
        index_and_distance >> INDEX_SHIFT
    }

    /// `INSTANCE` (VisibleForTesting).
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the data cannot be loaded (Java's
    /// `ExceptionInInitializerError`).
    pub fn instance() -> Result<Arc<LocaleDistance>> {
        match &*INSTANCE {
            Ok(i) => Ok(i.clone()),
            Err(m) => Err(missing_resource(m.clone(), "", "")),
        }
    }

    /// `LocaleDistance(Data data)` (private).
    fn new(data: Data) -> Result<Self> {
        let mut d = Self {
            trie: BytesTrie::new(Arc::from(data.trie), 0),
            region_to_partitions_index: data.region_to_partitions_index,
            partition_arrays: data.partition_arrays,
            paradigm_lsrs: data.paradigm_lsrs,
            default_language_distance: data.distances[IX_DEF_LANG_DISTANCE],
            default_script_distance: data.distances[IX_DEF_SCRIPT_DISTANCE],
            default_region_distance: data.distances[IX_DEF_REGION_DISTANCE],
            min_region_distance: data.distances[IX_MIN_REGION_DISTANCE],
            default_demotion_per_desired_locale: 0,
        };

        // For the default demotion value, use the
        // default region distance between unrelated Englishes.
        // Thus, unless demotion is turned off,
        // a mere region difference for one desired locale
        // is as good as a perfect match for the next following desired locale.
        // As of CLDR 36, we have <languageMatch desired="en_*_*" supported="en_*_*" distance="5"/>.
        let en = LSR::new("en", "Latn", "US", LSR::EXPLICIT_LSR);
        let en_gb = LSR::new("en", "Latn", "GB", LSR::EXPLICIT_LSR);
        let index_and_distance = d.get_best_index_and_distance(
            &en,
            &[en_gb],
            1,
            Self::shift_distance(50),
            Some(FavorSubtag::Language),
            Some(Direction::WithOneWay),
        )?;
        d.default_demotion_per_desired_locale = Self::get_distance_floor(index_and_distance);
        // (`if (DEBUG_OUTPUT)`: `LSR.DEBUG_OUTPUT` is a false constant; javac drops the block.)
        Ok(d)
    }

    /// `testOnlyDistance(ULocale desired, ULocale supported, int
    /// threshold, FavorSubtag favorSubtag)` (VisibleForTesting).
    ///
    /// # Errors
    ///
    /// The errors of the likely-subtags lookups.
    pub fn test_only_distance(
        &self,
        desired: &ULocale,
        supported: &ULocale,
        threshold: i32,
        favor_subtag: Option<FavorSubtag>,
    ) -> Result<i32> {
        let likely = LikelySubtags::instance()?;
        let supported_lsr = likely.make_maximized_lsr_from(supported, false)?;
        let desired_lsr = likely.make_maximized_lsr_from(desired, false)?;
        let index_and_distance = self.get_best_index_and_distance(
            &desired_lsr,
            &[supported_lsr],
            1,
            Self::shift_distance(threshold),
            favor_subtag,
            Some(Direction::WithOneWay),
        )?;
        Ok(Self::get_distance_floor(index_and_distance))
    }

    /// `getBestIndexAndDistance(LSR desired, LSR[] supportedLSRs, int
    /// supportedLSRsLength, int shiftedThreshold, FavorSubtag favorSubtag,
    /// LocaleMatcher.Direction direction)`: the index of the best-matching
    /// supported LSR (shifted left by `INDEX_SHIFT`) and its shifted
    /// distance, or a negative value when none is below the threshold.
    /// `None` stands for Java's `null` enum values.
    ///
    /// # Errors
    ///
    /// The errors of loading `LikelySubtags.INSTANCE` (used to break ties).
    pub fn get_best_index_and_distance(
        &self,
        desired: &LSR,
        supported_lsrs: &[LSR],
        supported_lsrs_length: i32,
        mut shifted_threshold: i32,
        favor_subtag: Option<FavorSubtag>,
        direction: Option<Direction>,
    ) -> Result<i32> {
        let mut iter = BytesTrie::from_other(&self.trie);
        // Look up the desired language only once for all supported LSRs.
        // Its "distance" is either a match point value of 0, or a non-match negative value.
        // Note: The data builder verifies that there are no <*, supported> or <desired, *> rules.
        let des_lang_distance = trie_next(&mut iter, &desired.language, false);
        let des_lang_state = if des_lang_distance >= 0 && supported_lsrs_length > 1 {
            iter.get_state64()
        } else {
            0
        };
        // Index of the supported LSR with the lowest distance.
        let mut best_index: i32 = -1;
        // Cached lookup info from LikelySubtags.compareLikely().
        let mut best_likely_info: i32 = -1;
        for sl_index in 0..supported_lsrs_length {
            let supported = &supported_lsrs[sl_index as usize];
            let mut star = false;
            let mut distance = des_lang_distance;
            if distance >= 0 {
                debug_assert!((distance & DISTANCE_IS_FINAL) == 0);
                if sl_index != 0 {
                    iter.reset_to_state64(des_lang_state);
                }
                distance = trie_next(&mut iter, &supported.language, true);
            }
            // Note: The data builder verifies that there are no rules with "any" (*) language and
            // real (non *) script or region subtags.
            // This means that if the lookup for either language fails we can use
            // the default distances without further lookups.
            let mut flags;
            if distance >= 0 {
                flags = distance & DISTANCE_IS_FINAL_OR_SKIP_SCRIPT;
                distance &= !DISTANCE_IS_FINAL_OR_SKIP_SCRIPT;
            } else {
                // <*, *>
                if desired.language == supported.language {
                    distance = 0;
                } else {
                    distance = self.default_language_distance;
                }
                flags = 0;
                star = true;
            }
            debug_assert!((0..=100).contains(&distance));
            // Round up the shifted threshold (if fraction bits are not 0)
            // for comparison with un-shifted distances until we need fraction bits.
            // (If we simply shifted non-zero fraction bits away, then we might ignore a language
            // when it's really still a micro distance below the threshold.)
            let rounded_threshold = (shifted_threshold + DISTANCE_FRACTION_MASK) >> DISTANCE_SHIFT;
            // We implement "favor subtag" by reducing the language subtag distance
            // (unscientifically reducing it to a quarter of the normal value),
            // so that the script distance is relatively more important.
            // For example, given a default language distance of 80, we reduce it to 20,
            // which is below the default threshold of 50, which is the default script distance.
            if favor_subtag == Some(FavorSubtag::Script) {
                distance >>= 2;
            }
            // Let distance == roundedThreshold pass until the tie-breaker logic
            // at the end of the loop.
            if distance > rounded_threshold {
                continue;
            }

            let script_distance;
            if star || flags != 0 {
                if desired.script == supported.script {
                    script_distance = 0;
                } else {
                    script_distance = self.default_script_distance;
                }
            } else {
                let state = iter.get_state64();
                let sd = get_des_supp_script_distance(
                    &mut iter,
                    state,
                    &desired.script,
                    &supported.script,
                );
                flags = sd & DISTANCE_IS_FINAL;
                script_distance = sd & !DISTANCE_IS_FINAL;
            }
            distance += script_distance;
            if distance > rounded_threshold {
                continue;
            }

            if desired.region == supported.region {
                // regionDistance = 0
            } else if star || (flags & DISTANCE_IS_FINAL) != 0 {
                distance += self.default_region_distance;
            } else {
                let remaining_threshold = rounded_threshold - distance;
                if self.min_region_distance > remaining_threshold {
                    continue;
                }

                // From here on we know the regions are not equal.
                // Map each region to zero or more partitions. (zero = one non-matching string)
                // (Each array of single-character partition strings is encoded as one string.)
                // If either side has more than one, then we find the maximum distance.
                // This could be optimized by adding some more structure, but probably not worth it.
                let state = iter.get_state64();
                distance += get_region_partitions_distance(
                    &mut iter,
                    state,
                    self.partitions_for_region(desired),
                    self.partitions_for_region(supported),
                    remaining_threshold,
                );
            }
            let mut shifted_distance = Self::shift_distance(distance);
            if shifted_distance == 0 {
                // Distinguish between equivalent but originally unequal locales via an
                // additional micro distance.
                shifted_distance |= desired.flags ^ supported.flags;
                if shifted_distance < shifted_threshold
                    && (direction != Some(Direction::OnlyTwoWay)
                        // Is there also a match when we swap desired/supported?
                        || self.is_match(supported, desired, shifted_threshold, favor_subtag)?)
                {
                    if shifted_distance == 0 {
                        return Ok(sl_index << INDEX_SHIFT);
                    }
                    best_index = sl_index;
                    shifted_threshold = shifted_distance;
                    best_likely_info = -1;
                }
            } else if shifted_distance < shifted_threshold {
                if direction != Some(Direction::OnlyTwoWay)
                    // Is there also a match when we swap desired/supported?
                    || self.is_match(supported, desired, shifted_threshold, favor_subtag)?
                {
                    best_index = sl_index;
                    shifted_threshold = shifted_distance;
                    best_likely_info = -1;
                }
            } else if shifted_distance == shifted_threshold
                && best_index >= 0
                && (direction != Some(Direction::OnlyTwoWay)
                    // Is there also a match when we swap desired/supported?
                    || self.is_match(supported, desired, shifted_threshold, favor_subtag)?)
            {
                best_likely_info = LikelySubtags::instance()?.compare_likely(
                    supported,
                    &supported_lsrs[best_index as usize],
                    best_likely_info,
                );
                if (best_likely_info & 1) != 0 {
                    // This supported locale matches as well as the previous best match,
                    // and neither matches perfectly,
                    // but this one is "more likely" (has more-default subtags).
                    best_index = sl_index;
                }
            }
        }
        Ok(if best_index >= 0 {
            (best_index << INDEX_SHIFT) | shifted_threshold
        } else {
            INDEX_NEG_1 | Self::shift_distance(ABOVE_THRESHOLD)
        })
    }

    fn is_match(
        &self,
        desired: &LSR,
        supported: &LSR,
        shifted_threshold: i32,
        favor_subtag: Option<FavorSubtag>,
    ) -> Result<bool> {
        Ok(self.get_best_index_and_distance(
            desired,
            std::slice::from_ref(supported),
            1,
            shifted_threshold,
            favor_subtag,
            None,
        )? >= 0)
    }

    fn partitions_for_region(&self, lsr: &LSR) -> &[u16] {
        // ill-formed region -> one non-matching string
        let p_index = self.region_to_partitions_index[lsr.region_index as usize] as i32;
        &self.partition_arrays[p_index as usize]
    }

    /// `isParadigmLSR(LSR lsr)`: whether the LSR is equivalent to one of the
    /// paradigm LSRs.
    pub fn is_paradigm_lsr(&self, lsr: &LSR) -> bool {
        // Linear search for a very short list (length 6 as of 2019),
        // because we look for equivalence not equality, and
        // HashSet does not support customizing equality.
        // If there are many paradigm LSRs we should revisit this.
        debug_assert!(self.paradigm_lsrs.len() <= 15);
        for plsr in &self.paradigm_lsrs {
            if lsr.is_equivalent_to(plsr) {
                return true;
            }
        }
        false
    }

    /// `getDefaultScriptDistance()` (VisibleForTesting).
    pub fn get_default_script_distance(&self) -> i32 {
        self.default_script_distance
    }

    /// `getDefaultRegionDistance()` (package-private; no caller in icu4j's
    /// main code).
    #[allow(dead_code)]
    pub(crate) fn get_default_region_distance(&self) -> i32 {
        self.default_region_distance
    }

    /// `getDefaultDemotionPerDesiredLocale()`.
    pub fn get_default_demotion_per_desired_locale(&self) -> i32 {
        self.default_demotion_per_desired_locale
    }

    /// `testOnlyGetDistanceTable()` (VisibleForTesting): the trie entries
    /// as `desired-supported-...` keys (a `TreeMap`, sorted by
    /// `String.compareTo`; the keys are ASCII).
    pub fn test_only_get_distance_table(&self) -> BTreeMap<String, i32> {
        let mut map = BTreeMap::new();
        let mut sb = String::new();
        for entry in self.trie.iterator() {
            sb.clear();
            let length = entry.bytes_length();
            for i in 0..length {
                let b = entry.byte_at(i);
                if b == b'*' as i8 {
                    // One * represents a (desired, supported) = (ANY, ANY) pair.
                    sb.push_str("*-*-");
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
            map.insert(sb.clone(), entry.value);
        }
        map
    }

    /// `testOnlyPrintDistanceTable()` (VisibleForTesting): prints the table
    /// to standard output.
    pub fn test_only_print_distance_table(&self) {
        for (key, value) in self.test_only_get_distance_table() {
            let mut suffix = "";
            let mut value = value;
            if (value & DISTANCE_SKIP_SCRIPT) != 0 {
                value &= !DISTANCE_SKIP_SCRIPT;
                suffix = " skip script";
            }
            println!("{key}={value}{suffix}");
        }
    }
}

/// `toString()`: the distance table (`AbstractMap.toString`).
impl fmt::Display for LocaleDistance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        let mut first = true;
        for (k, v) in self.test_only_get_distance_table() {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            write!(f, "{k}={v}")?;
        }
        f.write_str("}")
    }
}

fn get_des_supp_script_distance(
    iter: &mut BytesTrie,
    start_state: i64,
    desired: &str,
    supported: &str,
) -> i32 {
    // Note: The data builder verifies that there are no <*, supported> or <desired, *> rules.
    let mut distance = trie_next(iter, desired, false);
    if distance >= 0 {
        distance = trie_next(iter, supported, true);
    }
    if distance < 0 {
        let result = iter.reset_to_state64(start_state).next('*' as i32); // <*, *>
        debug_assert!(result.has_value());
        if desired == supported {
            distance = 0; // same script
        } else {
            distance = iter.get_value();
            debug_assert!(distance >= 0);
        }
        if result == TrieResult::FinalValue {
            distance |= DISTANCE_IS_FINAL;
        }
    }
    distance
}

fn get_region_partitions_distance(
    iter: &mut BytesTrie,
    start_state: i64,
    desired_partitions: &[u16],
    supported_partitions: &[u16],
    threshold: i32,
) -> i32 {
    let des_length = desired_partitions.len();
    let supp_length = supported_partitions.len();
    if des_length == 1 && supp_length == 1 {
        // Fastpath for single desired/supported partitions.
        let mut result = iter.next(desired_partitions[0] as i32 | END_OF_SUBTAG);
        if result.has_next() {
            result = iter.next(supported_partitions[0] as i32 | END_OF_SUBTAG);
            if result.has_value() {
                return iter.get_value();
            }
        }
        return get_fallback_region_distance(iter, start_state);
    }

    let mut region_distance = 0;
    // Fall back to * only once, not for each pair of partition strings.
    let mut star = false;
    let mut di = 0;
    loop {
        // Look up each desired-partition string only once,
        // not for each (desired, supported) pair.
        let result = iter.next(desired_partitions[di] as i32 | END_OF_SUBTAG);
        di += 1;
        if result.has_next() {
            let des_state = if supp_length > 1 {
                iter.get_state64()
            } else {
                0
            };
            let mut si = 0;
            loop {
                let result = iter.next(supported_partitions[si] as i32 | END_OF_SUBTAG);
                si += 1;
                let d;
                if result.has_value() {
                    d = iter.get_value();
                } else if star {
                    d = 0;
                } else {
                    d = get_fallback_region_distance(iter, start_state);
                    star = true;
                }
                if d > threshold {
                    return d;
                } else if region_distance < d {
                    region_distance = d;
                }
                if si < supp_length {
                    iter.reset_to_state64(des_state);
                } else {
                    break;
                }
            }
        } else if !star {
            let d = get_fallback_region_distance(iter, start_state);
            if d > threshold {
                return d;
            } else if region_distance < d {
                region_distance = d;
            }
            star = true;
        }
        if di < des_length {
            iter.reset_to_state64(start_state);
        } else {
            break;
        }
    }
    region_distance
}

fn get_fallback_region_distance(iter: &mut BytesTrie, start_state: i64) -> i32 {
    let result = iter.reset_to_state64(start_state).next('*' as i32); // <*, *>
    debug_assert!(result.has_value());
    let _ = result;
    let distance = iter.get_value();
    debug_assert!(distance >= 0);
    distance
}

fn trie_next(iter: &mut BytesTrie, s: &str, want_value: bool) -> i32 {
    if s.is_empty() {
        return -1; // no empty subtags in the distance data
    }
    let s: Vec<u16> = s.encode_utf16().collect();
    let end = s.len() - 1;
    let mut i = 0;
    loop {
        let c = s[i] as i32;
        if i < end {
            if !iter.next(c).has_next() {
                return -1;
            }
        } else {
            // last character of this subtag
            let result = iter.next(c | END_OF_SUBTAG);
            if want_value {
                if result.has_value() {
                    let mut value = iter.get_value();
                    if result == TrieResult::FinalValue {
                        value |= DISTANCE_IS_FINAL;
                    }
                    return value;
                }
            } else if result.has_next() {
                return 0;
            }
            return -1;
        }
        i += 1;
    }
}
