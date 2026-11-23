// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.util.CurrencyServiceShim`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::r#impl::icu_locale_service::{
    locale_key_factory_display_name, ICULocaleService, ICUResourceBundleFactory, LocaleKey, LocaleKeyFactory,
};
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::icu_service::{Factory, ICUService, Key, ServiceObject, VisibleIDMap};
use crate::util::currency::{Currency, RegistryKey, ServiceShim};
use crate::util::u_locale::ULocale;

/// This is a package-access implementation of registration for currency.
/// The shim is instantiated by reflection in Currency, all dependencies on
/// ICUService are located in this file. This structure is to allow ICU4J to
/// be built without service registration support.
///
/// Equivalent to the package-private final class
/// `com.ibm.icu.util.CurrencyServiceShim` (a `Currency.ServiceShim`); its
/// static `service` is a lazily initialized static. A registry key is the
/// registered factory (an `Arc<dyn Factory>`) boxed as the `Object` Java
/// returns.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CurrencyServiceShim;

impl CurrencyServiceShim {
    /// The implicit constructor `Class.newInstance()` calls (it cannot
    /// fail).
    ///
    /// # Errors
    ///
    /// None.
    pub(crate) fn new() -> Result<Self> {
        Ok(CurrencyServiceShim)
    }
}

impl ServiceShim for CurrencyServiceShim {
    fn get_available_locales(&self) -> Result<Vec<Locale>> {
        if SERVICE.is_default() {
            return Ok((*ICUResourceBundle::get_available_locales()?).clone());
        }
        SERVICE.get_available_locales()
    }

    fn get_available_u_locales(&self) -> Result<Vec<ULocale>> {
        if SERVICE.is_default() {
            return Ok(ICUResourceBundle::get_available_u_locales()?
                .unwrap_or_else(|| panic!("java.lang.NullPointerException")));
        }
        SERVICE.get_available_u_locales()
    }

    fn create_instance(&self, loc: &ULocale) -> Result<Option<Currency>> {
        // TODO: convert to ULocale when service switches over

        if SERVICE.is_default() {
            return Currency::create_currency(loc);
        }
        let curr = SERVICE.get_locale(Some(loc))?;
        match curr {
            None => Ok(None),
            Some(obj) => match obj.downcast_ref::<Currency>() {
                Some(c) => Ok(Some(c.clone())),
                None => Err(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "the registered object cannot be cast to class com.ibm.icu.util.Currency",
                )),
            },
        }
    }

    fn register_instance(&self, currency: Currency, locale: &ULocale) -> Result<RegistryKey> {
        let factory = SERVICE.register_object_locale(Arc::new(currency), locale, LocaleKey::KIND_ANY, true)?;
        Ok(Arc::new(factory))
    }

    /// Equivalent to `unregister(Object registryKey)`.
    ///
    /// # Panics
    ///
    /// `ClassCastException` when the key is not a registered factory (Java's
    /// `(Factory)registryKey`).
    fn unregister(&self, registry_key: &RegistryKey) -> bool {
        match registry_key.downcast_ref::<Arc<dyn Factory>>() {
            Some(factory) => SERVICE.unregister_factory(factory),
            None => panic!(
                "java.lang.ClassCastException: the registry key cannot be cast to class com.ibm.icu.impl.ICUService$Factory"
            ),
        }
    }
}

/// The `CurrencyFactory` local class of `CFService`: an
/// `ICUResourceBundleFactory` creating `Currency.createCurrency(loc)`.
struct CurrencyFactory {
    inner: ICUResourceBundleFactory,
}

impl LocaleKeyFactory for CurrencyFactory {
    fn name(&self) -> Option<&str> {
        None
    }

    fn visible(&self) -> bool {
        true
    }

    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        self.inner.get_supported_ids()
    }

    fn handle_create(&self, loc: &ULocale, _kind: i32, _srvc: &ICUService) -> Result<Option<ServiceObject>> {
        Ok(Currency::create_currency(loc)?.map(|c| Arc::new(c) as ServiceObject))
    }
}

impl Factory for CurrencyFactory {
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        self.lkf_create(key, service)
    }

    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        let visible_ids =
            ICUResourceBundle::get_available_locale_name_set_of(&self.inner.bundle_name, self.inner.loader())?; // only visible ids
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
        format!("{}, bundle: {}", self.lkf_to_string(), self.inner.bundle_name)
    }
}

/// `service`: the `CFService` (`ICULocaleService("Currency")` with the
/// resource-bundle currency factory registered and marked default).
static SERVICE: LazyLock<ICULocaleService> = LazyLock::new(|| {
    let service = ICULocaleService::new("Currency");
    let factory: Arc<dyn Factory> = Arc::new(CurrencyFactory { inner: ICUResourceBundleFactory::new() });
    service
        .register_factory(factory)
        .unwrap_or_else(|e| panic!("java.lang.ExceptionInInitializerError: {e}"));
    service.mark_default();
    service
});
