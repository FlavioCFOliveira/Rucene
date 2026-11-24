// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NumberFormatServiceShim`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Weak};

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::r#impl::icu_locale_service::{
    locale_key_factory_display_name, ICULocaleService, ICUResourceBundleFactory, LocaleKey,
    LocaleKeyFactory,
};
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::icu_service::{Factory, ICUService, Key, ServiceObject, VisibleIDMap};
use crate::text::number_format::{
    self, unable_to_construct, NumberFormat, NumberFormatFactory, NumberFormatShim, RegistryKey,
    ACCOUNTINGCURRENCYSTYLE, CASHCURRENCYSTYLE, CURRENCYSTYLE, ISOCURRENCYSTYLE,
    PLURALCURRENCYSTYLE, STANDARDCURRENCYSTYLE,
};
use crate::util::currency::Currency;
use crate::util::u_locale::ULocale;

/// The `NumberFormat` a service object holds.
fn number_format_of(obj: &ServiceObject) -> Option<&dyn NumberFormat> {
    obj.downcast_ref::<Box<dyn NumberFormat>>()
        .map(|b| b.as_ref())
}

/// The registration service behind `NumberFormat.getInstance` and
/// `registerFactory`.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.NumberFormatServiceShim` (a
/// `NumberFormat.NumberFormatShim`); its static `service` is a lazily
/// initialized static.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NumberFormatServiceShim;

impl NumberFormatShim for NumberFormatServiceShim {
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

    fn register_factory(&self, factory: Arc<dyn NumberFormatFactory>) -> Result<RegistryKey> {
        let f: Arc<NFFactory> = Arc::new_cyclic(|me| NFFactory {
            delegate: factory,
            me: me.clone(),
        });
        SERVICE.register_factory(f)
    }

    fn unregister(&self, registry_key: &RegistryKey) -> bool {
        SERVICE.unregister_factory(registry_key)
    }

    fn create_instance(
        &self,
        desired_locale: &ULocale,
        choice: i32,
    ) -> Result<Box<dyn NumberFormat>> {
        // use service cache
        //          if (service.isDefault()) {
        //              return NumberFormat.createInstance(desiredLocale, choice);
        //          }

        let mut actual_loc: Option<ULocale> = None;
        let fmt = SERVICE.get_locale_kind(Some(desired_locale), choice, Some(&mut actual_loc))?;
        let Some(fmt) = fmt.as_ref().and_then(number_format_of) else {
            return Err(unable_to_construct());
        };
        let fmt = fmt.clone_number_format();

        // If we are creating a currency type formatter, then we may have to set the currency
        // explicitly, since the actualLoc may be different than the desiredLocale
        if choice == CURRENCYSTYLE
            || choice == ISOCURRENCYSTYLE
            || choice == PLURALCURRENCYSTYLE
            || choice == ACCOUNTINGCURRENCYSTYLE
            || choice == CASHCURRENCYSTYLE
            || choice == STANDARDCURRENCYSTYLE
        {
            fmt.set_currency(Currency::get_instance_ulocale(desired_locale)?)?;
        }

        let uloc = actual_loc;
        fmt.set_locale(uloc.clone(), uloc)?; // services make no distinction between actual & valid
        Ok(fmt)
    }
}

/// `NumberFormatServiceShim.NFFactory`: a `LocaleKeyFactory` delegating to a
/// registered `NumberFormatFactory`.
struct NFFactory {
    delegate: Arc<dyn NumberFormatFactory>,
    /// This factory, for the `srvc.getKey(key, null, this)` fallback.
    me: Weak<NFFactory>,
}

impl LocaleKeyFactory for NFFactory {
    fn name(&self) -> Option<&str> {
        None
    }

    fn visible(&self) -> bool {
        self.delegate.visible()
    }

    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        Ok(self.delegate.get_supported_locale_names())
    }
}

impl Factory for NFFactory {
    fn create(&self, key: &dyn Key, srvc: &ICUService) -> Result<Option<ServiceObject>> {
        if !self.handles_key(Some(key))? {
            return Ok(None);
        }
        let Some(lkey) = key.as_any().downcast_ref::<LocaleKey>() else {
            return Ok(None);
        };
        let result = self
            .delegate
            .create_format(&lkey.canonical_locale()?, lkey.kind())?;
        match result {
            Some(r) => Ok(Some(Arc::new(r) as ServiceObject)),
            None => {
                let me: Option<Arc<dyn Factory>> = self.me.upgrade().map(|m| m as Arc<dyn Factory>);
                srvc.get_key(Some(Box::new(lkey.clone())), None, me.as_ref())
            }
        }
    }

    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        let me: Arc<dyn Factory> = self.clone();
        self.lkf_update_visible_ids(me, result)
    }

    fn get_display_name(&self, id: &str, locale: Option<&ULocale>) -> Result<Option<String>> {
        locale_key_factory_display_name(id, locale)
    }

    fn to_java_string(&self) -> String {
        self.lkf_to_string()
    }
}

/// The `RBNumberFormatFactory` local class of `NFService`: an
/// `ICUResourceBundleFactory` creating `NumberFormat.createInstance(loc,
/// kind)`.
struct RBNumberFormatFactory {
    inner: ICUResourceBundleFactory,
}

impl LocaleKeyFactory for RBNumberFormatFactory {
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
        loc: &ULocale,
        kind: i32,
        _srvc: &ICUService,
    ) -> Result<Option<ServiceObject>> {
        Ok(number_format::create_instance(loc, kind)?.map(|f| Arc::new(f) as ServiceObject))
    }
}

impl Factory for RBNumberFormatFactory {
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        self.lkf_create(key, service)
    }

    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
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

/// `service`: the `NFService` (`ICULocaleService("NumberFormat")` with the
/// resource-bundle factory registered and marked default).
static SERVICE: LazyLock<ICULocaleService> = LazyLock::new(|| {
    let service = ICULocaleService::new("NumberFormat");
    let factory: Arc<dyn Factory> = Arc::new(RBNumberFormatFactory {
        inner: ICUResourceBundleFactory::new(),
    });
    service
        .register_factory(factory)
        .unwrap_or_else(|e| panic!("java.lang.ExceptionInInitializerError: {e}"));
    service.mark_default();
    service
});
