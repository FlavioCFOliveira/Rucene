// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CollatorServiceShim`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::r#impl::coll::collation_loader::CollationLoader;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_locale_service::{
    locale_key_factory_display_name, ICULocaleService, ICULocaleServiceSubclass,
    ICUResourceBundleFactory, LocaleKey, LocaleKeyFactory,
};
use crate::r#impl::icu_resource_bundle::{icu_data_class_loader, ICUResourceBundle};
use crate::r#impl::icu_service::{Factory, ICUService, Key, ServiceObject, VisibleIDMap};
use crate::text::collator::{Collator, CollatorFactory, RegistryKey, ServiceShim};
use crate::text::rule_based_collator::RuleBasedCollator;
use crate::util::output::Output;
use crate::util::u_locale::ULocale;

/// The collator service objects: `Box<dyn Collator>` behind the service's
/// `Arc<dyn Any>`.
fn collator_of(obj: &ServiceObject) -> Option<&dyn Collator> {
    obj.downcast_ref::<Box<dyn Collator>>().map(|b| b.as_ref())
}

/// The collator service.
///
/// Equivalent to `com.ibm.icu.text.CollatorServiceShim` (package private,
/// the only `Collator.ServiceShim`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CollatorServiceShim;

impl ServiceShim for CollatorServiceShim {
    fn get_instance(&self, locale: &ULocale) -> Result<Box<dyn Collator>> {
        // use service cache, it's faster than instantiation
        //          if (service.isDefault()) {
        //              return new RuleBasedCollator(locale);
        //          }
        let mut actual_loc: Option<ULocale> = None;
        let coll =
            SERVICE.get_locale_kind(Some(locale), LocaleKey::KIND_ANY, Some(&mut actual_loc))?;
        let Some(coll) = coll.as_ref().and_then(collator_of) else {
            // CLOVER:OFF
            //Can't really change coll after it's been initialized
            return Err(
                MissingResourceException::new("Could not locate Collator data", "", "").error(),
            );
            // CLOVER:ON
        };
        coll.clone_box()
    }

    fn register_instance(
        &self,
        mut collator: Box<dyn Collator>,
        locale: &ULocale,
    ) -> Result<RegistryKey> {
        // Set the collator locales while registering so that getInstance()
        // need not guess whether the collator's locales are already set properly
        // (as they are by the data loader).
        collator.set_locale(Some(locale.clone()), Some(locale.clone()));
        SERVICE.register_object_locale(Arc::new(collator), locale, LocaleKey::KIND_ANY, true)
    }

    fn register_factory(&self, f: Arc<dyn CollatorFactory>) -> Result<RegistryKey> {
        SERVICE.register_factory(Arc::new(CFactory { delegate: f }))
    }

    fn unregister(&self, registry_key: &RegistryKey) -> bool {
        SERVICE.unregister_factory(registry_key)
    }

    fn get_available_locales(&self) -> Result<Vec<Locale>> {
        // TODO rewrite this to just wrap getAvailableULocales later
        if SERVICE.is_default() {
            Ok((*ICUResourceBundle::get_available_locales_loader(
                ICUData::ICU_COLLATION_BASE_NAME,
                icu_data_class_loader(),
            )?)
            .clone())
        } else {
            SERVICE.get_available_locales()
        }
    }

    fn get_available_u_locales(&self) -> Result<Vec<ULocale>> {
        if SERVICE.is_default() {
            Ok(ICUResourceBundle::get_available_u_locales_loader(
                ICUData::ICU_COLLATION_BASE_NAME,
                icu_data_class_loader(),
            )?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException")))
        } else {
            SERVICE.get_available_u_locales()
        }
    }

    fn get_display_name(
        &self,
        object_locale: &ULocale,
        display_locale: &ULocale,
    ) -> Result<Option<String>> {
        let id = object_locale.get_name();
        SERVICE.get_display_name_in(id, Some(display_locale))
    }
}

/// The local class `CFactory` of `registerFactory(CollatorFactory)`.
struct CFactory {
    delegate: Arc<dyn CollatorFactory>,
}

impl LocaleKeyFactory for CFactory {
    fn name(&self) -> Option<&str> {
        None
    }

    fn visible(&self) -> bool {
        self.delegate.visible()
    }

    fn handle_create(
        &self,
        loc: &ULocale,
        _kind: i32,
        _srvc: &ICUService,
    ) -> Result<Option<ServiceObject>> {
        let coll = self.delegate.create_collator(loc)?;
        Ok(coll.map(|c| Arc::new(c) as ServiceObject))
    }

    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        self.delegate.get_supported_locale_ids()
    }
}

impl Factory for CFactory {
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        self.lkf_create(key, service)
    }

    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        let me: Arc<dyn Factory> = self.clone();
        self.lkf_update_visible_ids(me, result)
    }

    fn get_display_name(
        &self,
        id: &str,
        display_locale: Option<&ULocale>,
    ) -> Result<Option<String>> {
        let object_locale = ULocale::new(id)?;
        match display_locale {
            Some(dl) => self.delegate.get_display_name(&object_locale, dl),
            None => Err(LuceneError::null_pointer()),
        }
    }

    fn to_java_string(&self) -> String {
        self.lkf_to_string()
    }
}

/// The local class `CollatorFactory` of `CService()`: an
/// `ICUResourceBundleFactory` over the collation data whose `handleCreate`
/// makes a collator.
struct CServiceCollatorFactory {
    inner: ICUResourceBundleFactory,
}

impl LocaleKeyFactory for CServiceCollatorFactory {
    fn name(&self) -> Option<&str> {
        None
    }

    fn visible(&self) -> bool {
        true
    }

    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        self.inner.get_supported_ids()
    }

    fn handle_create(
        &self,
        uloc: &ULocale,
        _kind: i32,
        _srvc: &ICUService,
    ) -> Result<Option<ServiceObject>> {
        Ok(Some(Arc::new(make_instance(uloc)?)))
    }
}

impl Factory for CServiceCollatorFactory {
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        self.lkf_create(key, service)
    }

    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        // ICUResourceBundleFactory.updateVisibleIDs: only visible ids
        let visible_ids = ICUResourceBundle::get_available_locale_name_set_of(
            &self.inner.bundle_name,
            self.inner.loader(),
        )?;
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
        format!(
            "{}, bundle: {}",
            self.lkf_to_string(),
            self.inner.bundle_name
        )
    }
}

/// The overrides of `CService` (a subclass of `ICULocaleService`).
struct CServiceSubclass;

impl ICULocaleServiceSubclass for CServiceSubclass {
    fn validate_fallback_locale(&self) -> Option<String> {
        Some(String::new())
    }

    // CLOVER:OFF
    // The following method can not be reached by testing
    fn handle_default(
        &self,
        _service: &ICUService,
        _key: Option<&dyn Key>,
        actual_id_return: Option<&mut String>,
    ) -> Result<Option<ServiceObject>> {
        if let Some(r) = actual_id_return {
            *r = "root".to_string();
        }
        match make_instance(&ULocale::root()) {
            Ok(c) => Ok(Some(Arc::new(c))),
            Err(e) if MissingResourceException::of(&e).is_some() => Ok(None),
            Err(e) => Err(e),
        }
    }
    // CLOVER:ON
}

/// `service = new CService()`.
static SERVICE: LazyLock<ICULocaleService> = LazyLock::new(|| {
    let service = ICULocaleService::with_subclass("Collator", Box::new(CServiceSubclass));
    let factory: Arc<dyn Factory> = Arc::new(CServiceCollatorFactory {
        inner: ICUResourceBundleFactory::with_bundle_name(ICUData::ICU_COLLATION_BASE_NAME),
    });
    service
        .register_factory(factory)
        .unwrap_or_else(|e| panic!("java.lang.ExceptionInInitializerError: {e}"));
    service.mark_default();
    service
});

/// Ported from C++ Collator::makeInstance().
fn make_instance(desired_locale: &ULocale) -> Result<Box<dyn Collator>> {
    let mut valid_locale = Output::with_value(ULocale::root());
    let t = CollationLoader::load_tailoring(desired_locale, &mut valid_locale)?;
    Ok(Box::new(RuleBasedCollator::from_tailoring(
        t,
        valid_locale.value,
    )))
}
