// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.BreakIteratorFactory`: the service behind
//! `BreakIterator`'s factories and registration.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::StringCharacterIterator;
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::to_rust_string;
use crate::r#impl::assert::Assert;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_locale_service::{
    locale_key_factory_display_name, ICULocaleService, ICULocaleServiceSubclass, ICUResourceBundleFactory,
    LocaleKeyFactory,
};
use crate::r#impl::icu_resource_bundle::{ICUResourceBundle, OpenType};
use crate::r#impl::icu_service::{Factory, ICUService, Key, ServiceObject, VisibleIDMap};
use crate::text::break_iterator::{BreakIterator, BreakIteratorServiceShim, KIND_LINE, KIND_SENTENCE};
use crate::text::filtered_break_iterator_builder;
use crate::text::rule_based_break_iterator::RuleBasedBreakIterator;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::missing_resource;

/// The service object a registered or created break iterator is stored
/// as.
///
/// **Divergence.** Java's service hands back the very object it holds
/// (`BreakIteratorFactory.java`, `createBreakIterator`), so callers share
/// and may change it; a `Box<dyn BreakIterator>` cannot be shared, so the
/// service holds the iterator behind a lock and each request receives a
/// clone of it.
type ServiceBreakIterator = Mutex<Box<dyn BreakIterator>>;

/// `KIND_NAMES`: the resource keys of the five kinds.
const KIND_NAMES: [&str; 5] = ["grapheme", "word", "line", "sentence", "title"];

/// `BFService`: the `ICULocaleService` subclass whose
/// `validateFallbackLocale` answers `""`.
struct BFServiceSubclass;

impl ICULocaleServiceSubclass for BFServiceSubclass {
    /// `validateFallbackLocale()`: `""` — the default implementation of
    /// `validateFallbackLocale` uses the default locale, which is wrong for
    /// break iterators.
    fn validate_fallback_locale(&self) -> Option<String> {
        Some(String::new())
    }
}

/// `RBBreakIteratorFactory`: the local subclass of
/// `ICUResourceBundleFactory` that creates break iterators.
struct RBBreakIteratorFactory {
    base: ICUResourceBundleFactory,
}

impl LocaleKeyFactory for RBBreakIteratorFactory {
    fn name(&self) -> Option<&str> {
        self.base.name()
    }

    fn visible(&self) -> bool {
        self.base.visible()
    }

    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        self.base.get_supported_ids()
    }

    /// `handleCreate(ULocale loc, int kind, ICUService srvc)`:
    /// `createBreakInstance(loc, kind)`.
    fn handle_create(&self, loc: &ULocale, kind: i32, _service: &ICUService) -> Result<Option<ServiceObject>> {
        let iter = create_break_instance(loc, kind)?;
        let obj: ServiceObject = Arc::new(Mutex::new(iter));
        Ok(Some(obj))
    }
}

impl Factory for RBBreakIteratorFactory {
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        self.lkf_create(key, service)
    }

    /// `ICUResourceBundleFactory.updateVisibleIDs`: the bundle's available
    /// locale names.
    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        let visible_ids =
            ICUResourceBundle::get_available_locale_name_set_of(&self.base.bundle_name, self.base.loader())?; // only visible ids
        for id in visible_ids.iter() {
            let me: Arc<dyn Factory> = self.clone();
            result.insert(id.clone(), me);
        }
        Ok(())
    }

    fn get_display_name(&self, id: &str, locale: Option<&ULocale>) -> Result<Option<String>> {
        locale_key_factory_display_name(id, locale)
    }

    fn to_java_string(&self) -> String {
        self.base.to_java_string()
    }
}

/// The break-iterator service shim.
///
/// Equivalent to the package-private final class
/// `com.ibm.icu.text.BreakIteratorFactory`; its static `service` is the
/// field of the single instance `BreakIterator` keeps.
pub(crate) struct BreakIteratorFactory {
    /// `service`: the `BFService`.
    service: ICULocaleService,
}

impl BreakIteratorFactory {
    /// The shim with its `BFService`: one `RBBreakIteratorFactory`, marked
    /// default.
    ///
    /// # Errors
    ///
    /// The exceptions of the factory registration.
    pub(crate) fn new() -> Result<Self> {
        let service = ICULocaleService::with_subclass("BreakIterator", Box::new(BFServiceSubclass));
        service.register_factory(Arc::new(RBBreakIteratorFactory {
            base: ICUResourceBundleFactory::new(),
        }))?;
        service.mark_default();
        Ok(Self { service })
    }
}

impl BreakIteratorServiceShim for BreakIteratorFactory {
    fn register_instance(&self, mut iter: Box<dyn BreakIterator>, locale: &ULocale, kind: i32) -> Result<Arc<dyn Factory>> {
        iter.set_text(Some(Box::new(StringCharacterIterator::new(Vec::new()))));
        let obj: ServiceObject = Arc::new(Mutex::new(iter));
        self.service.register_object_locale(obj, locale, kind, true)
    }

    fn unregister(&self, key: &Arc<dyn Factory>) -> bool {
        if self.service.is_default() {
            return false;
        }
        self.service.unregister_factory(key)
    }

    fn get_available_locales(&self) -> Result<Vec<Locale>> {
        self.service.get_available_locales()
    }

    fn get_available_u_locales(&self) -> Result<Vec<ULocale>> {
        self.service.get_available_u_locales()
    }

    fn create_break_iterator(&self, locale: &ULocale, kind: i32) -> Result<Box<dyn BreakIterator>> {
        // TODO: convert to ULocale when service switches over
        if self.service.is_default() {
            return create_break_instance(locale, kind);
        }
        let mut actual_loc: Option<ULocale> = None;
        let obj = self.service.get_locale_kind(Some(locale), kind, Some(&mut actual_loc))?;
        let obj = obj.ok_or_else(LuceneError::null_pointer)?;
        let shared = obj
            .downcast_ref::<ServiceBreakIterator>()
            .ok_or_else(|| LuceneError::unchecked("java.lang.ClassCastException", "not a BreakIterator"))?;
        let mut iter = shared.lock().unwrap_or_else(|e| e.into_inner()).clone_box();
        iter.set_locale(actual_loc.clone(), actual_loc)?;
        Ok(iter)
    }
}

/// `createBreakInstance(ULocale locale, int kind)`: the rule-based
/// iterator of `kind` for `locale` from the `brkitr` data, wrapped in the
/// sentence filter for `ss=standard`.
///
/// # Errors
///
/// `MissingResourceException` (with the cause's `toString()`) when the
/// rules cannot be found or read.
fn create_break_instance(locale: &ULocale, kind: i32) -> Result<Box<dyn BreakIterator>> {
    let rb = ICUResourceBundle::get_bundle_instance_ulocale(
        Some(ICUData::ICU_BRKITR_BASE_NAME),
        Some(locale),
        OpenType::LocaleRoot,
    )?;

    //
    //  Get the binary rules.
    //
    let mut type_key_ext = String::new();
    if kind == KIND_LINE {
        let key_value = locale.get_keyword_value("lb");
        if let Some(kv) = key_value.as_deref() {
            if kv == "strict" || kv == "normal" || kv == "loose" {
                type_key_ext = format!("_{kv}");
            }
        }
        let language = locale.get_language();
        // lw=phrase is only supported in Japanese and Korean
        if language == "ja" || language == "ko" {
            let key_value = locale.get_keyword_value("lw");
            if key_value.as_deref() == Some("phrase") {
                type_key_ext.push_str("_phrase");
            }
        }
    }

    let found = (|| -> Result<(Vec<u16>, crate::jdk::ByteBuffer)> {
        let type_key = if type_key_ext.is_empty() {
            KIND_NAMES[kind as usize].to_string()
        } else {
            format!("{}{}", KIND_NAMES[kind as usize], type_key_ext)
        };
        let brkfname = rb.get_string_with_fallback(&format!("boundaries/{type_key}"))?;
        let rules_file_name = format!("{}/{}", ICUData::ICU_BRKITR_NAME, to_rust_string(&brkfname));
        let bytes = ICUBinary::get_required_data(&rules_file_name)?;
        Ok((brkfname, bytes))
    })();
    let (brkfname, bytes) = match found {
        Ok(v) => v,
        Err(e) => return Err(missing_resource(e.to_string(), "", "")),
    };

    //
    // Create a normal RuleBasedBreakIterator.
    //
    let is_phrase_breaking = to_rust_string(&brkfname).contains("phrase");
    let mut iter = match RuleBasedBreakIterator::get_instance_from_compiled_rules_phrase(bytes, is_phrase_breaking) {
        Ok(iter) => iter,
        Err(e) if e.is_io_exception() => {
            // Shouldn't be possible to get here.
            // If it happens, the compiled break rules are probably corrupted.
            Assert::fail_exception(&e)?;
            unreachable!("Assert.fail always throws")
        }
        Err(e) => return Err(e),
    };
    // TODO: Determine valid and actual locale correctly.
    let uloc = ULocale::for_locale(Some(&rb.get_locale()))?;
    iter.set_locale(uloc.clone(), uloc)?;

    // filtered break
    if kind == KIND_SENTENCE {
        let ss_keyword = locale.get_keyword_value("ss");
        if ss_keyword.as_deref() == Some("standard") {
            let base = ULocale::new(&locale.get_base_name())?;
            return filtered_break_iterator_builder::get_instance_ulocale(&base)?
                .wrap_iterator_with_filter(Box::new(iter));
        }
    }

    Ok(Box::new(iter))
}
