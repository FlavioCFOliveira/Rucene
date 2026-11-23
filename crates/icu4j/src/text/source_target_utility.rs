// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.SourceTargetUtility`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rucene::error::Result;

use crate::jdk::units;
use crate::lang::char_sequences::CharSequences;
use crate::text::normalizer2::{self, Normalizer2};
use crate::text::transform::Transform;
use crate::text::transliterator::Transliterator;
use crate::text::unicode_set::UnicodeSet;
use crate::text::utf16::UTF16;

/// The `Transform<String,String>` of a `SourceTargetUtility`: a UTF-16
/// string transform whose runtime exceptions are `Result` errors.
pub(crate) type StringTransformer = Box<dyn for<'a> Transform<&'a [u16], Result<Vec<u16>>> + Send + Sync>;

/// A `Transform<String,String>` over a closure (Java's anonymous
/// `Transform` classes).
pub(crate) struct FnTransform<F>(pub F);

impl<'a, F> Transform<&'a [u16], Result<Vec<u16>>> for FnTransform<F>
where
    F: Fn(&[u16]) -> Result<Vec<u16>>,
{
    fn transform(&self, source: &'a [u16]) -> Result<Vec<u16>> {
        (self.0)(source)
    }
}

/// `NON_STARTERS`: the code points with a non-zero combining class.
pub(crate) static NON_STARTERS: LazyLock<UnicodeSet> = LazyLock::new(|| match UnicodeSet::from_pattern(&units("[:^ccc=0:]")) {
    Ok(mut set) => {
        set.freeze();
        set
    }
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

/// `NFC`.
static NFC: LazyLock<Arc<dyn Normalizer2>> = LazyLock::new(|| match normalizer2::get_nfc_instance() {
    Ok(n) => n,
    Err(e) => panic!("java.lang.ExceptionInInitializerError: {e}"),
});

/// Simple internal utility class for helping with `getSource/TargetSet`.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.SourceTargetUtility`.
pub(crate) struct SourceTargetUtility {
    transform: StringTransformer,
    source_cache: UnicodeSet,
    source_strings: HashSet<Vec<u16>>,
}

impl SourceTargetUtility {
    /// `SourceTargetUtility(Transform<String, String> transform)`.
    ///
    /// # Errors
    ///
    /// The errors of the transform.
    pub fn new(transform: StringTransformer) -> Result<Self> {
        Self::with_normalizer(transform, None)
    }

    /// `SourceTargetUtility(Transform<String, String> transform,
    /// Normalizer2 normalizer)`: the characters (and decompositions) the
    /// transform changes, computed over all code points.
    ///
    /// # Errors
    ///
    /// The errors of the transform and of the normalizer.
    pub fn with_normalizer(transform: StringTransformer, normalizer: Option<&dyn Normalizer2>) -> Result<Self> {
        let mut source_cache = if normalizer.is_some() { UnicodeSet::from_pattern(&units("[:^ccc=0:]"))? } else { UnicodeSet::new() };
        let mut source_strings = HashSet::new();
        for i in 0..=0x10FFFF {
            let s = transform.transform(&UTF16::value_of(i)?)?;
            let mut added = false;
            if !CharSequences::equals_cp(i, Some(&s)) {
                source_cache.add(i)?;
                added = true;
            }
            let Some(normalizer) = normalizer else {
                continue;
            };
            let Some(d) = NFC.get_decomposition(i)? else {
                continue;
            };
            let s = transform.transform(&d)?;
            if d != s {
                source_strings.insert(d);
            }
            if added {
                continue;
            }
            if !normalizer.is_inert(i)? {
                source_cache.add(i)?;
                continue;
            }
        }
        source_cache.freeze();
        Ok(Self {
            transform,
            source_cache,
            source_strings,
        })
    }

    /// `addSourceTargetSet(Transliterator transliterator, UnicodeSet
    /// inputFilter, UnicodeSet sourceSet, UnicodeSet targetSet)`.
    ///
    /// # Errors
    ///
    /// The errors of the transform and of the `UnicodeSet` operations.
    pub fn add_source_target_set(
        &self,
        transliterator: &dyn Transliterator,
        input_filter: &UnicodeSet,
        source_set: &mut UnicodeSet,
        target_set: &mut UnicodeSet,
    ) -> Result<()> {
        let my_filter = transliterator.get_filter_as_unicode_set(input_filter)?;
        let mut affected_characters = UnicodeSet::from_set(&self.source_cache);
        affected_characters.retain_all(&my_filter)?;
        source_set.add_all(&affected_characters)?;
        for s in affected_characters.iter() {
            target_set.add_all_str(&self.transform.transform(&s)?)?;
        }
        for s in &self.source_strings {
            if my_filter.contains_all_str(s) {
                let t = self.transform.transform(s)?;
                if *s != t {
                    target_set.add_all_str(&t)?;
                    source_set.add_all_str(s)?;
                }
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for SourceTargetUtility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceTargetUtility").field("source_cache", &self.source_cache).finish_non_exhaustive()
    }
}
