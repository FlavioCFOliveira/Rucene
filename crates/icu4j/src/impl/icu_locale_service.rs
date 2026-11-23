// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICULocaleService`.

use std::any::Any;
use std::collections::HashSet;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::{ClassLoader, JavaClass};
use crate::r#impl::class_loader_util::ClassLoaderUtil;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::icu_service::{Factory, ICUService, Key, ServiceObject, ServiceOverrides, VisibleIDMap};
use crate::r#impl::locale::ascii_util::{len16, substring16, substring16_from};
use crate::r#impl::locale_utility::LocaleUtility;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::UResourceBundle;

/// A subclass of `ICULocaleService`: its overrides of `handleDefault`,
/// `reInitializeFactories` and `validateFallbackLocale`.
pub trait ICULocaleServiceSubclass: Send + Sync {
    /// `handleDefault(Key key, String[] actualIDReturn)`: `null` by default.
    ///
    /// # Errors
    ///
    /// Whatever the subclass throws.
    fn handle_default(
        &self,
        _service: &ICUService,
        _key: Option<&dyn Key>,
        _actual_id_return: Option<&mut String>,
    ) -> Result<Option<ServiceObject>> {
        Ok(None)
    }

    /// `reInitializeFactories()`: clears the factories by default.
    fn re_initialize_factories(&self, _service: &ICUService, factories: &mut Vec<Arc<dyn Factory>>) {
        factories.clear();
    }

    /// `validateFallbackLocale()` override: `None` keeps
    /// `ICULocaleService`'s own.
    fn validate_fallback_locale(&self) -> Option<String> {
        None
    }
}

/// `ICULocaleService` with no subclass.
struct NoSubclass;

impl ICULocaleServiceSubclass for NoSubclass {}

/// `fallbackLocale` and `fallbackLocaleName`.
#[derive(Default)]
struct FallbackState {
    fallback_locale: Option<ULocale>,
    fallback_locale_name: Option<String>,
}

/// The `ICUService` overrides of `ICULocaleService`.
struct LocaleOverrides {
    state: Arc<Mutex<FallbackState>>,
    sub: Box<dyn ICULocaleServiceSubclass>,
}

impl LocaleOverrides {
    /// `validateFallbackLocale()`: the base name of the default locale,
    /// clearing the service cache when it changed.
    fn validate_fallback_locale(&self, service: &ICUService) -> Option<String> {
        if let Some(s) = self.sub.validate_fallback_locale() {
            return Some(s);
        }
        let loc = ULocale::get_default();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let same = st.fallback_locale.as_ref().is_some_and(|f| f == &loc);
        if !same {
            st.fallback_locale_name = Some(loc.get_base_name());
            service.clear_service_cache();
            st.fallback_locale = Some(loc);
        }
        st.fallback_locale_name.clone()
    }
}

/// The `Arc` the service holds.
struct LocaleOverridesHandle(Arc<LocaleOverrides>);

impl ServiceOverrides for LocaleOverridesHandle {
    fn create_key(&self, service: &ICUService, id: Option<&str>) -> Result<Option<Box<dyn Key>>> {
        self.0.create_key(service, id)
    }

    fn handle_default(
        &self,
        service: &ICUService,
        key: Option<&dyn Key>,
        actual_id_return: Option<&mut String>,
    ) -> Result<Option<ServiceObject>> {
        self.0.handle_default(service, key, actual_id_return)
    }

    fn re_initialize_factories(&self, service: &ICUService, factories: &mut Vec<Arc<dyn Factory>>) {
        self.0.re_initialize_factories(service, factories);
    }
}

impl ServiceOverrides for LocaleOverrides {
    fn create_key(&self, service: &ICUService, id: Option<&str>) -> Result<Option<Box<dyn Key>>> {
        let fallback = self.validate_fallback_locale(service);
        Ok(LocaleKey::create_with_canonical_fallback(id, fallback.as_deref(), LocaleKey::KIND_ANY)?
            .map(|k| Box::new(k) as Box<dyn Key>))
    }

    fn handle_default(
        &self,
        service: &ICUService,
        key: Option<&dyn Key>,
        actual_id_return: Option<&mut String>,
    ) -> Result<Option<ServiceObject>> {
        self.sub.handle_default(service, key, actual_id_return)
    }

    fn re_initialize_factories(&self, service: &ICUService, factories: &mut Vec<Arc<dyn Factory>>) {
        self.sub.re_initialize_factories(service, factories);
    }
}

/// A service with locale-based keys and fallback.
///
/// Equivalent to `com.ibm.icu.impl.ICULocaleService`, a subclass of
/// `ICUService` (which it dereferences to); its own subclasses give an
/// [`ICULocaleServiceSubclass`].
pub struct ICULocaleService {
    service: Arc<ICUService>,
    overrides: Arc<LocaleOverrides>,
}

impl Deref for ICULocaleService {
    type Target = ICUService;

    fn deref(&self) -> &ICUService {
        &self.service
    }
}

impl ICULocaleService {
    /// `new ICULocaleService(String name)`.
    pub fn new(name: &str) -> Self {
        Self::with_subclass(name, Box::new(NoSubclass))
    }

    /// A service whose subclass methods are `sub`.
    pub fn with_subclass(name: &str, sub: Box<dyn ICULocaleServiceSubclass>) -> Self {
        let overrides = Arc::new(LocaleOverrides {
            state: Arc::new(Mutex::new(FallbackState::default())),
            sub,
        });
        let service = ICUService::with_overrides(name, Box::new(LocaleOverridesHandle(overrides.clone())));
        Self { service, overrides }
    }

    /// The underlying service.
    pub fn service(&self) -> &Arc<ICUService> {
        &self.service
    }

    /// `get(ULocale locale)`.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_locale(&self, locale: Option<&ULocale>) -> Result<Option<ServiceObject>> {
        self.get_locale_kind(locale, LocaleKey::KIND_ANY, None)
    }

    /// `get(ULocale locale, int kind, ULocale[] actualReturn)`: the object
    /// for the locale and kind; `actual_return` receives the locale that
    /// was found.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_locale_kind(
        &self,
        locale: Option<&ULocale>,
        kind: i32,
        actual_return: Option<&mut Option<ULocale>>,
    ) -> Result<Option<ServiceObject>> {
        let key = self.create_key_locale(locale, kind);
        match actual_return {
            None => self.service.get_key(key, None, None),
            Some(actual_return) => {
                let mut temp = String::new();
                let result = self.service.get_key(key, Some(&mut temp), None)?;
                if result.is_some() {
                    if let Some(n) = temp.find('/') {
                        temp = temp[n + 1..].to_string();
                    }
                    *actual_return = Some(ULocale::new(&temp)?);
                }
                Ok(result)
            }
        }
    }

    /// `registerObject(Object obj, ULocale locale, int kind, boolean
    /// visible)`.
    ///
    /// # Errors
    ///
    /// None in practice.
    pub fn register_object_locale(
        &self,
        obj: ServiceObject,
        locale: &ULocale,
        kind: i32,
        visible: bool,
    ) -> Result<Arc<dyn Factory>> {
        let factory: Arc<dyn Factory> = Arc::new(SimpleLocaleKeyFactory::new(obj, locale, kind, visible, None));
        self.service.register_factory(factory)
    }

    /// `getAvailableLocales()`: the visible ids as JDK locales.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_available_locales(&self) -> Result<Vec<Locale>> {
        // TODO make this wrap getAvailableULocales later
        let vis_ids = self.service.get_visible_ids()?;
        Ok(vis_ids.iter().map(|id| LocaleUtility::get_locale_from_name(id)).collect())
    }

    /// `getAvailableULocales()`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale`; whatever the factories throw.
    pub fn get_available_u_locales(&self) -> Result<Vec<ULocale>> {
        let vis_ids = self.service.get_visible_ids()?;
        vis_ids.iter().map(|id| ULocale::new(id)).collect()
    }

    /// `validateFallbackLocale()`: the subclass's, else the base name of
    /// the default locale (clearing the service cache when it changed).
    pub fn validate_fallback_locale(&self) -> Option<String> {
        self.overrides.validate_fallback_locale(&self.service)
    }

    fn fallback(&self) -> Option<String> {
        self.validate_fallback_locale()
    }

    /// `createKey(String id, int kind)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.getName`.
    pub fn create_key_kind(&self, id: Option<&str>, kind: i32) -> Result<Option<Box<dyn Key>>> {
        let fb = self.fallback();
        Ok(LocaleKey::create_with_canonical_fallback(id, fb.as_deref(), kind)?.map(|k| Box::new(k) as Box<dyn Key>))
    }

    /// `createKey(ULocale l, int kind)`.
    pub fn create_key_locale(&self, l: Option<&ULocale>, kind: i32) -> Option<Box<dyn Key>> {
        let fb = self.fallback();
        LocaleKey::create_with_canonical(l, fb.as_deref(), kind).map(|k| Box::new(k) as Box<dyn Key>)
    }
}

/// A subclass of `Key` that implements a locale fallback mechanism.
///
/// Equivalent to `ICULocaleService.LocaleKey`.
#[derive(Debug, Clone)]
pub struct LocaleKey {
    id: String,
    kind: i32,
    varstart: i32,
    primary_id: String,
    fallback_id: Option<String>,
    current_id: Option<String>,
}

impl LocaleKey {
    /// `KIND_ANY`.
    pub const KIND_ANY: i32 = -1;

    /// `createWithCanonicalFallback(String primaryID, String
    /// canonicalFallbackID, int kind)`: `None` for a `None` primary id.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.getName`.
    pub fn create_with_canonical_fallback(
        primary_id: Option<&str>,
        canonical_fallback_id: Option<&str>,
        kind: i32,
    ) -> Result<Option<LocaleKey>> {
        let Some(primary_id) = primary_id else {
            return Ok(None);
        };
        let canonical_primary_id = ULocale::get_name_of(Some(primary_id))?;
        Ok(Some(LocaleKey::new(primary_id, Some(&canonical_primary_id), canonical_fallback_id, kind)))
    }

    /// `createWithCanonical(ULocale locale, String canonicalFallbackID,
    /// int kind)`.
    pub fn create_with_canonical(locale: Option<&ULocale>, canonical_fallback_id: Option<&str>, kind: i32) -> Option<LocaleKey> {
        let locale = locale?;
        let canonical_primary_id = locale.get_name();
        Some(LocaleKey::new(canonical_primary_id, Some(canonical_primary_id), canonical_fallback_id, kind))
    }

    /// `LocaleKey(String primaryID, String canonicalPrimaryID, String
    /// canonicalFallbackID, int kind)` (protected).
    pub fn new(primary_id: &str, canonical_primary_id: Option<&str>, canonical_fallback_id: Option<&str>, kind: i32) -> Self {
        let mut varstart = 0i32;
        let p: String;
        let fallback_id: Option<String>;
        match canonical_primary_id {
            Some(c) if !c.eq_ignore_ascii_case("root") || len16(c) != 4 => {
                let idx = crate::r#impl::locale::ascii_util::index_of16(c, "@", 0);
                if idx == 4 && c.as_bytes()[..4].eq_ignore_ascii_case(b"root") {
                    p = substring16_from(c, 4);
                    varstart = 0;
                    fallback_id = None;
                } else {
                    p = c.to_string();
                    varstart = idx;
                    fallback_id = match canonical_fallback_id {
                        Some(f) if p != f => Some(f.to_string()),
                        _ => Some(String::new()),
                    };
                }
            }
            _ => {
                p = String::new();
                fallback_id = None;
            }
        }
        let current_id = if varstart == -1 {
            p.clone()
        } else {
            substring16(&p, 0, varstart)
        };
        Self {
            id: primary_id.to_string(),
            kind,
            varstart,
            primary_id: p,
            fallback_id,
            current_id: Some(current_id),
        }
    }

    /// `prefix()`: the kind as a string, `None` for `KIND_ANY`.
    pub fn prefix(&self) -> Option<String> {
        if self.kind == Self::KIND_ANY {
            None
        } else {
            Some(self.kind().to_string())
        }
    }

    /// `kind()`.
    pub fn kind(&self) -> i32 {
        self.kind
    }

    /// `canonicalLocale()`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale`.
    pub fn canonical_locale(&self) -> Result<ULocale> {
        ULocale::new(&self.primary_id)
    }

    /// `currentLocale()`.
    ///
    /// # Errors
    ///
    /// The errors of `new ULocale`; `NullPointerException` after the last
    /// fallback.
    pub fn current_locale(&self) -> Result<ULocale> {
        let current = self.current_id.as_deref().ok_or_else(LuceneError::null_pointer)?;
        if self.varstart == -1 {
            ULocale::new(current)
        } else {
            ULocale::new(&format!("{}{}", current, substring16_from(&self.primary_id, self.varstart)))
        }
    }
}

impl Key for LocaleKey {
    fn id(&self) -> &str {
        &self.id
    }

    fn canonical_id(&self) -> Option<String> {
        Some(self.primary_id.clone())
    }

    fn current_id(&self) -> Option<String> {
        self.current_id.clone()
    }

    fn current_descriptor(&self) -> Option<String> {
        let result = self.current_id()?;
        let mut buf = String::new(); // default capacity 16 is usually good enough
        if self.kind != Self::KIND_ANY {
            buf.push_str(&self.prefix().unwrap_or_default());
        }
        buf.push('/');
        buf.push_str(&result);
        if self.varstart != -1 {
            buf.push_str(&substring16_from(&self.primary_id, self.varstart));
        }
        Some(buf)
    }

    fn fallback(&mut self) -> bool {
        let current = self.current_id.clone().unwrap_or_default();
        let units: Vec<u16> = current.encode_utf16().collect();
        let mut x = crate::jdk::last_index_of_unit(&units, b'_' as u16, units.len() as i32);
        if x != -1 {
            loop {
                x -= 1;
                if !(x >= 0 && units[x as usize] == b'_' as u16) {
                    break;
                }
                // handle zh__PINYIN
            }
            self.current_id = Some(String::from_utf16_lossy(&units[..(x + 1) as usize]));
            return true;
        }
        if let Some(f) = self.fallback_id.clone() {
            self.current_id = Some(f.clone());
            self.fallback_id = if f.is_empty() { None } else { Some(String::new()) };
            return true;
        }
        self.current_id = None;
        false
    }

    fn is_fallback_of(&self, id: &str) -> bool {
        LocaleUtility::is_fallback_of(&self.primary_id, id)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The methods of the abstract class `ICULocaleService.LocaleKeyFactory`,
/// with Java's bodies as the provided methods; every implementor is a
/// [`Factory`].
pub trait LocaleKeyFactory: Send + Sync + Any {
    /// `name` (protected final).
    fn name(&self) -> Option<&str>;

    /// `visible` (protected final).
    fn visible(&self) -> bool;

    /// `create(Key key, ICUService service)`: `handleCreate` of the key's
    /// current locale when the key is handled.
    ///
    /// # Errors
    ///
    /// Whatever `handleCreate` throws.
    fn lkf_create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        if self.handles_key(Some(key))? {
            let lkey = key
                .as_any()
                .downcast_ref::<LocaleKey>()
                .ok_or_else(|| LuceneError::unchecked("java.lang.ClassCastException", "the key is not a LocaleKey"))?;
            let kind = lkey.kind();
            let uloc = lkey.current_locale()?;
            return self.handle_create(&uloc, kind, service);
        }
        Ok(None)
    }

    /// `handlesKey(Key key)` (protected).
    ///
    /// # Errors
    ///
    /// Whatever `getSupportedIDs` throws.
    fn handles_key(&self, key: Option<&dyn Key>) -> Result<bool> {
        if let Some(key) = key {
            let id = key.current_id();
            let supported = self.get_supported_ids()?;
            return Ok(id.is_some_and(|i| supported.contains(&i)));
        }
        Ok(false)
    }

    /// `updateVisibleIDs(Map<String, Factory> result)`: the supported ids.
    ///
    /// # Errors
    ///
    /// Whatever `getSupportedIDs` throws.
    fn lkf_update_visible_ids(&self, me: Arc<dyn Factory>, result: &mut VisibleIDMap) -> Result<()> {
        let cache = self.get_supported_ids()?;
        for id in cache {
            if self.visible() {
                result.insert(id, me.clone());
            } else {
                result.remove(&id);
            }
        }
        Ok(())
    }

    /// `handleCreate(ULocale loc, int kind, ICUService service)`
    /// (protected): `null` by default.
    ///
    /// # Errors
    ///
    /// Whatever the subclass throws.
    fn handle_create(&self, _loc: &ULocale, _kind: i32, _service: &ICUService) -> Result<Option<ServiceObject>> {
        Ok(None)
    }

    /// `isSupportedID(String id)` (protected).
    ///
    /// # Errors
    ///
    /// Whatever `getSupportedIDs` throws.
    fn is_supported_id(&self, id: &str) -> Result<bool> {
        Ok(self.get_supported_ids()?.contains(id))
    }

    /// `getSupportedIDs()` (protected): empty by default.
    ///
    /// # Errors
    ///
    /// Whatever the subclass throws.
    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }

    /// `toString()`.
    fn lkf_to_string(&self) -> String {
        let mut buf = String::from("com.ibm.icu.impl.ICULocaleService$LocaleKeyFactory");
        if let Some(n) = self.name() {
            buf.push_str(", name: ");
            buf.push_str(n);
        }
        buf.push_str(", visible: ");
        buf.push_str(&self.visible().to_string());
        buf
    }
}

/// `LocaleKeyFactory.VISIBLE`.
pub const VISIBLE: bool = true;
/// `LocaleKeyFactory.INVISIBLE`.
pub const INVISIBLE: bool = false;

/// `LocaleKeyFactory.getDisplayName(String id, ULocale locale)`: the id
/// itself for a `null` locale.
///
/// Not ported yet for a non-null locale: Java returns `new
/// ULocale(id).getDisplayName(locale)`, which needs `LocaleDisplayNames`
/// (outside the ported closure; see the rmp task #644 log). The port
/// throws `UnsupportedOperationException` naming the missing member there.
///
/// # Errors
///
/// `UnsupportedOperationException` for a non-null locale.
pub fn locale_key_factory_display_name(id: &str, locale: Option<&ULocale>) -> Result<Option<String>> {
    match locale {
        None => Ok(Some(id.to_string())),
        Some(_) => Err(LuceneError::unsupported_operation(
            "ULocale.getDisplayName(ULocale) is not ported (LocaleDisplayNames)",
        )),
    }
}

/// A `LocaleKeyFactory` that just returns a single object for a kind and
/// locale.
///
/// Equivalent to `ICULocaleService.SimpleLocaleKeyFactory`.
pub struct SimpleLocaleKeyFactory {
    name: Option<String>,
    visible: bool,
    obj: ServiceObject,
    id: String,
    kind: i32,
}

impl SimpleLocaleKeyFactory {
    /// `new SimpleLocaleKeyFactory(Object obj, ULocale locale, int kind,
    /// boolean visible, String name)`.
    pub fn new(obj: ServiceObject, locale: &ULocale, kind: i32, visible: bool, name: Option<&str>) -> Self {
        Self {
            name: name.map(str::to_string),
            visible,
            obj,
            id: locale.get_base_name(),
            kind,
        }
    }
}

impl LocaleKeyFactory for SimpleLocaleKeyFactory {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn visible(&self) -> bool {
        self.visible
    }

    fn is_supported_id(&self, id_to_check: &str) -> Result<bool> {
        Ok(self.id == id_to_check)
    }
}

impl Factory for SimpleLocaleKeyFactory {
    /// `create(Key key, ICUService service)`.
    fn create(&self, key: &dyn Key, _service: &ICUService) -> Result<Option<ServiceObject>> {
        let Some(lkey) = key.as_any().downcast_ref::<LocaleKey>() else {
            return Ok(None);
        };
        if self.kind != LocaleKey::KIND_ANY && self.kind != lkey.kind() {
            return Ok(None);
        }
        if lkey.current_id().as_deref() != Some(&self.id) {
            return Ok(None);
        }
        Ok(Some(self.obj.clone()))
    }

    /// `updateVisibleIDs(Map<String, Factory> result)`.
    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        if self.visible {
            let id = self.id.clone();
            result.insert(id, self);
        } else {
            result.remove(&self.id);
        }
        Ok(())
    }

    fn get_display_name(&self, id: &str, locale: Option<&ULocale>) -> Result<Option<String>> {
        locale_key_factory_display_name(id, locale)
    }

    fn to_java_string(&self) -> String {
        format!("{}, id: {}, kind: {}", self.lkf_to_string(), self.id, self.kind)
    }
}

/// A `LocaleKeyFactory` that creates a resource bundle for each supported
/// locale of a bundle name.
///
/// Equivalent to `ICULocaleService.ICUResourceBundleFactory`; `loader()`
/// is the class loader of the factory's class (`ICULocaleService`'s,
/// unless `class` names a subclass).
pub struct ICUResourceBundleFactory {
    /// `bundleName` (protected final).
    pub bundle_name: String,
    class: JavaClass,
}

impl Default for ICUResourceBundleFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ICUResourceBundleFactory {
    /// `new ICUResourceBundleFactory()`: the ICU data.
    pub fn new() -> Self {
        Self::with_bundle_name(ICUData::ICU_BASE_NAME)
    }

    /// `new ICUResourceBundleFactory(String bundleName)`.
    pub fn with_bundle_name(bundle_name: &str) -> Self {
        Self {
            bundle_name: bundle_name.to_string(),
            class: JavaClass::new("com.ibm.icu.impl.ICULocaleService$ICUResourceBundleFactory"),
        }
    }

    /// `loader()` (protected).
    pub fn loader(&self) -> ClassLoader {
        ClassLoaderUtil::get_class_loader_of(&self.class)
    }
}

impl LocaleKeyFactory for ICUResourceBundleFactory {
    fn name(&self) -> Option<&str> {
        None
    }

    fn visible(&self) -> bool {
        true
    }

    /// `getSupportedIDs()`: the full locale name set of the bundle.
    fn get_supported_ids(&self) -> Result<HashSet<String>> {
        Ok((*ICUResourceBundle::get_full_locale_name_set_of(&self.bundle_name, self.loader())?).clone())
    }

    /// `handleCreate(ULocale loc, int kind, ICUService service)`.
    fn handle_create(&self, loc: &ULocale, _kind: i32, _service: &ICUService) -> Result<Option<ServiceObject>> {
        let b = UResourceBundle::get_bundle_instance_for_loader(Some(&self.bundle_name), Some(loc), self.loader())?;
        Ok(Some(Arc::new(b)))
    }
}

impl Factory for ICUResourceBundleFactory {
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>> {
        self.lkf_create(key, service)
    }

    /// `updateVisibleIDs(Map<String, Factory> result)`: only the visible
    /// ids (the available locale name set).
    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        let visible_ids = ICUResourceBundle::get_available_locale_name_set_of(&self.bundle_name, self.loader())?; // only visible ids
        for id in visible_ids.iter() {
            result.insert(id.clone(), self.clone());
        }
        Ok(())
    }

    fn get_display_name(&self, id: &str, locale: Option<&ULocale>) -> Result<Option<String>> {
        locale_key_factory_display_name(id, locale)
    }

    fn to_java_string(&self) -> String {
        format!("{}, bundle: {}", self.lkf_to_string(), self.bundle_name)
    }
}
