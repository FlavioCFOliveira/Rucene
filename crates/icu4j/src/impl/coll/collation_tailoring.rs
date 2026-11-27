// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationTailoring`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use rucene::error::Result;

use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::r#impl::coll::shared_object::Reference;
use crate::r#impl::norm2_all_modes::Norm2AllModes;
use crate::r#impl::trie2_32::Trie2_32;
use crate::text::unicode_set::UnicodeSet;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;
use crate::util::version_info::VersionInfo;

/// Collation tailoring data & settings. This is a container of values for
/// a collation tailoring built from rules or deserialized from binary
/// data.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationTailoring`. Java's
/// `maxExpansions`, set lazily under the tailoring's monitor by
/// `RuleBasedCollator.initMaxExpansions()`, is a [`OnceLock`].
#[derive(Debug)]
pub struct CollationTailoring {
    /// Data for sorting etc.: == base data or ownedData.
    pub data: Option<Arc<CollationData>>,
    /// Reference-counted settings.
    pub settings: Reference<CollationSettings>,
    rules: Option<Vec<u16>>,
    rules_resource: Option<UResourceBundle>,
    /// The locale is `None` (C++: bogus) when built from rules or
    /// constructed from a binary blob.
    pub actual_locale: Option<ULocale>,
    /// UCA version u.v.w & rules version r.s.t.q.
    pub version: i32,
    /// Owned objects.
    pub owned_data: Option<Arc<CollationData>>,
    /// `trie`.
    pub trie: Option<Arc<Trie2_32>>,
    /// `unsafeBackwardSet`.
    pub unsafe_backward_set: Option<Arc<UnicodeSet>>,
    /// `maxExpansions`.
    pub max_expansions: OnceLock<HashMap<i32, i32>>,
}

impl CollationTailoring {
    /// `CollationTailoring(SharedObject.Reference<CollationSettings>)`.
    pub fn new(base_settings: Option<&Reference<CollationSettings>>) -> Self {
        let settings = match base_settings {
            Some(base_settings) => {
                debug_assert!(base_settings
                    .read_only()
                    .is_some_and(|s| s.reorder_codes.is_empty()));
                debug_assert!(base_settings
                    .read_only()
                    .is_some_and(|s| s.reorder_table.is_none()));
                debug_assert!(base_settings
                    .read_only()
                    .is_some_and(|s| s.min_high_no_reorder == 0));
                base_settings.clone()
            }
            None => Reference::new(Some(CollationSettings::new())),
        };
        Self {
            data: None,
            settings,
            rules: None,
            rules_resource: None,
            actual_locale: Some(ULocale::root()),
            version: 0,
            owned_data: None,
            trie: None,
            unsafe_backward_set: None,
            max_expansions: OnceLock::new(),
        }
    }

    /// The new owned data that `ensureOwnedData()` creates: a
    /// `CollationData` over the NFC implementation. The caller fills it
    /// and installs it with [`set_owned_data`](Self::set_owned_data).
    ///
    /// **Divergence.** Java's `ensureOwnedData()`
    /// (`CollationTailoring.java:46-52`) creates `ownedData`, aliases it as
    /// `data`, and the reader/builder then fill its fields in place. Shared
    /// data cannot be mutated in Rust, so the data is filled first and
    /// installed when complete; nothing reads it in between.
    ///
    /// # Errors
    ///
    /// The exception of loading the NFC data.
    pub fn new_owned_data(&self) -> Result<CollationData> {
        match &self.owned_data {
            Some(d) => Ok((**d).clone()),
            None => {
                let nfc_impl = Norm2AllModes::get_nfc_instance()?.impl_.clone();
                Ok(CollationData::new(nfc_impl))
            }
        }
    }

    /// Installs the owned data as `ownedData` and `data`.
    pub fn set_owned_data(&mut self, data: CollationData) {
        let data = Arc::new(data);
        self.owned_data = Some(data.clone());
        self.data = Some(data);
    }

    /// The data; Java dereferences `data` without a check.
    pub fn data(&self) -> &Arc<CollationData> {
        self.data
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// The settings; Java dereferences `settings.readOnly()` without a check.
    pub fn settings(&self) -> &CollationSettings {
        self.settings
            .read_only()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `setRules(String)`.
    pub fn set_rules(&mut self, r: Vec<u16>) {
        debug_assert!(self.rules.is_none() && self.rules_resource.is_none());
        self.rules = Some(r);
    }

    /// `setRulesResource(UResourceBundle)`.
    pub fn set_rules_resource(&mut self, res: UResourceBundle) {
        debug_assert!(self.rules.is_none() && self.rules_resource.is_none());
        self.rules_resource = Some(res);
    }

    /// `getRules()`.
    ///
    /// # Errors
    ///
    /// The exception of reading the rules resource.
    pub fn get_rules(&self) -> Result<Vec<u16>> {
        if let Some(rules) = &self.rules {
            return Ok(rules.clone());
        }
        if let Some(res) = &self.rules_resource {
            return res.get_string();
        }
        Ok(Vec::new())
    }

    /// `makeBaseVersion(VersionInfo)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` of `VersionInfo.getInstance`.
    pub fn make_base_version(uca_version: &VersionInfo) -> Result<VersionInfo> {
        VersionInfo::get_instance4(
            VersionInfo::UCOL_BUILDER_VERSION.get_major(),
            (uca_version.get_major() << 3) + uca_version.get_minor(),
            uca_version.get_milli() << 6,
            0,
        )
    }

    /// `setVersion(int, int)`.
    pub fn set_version(&mut self, base_version: i32, rules_version: i32) {
        // See comments for version field.
        let r = (rules_version >> 16) & 0xff00;
        let s = (rules_version >> 16) & 0xff;
        let t = (rules_version >> 8) & 0xff;
        let q = rules_version & 0xff;
        self.version = (VersionInfo::UCOL_BUILDER_VERSION.get_major() << 24)
            | (base_version & 0xffc000) // UCA version u.v.w
            | ((r + (r >> 6)) & 0x3f00)
            | (((s << 3) + (s >> 5) + t + (q << 4) + (q >> 4)) & 0xff);
    }

    /// `getUCAVersion()`.
    pub fn get_uca_version(&self) -> i32 {
        // Version second byte/bits 23..16 to bits 11..4,
        // third byte/bits 15..14 to bits 1..0.
        ((self.version >> 12) & 0xff0) | ((self.version >> 14) & 3)
    }
}
