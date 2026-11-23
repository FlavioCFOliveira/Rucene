// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NormalizationTransliterator`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;

use crate::jdk::{append_code_point, char_count, units};
use crate::r#impl::norm2_all_modes::Norm2AllModes;
use crate::r#impl::normalizer2_impl::UTF16Plus;
use crate::text::normalizer2::{self, Normalizer2};
use crate::text::replaceable::Replaceable;
use crate::text::source_target_utility::SourceTargetUtility;
use crate::text::transform::Transform;
use crate::text::transliterator::{
    factory, register_factory_in, register_special_inverse, transliterator_boilerplate, Position, Transliterator, TransliteratorBase,
};
use crate::text::transliterator_registry::TransliteratorRegistry;
use crate::text::unicode_set::UnicodeSet;

/// `SOURCE_CACHE`: the source/target utility per normalizer.
///
/// **Divergence.** Java keys a `HashMap` by the `Normalizer2` object
/// (`NormalizationTransliterator.java:139`), which does not override
/// `hashCode`/`equals` (identity); the key here is the object's address
/// (the normalizers are process-wide singletons).
static SOURCE_CACHE: LazyLock<Mutex<HashMap<usize, Arc<SourceTargetUtility>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// A transliterator that performs normalization.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.NormalizationTransliterator`.
#[derive(Debug)]
pub struct NormalizationTransliterator {
    base: TransliteratorBase,
    norm2: Arc<dyn Normalizer2>,
}

impl NormalizationTransliterator {
    /// `register()`: system registration hook (on the registry under
    /// construction).
    pub(crate) fn register(registry: &TransliteratorRegistry) -> Result<()> {
        register_factory_in(
            registry,
            &units("Any-NFC"),
            factory(|_id| Ok(Some(Arc::new(NormalizationTransliterator::new("NFC", normalizer2::get_nfc_instance()?)) as Arc<dyn Transliterator>))),
        )?;
        register_factory_in(
            registry,
            &units("Any-NFD"),
            factory(|_id| Ok(Some(Arc::new(NormalizationTransliterator::new("NFD", normalizer2::get_nfd_instance()?)) as Arc<dyn Transliterator>))),
        )?;
        register_factory_in(
            registry,
            &units("Any-NFKC"),
            factory(|_id| {
                Ok(Some(Arc::new(NormalizationTransliterator::new("NFKC", normalizer2::get_nfkc_instance()?)) as Arc<dyn Transliterator>))
            }),
        )?;
        register_factory_in(
            registry,
            &units("Any-NFKD"),
            factory(|_id| {
                Ok(Some(Arc::new(NormalizationTransliterator::new("NFKD", normalizer2::get_nfkd_instance()?)) as Arc<dyn Transliterator>))
            }),
        )?;
        register_factory_in(
            registry,
            &units("Any-FCD"),
            factory(|_id| {
                Ok(Some(Arc::new(NormalizationTransliterator::new("FCD", Norm2AllModes::get_fcd_normalizer2()?)) as Arc<dyn Transliterator>))
            }),
        )?;
        register_factory_in(
            registry,
            &units("Any-FCC"),
            factory(|_id| {
                Ok(Some(
                    Arc::new(NormalizationTransliterator::new("FCC", Norm2AllModes::get_nfc_instance()?.fcc.clone())) as Arc<dyn Transliterator>
                ))
            }),
        )?;
        register_special_inverse(&units("NFC"), &units("NFD"), true);
        register_special_inverse(&units("NFKC"), &units("NFKD"), true);
        register_special_inverse(&units("FCC"), &units("NFD"), false);
        register_special_inverse(&units("FCD"), &units("FCD"), false);
        Ok(())
    }

    /// `NormalizationTransliterator(String id, Normalizer2 n2)`: constructs
    /// a transliterator.
    fn new(id: &str, n2: Arc<dyn Normalizer2>) -> Self {
        Self {
            base: TransliteratorBase::with_id(units(id)),
            norm2: n2,
        }
    }
}

/// `NormalizingTransform`: a `Transform<String,String>` that normalizes.
///
/// Equivalent to the nested class
/// `NormalizationTransliterator.NormalizingTransform`.
pub(crate) struct NormalizingTransform {
    norm2: Arc<dyn Normalizer2>,
}

impl NormalizingTransform {
    /// `NormalizingTransform(Normalizer2 norm2)`.
    pub(crate) fn new(norm2: Arc<dyn Normalizer2>) -> Self {
        Self { norm2 }
    }
}

impl<'a> Transform<&'a [u16], Result<Vec<u16>>> for NormalizingTransform {
    /// `transform(String source)`.
    fn transform(&self, source: &'a [u16]) -> Result<Vec<u16>> {
        self.norm2.normalize_str(source)
    }
}

impl Transliterator for NormalizationTransliterator {
    transliterator_boilerplate!();

    /// `handleTransliterate(Replaceable text, Position offsets, boolean
    /// isIncremental)`.
    fn handle_transliterate(&self, text: &dyn Replaceable, offsets: &mut Position, is_incremental: bool) -> Result<()> {
        // start and limit of the input range
        let mut start = offsets.start;
        let mut limit = offsets.limit;
        if start >= limit {
            return Ok(());
        }

        /*
         * Normalize as short chunks at a time as possible even in
         * bulk mode, so that styled text is minimally disrupted.
         * In incremental mode, a chunk that ends with offsets.limit
         * must not be normalized.
         *
         * If it was known that the input text is not styled, then
         * a bulk mode normalization could be used.
         * (For details, see the comment in the C++ version.)
         */
        let mut segment: Vec<u16> = Vec::new();
        let mut normalized: Vec<u16> = Vec::new();
        let mut c = text.char32_at(start);
        loop {
            let prev = start;
            // Skip at least one character so we make progress.
            // c holds the character at start.
            segment.clear();
            loop {
                append_code_point(&mut segment, c)?;
                start += char_count(c);
                if !(start < limit && {
                    c = text.char32_at(start);
                    !self.norm2.has_boundary_before(c)?
                }) {
                    break;
                }
            }
            if start == limit && is_incremental && !self.norm2.has_boundary_after(c)? {
                // stop in incremental mode when we reach the input limit
                // in case there are additional characters that could change the
                // normalization result
                start = prev;
                break;
            }
            self.norm2.normalize(&segment, &mut normalized)?;
            if !UTF16Plus::equal(&segment, &normalized) {
                // replace the input chunk with its normalized form
                text.replace(prev, start, &normalized);

                // update all necessary indexes accordingly
                let delta = normalized.len() as i32 - (start - prev);
                start += delta;
                limit += delta;
            }
            if start >= limit {
                break;
            }
        }

        offsets.start = start;
        offsets.context_limit += limit - offsets.limit;
        offsets.limit = limit;
        Ok(())
    }

    /// `addSourceTargetSet(UnicodeSet inputFilter, UnicodeSet sourceSet,
    /// UnicodeSet targetSet)`.
    fn add_source_target_set(&self, input_filter: &UnicodeSet, source_set: &mut UnicodeSet, target_set: &mut UnicodeSet) -> Result<()> {
        let cache = {
            let mut map = SOURCE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            let key = Arc::as_ptr(&self.norm2) as *const () as usize;
            match map.get(&key) {
                Some(c) => c.clone(),
                None => {
                    let c = Arc::new(SourceTargetUtility::with_normalizer(
                        Box::new(NormalizingTransform::new(self.norm2.clone())),
                        Some(&*self.norm2),
                    )?);
                    map.insert(key, c.clone());
                    c
                }
            }
        };
        cache.add_source_target_set(self, input_filter, source_set, target_set)
    }
}
