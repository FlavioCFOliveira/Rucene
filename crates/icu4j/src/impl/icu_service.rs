// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUService`.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use rucene::error::{LuceneError, Result};

use crate::r#impl::icu_debug::ICUDebug;
use crate::r#impl::icu_notifier::ICUNotifier;
use crate::r#impl::icurw_lock::ICURWLock;
use crate::util::u_locale::{Category, ULocale};
use crate::util::u_resource_bundle::null_str;

/// `DEBUG`.
static DEBUG: LazyLock<bool> = LazyLock::new(|| ICUDebug::enabled_arg("service"));

/// A service object: Java's `Object` results of factories.
pub type ServiceObject = Arc<dyn Any + Send + Sync>;

/// Keys are used to communicate with factories to generate an instance of
/// the service.
///
/// Equivalent to the class `ICUService.Key`, whose overridable methods are
/// the trait's provided methods; [`BaseKey`] is Java's `new Key(id)`.
pub trait Key: Send + Sync + Any {
    /// `id()` (final): the original id used to construct this key.
    fn id(&self) -> &str;

    /// `canonicalID()`: the canonical version of the original id.
    fn canonical_id(&self) -> Option<String> {
        Some(self.id().to_string())
    }

    /// `currentID()`: the (canonical) current id, `None` if no current id.
    fn current_id(&self) -> Option<String> {
        self.canonical_id()
    }

    /// `currentDescriptor()`: the current descriptor, `/` and the current
    /// id by default.
    fn current_descriptor(&self) -> Option<String> {
        Some(format!("/{}", null_str(self.current_id().as_deref())))
    }

    /// `fallback()`: whether the key has fallen back to another id.
    fn fallback(&mut self) -> bool {
        false
    }

    /// `isFallbackOf(String idToCheck)`.
    fn is_fallback_of(&self, id_to_check: &str) -> bool {
        self.canonical_id().as_deref() == Some(id_to_check)
    }

    /// `this` as `Any`, for `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}

/// `new ICUService.Key(String id)`.
#[derive(Debug, Clone)]
pub struct BaseKey {
    id: String,
}

impl BaseKey {
    /// `new Key(String id)`.
    pub fn new(id: &str) -> Self {
        Self { id: id.to_string() }
    }
}

impl Key for BaseKey {
    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The visible id map of a service: id to factory.
pub type VisibleIDMap = HashMap<String, Arc<dyn Factory>>;

/// Factories generate the service objects maintained by the service.
///
/// Equivalent to the interface `ICUService.Factory`.
pub trait Factory: Send + Sync + Any {
    /// `create(Key key, ICUService service)`: the service object for the
    /// key's current id, `None` when not supported.
    ///
    /// # Errors
    ///
    /// Whatever the factory throws.
    fn create(&self, key: &dyn Key, service: &ICUService) -> Result<Option<ServiceObject>>;

    /// `updateVisibleIDs(Map<String, Factory> result)`: adds (or removes)
    /// this factory's visible ids.
    ///
    /// # Errors
    ///
    /// Whatever the factory throws.
    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()>;

    /// `getDisplayName(String id, ULocale locale)`.
    ///
    /// # Errors
    ///
    /// Whatever the factory throws.
    fn get_display_name(&self, id: &str, locale: Option<&ULocale>) -> Result<Option<String>>;

    /// `toString()`.
    fn to_java_string(&self) -> String;
}

/// A default implementation of factory: returns a fixed object for one id.
///
/// Equivalent to `ICUService.SimpleFactory`.
pub struct SimpleFactory {
    /// `instance`.
    pub instance: ServiceObject,
    /// `id`.
    pub id: String,
    /// `visible`.
    pub visible: bool,
}

impl SimpleFactory {
    /// `new SimpleFactory(Object instance, String id)`: visible.
    pub fn new(instance: ServiceObject, id: &str) -> Self {
        Self::with_visible(instance, id, true)
    }

    /// `new SimpleFactory(Object instance, String id, boolean visible)`
    /// (Java's null checks are the types' guarantees).
    pub fn with_visible(instance: ServiceObject, id: &str, visible: bool) -> Self {
        Self {
            instance,
            id: id.to_string(),
            visible,
        }
    }
}

impl Factory for SimpleFactory {
    fn create(&self, key: &dyn Key, _service: &ICUService) -> Result<Option<ServiceObject>> {
        if key.current_id().as_deref() == Some(&self.id) {
            return Ok(Some(self.instance.clone()));
        }
        Ok(None)
    }

    fn update_visible_ids(self: Arc<Self>, result: &mut VisibleIDMap) -> Result<()> {
        if self.visible {
            let id = self.id.clone();
            result.insert(id, self);
        } else {
            result.remove(&self.id);
        }
        Ok(())
    }

    fn get_display_name(
        &self,
        identifier: &str,
        _locale: Option<&ULocale>,
    ) -> Result<Option<String>> {
        Ok(if self.visible && self.id == identifier {
            Some(identifier.to_string())
        } else {
            None
        })
    }

    fn to_java_string(&self) -> String {
        format!(
            "com.ibm.icu.impl.ICUService$SimpleFactory, id: {}, visible: {}",
            self.id, self.visible
        )
    }
}

/// A listener notified when the service's factories change.
///
/// Equivalent to the interface `ICUService.ServiceListener`.
pub trait ServiceListener: Send + Sync {
    /// `serviceChanged(ICUService service)`.
    fn service_changed(&self, service: &ICUService);
}

/// The methods a subclass of `ICUService` overrides: `createKey`,
/// `handleDefault` and `reInitializeFactories`.
pub trait ServiceOverrides: Send + Sync {
    /// `createKey(String id)`: a key for the id, `None` for `null`.
    ///
    /// # Errors
    ///
    /// Whatever the subclass throws.
    fn create_key(&self, _service: &ICUService, id: Option<&str>) -> Result<Option<Box<dyn Key>>> {
        Ok(id.map(|i| Box::new(BaseKey::new(i)) as Box<dyn Key>))
    }

    /// `handleDefault(Key key, String[] actualIDReturn)`: the default
    /// handler, `null` in `ICUService`.
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
    fn re_initialize_factories(
        &self,
        _service: &ICUService,
        factories: &mut Vec<Arc<dyn Factory>>,
    ) {
        factories.clear();
    }
}

/// The `ICUService` behaviour itself.
struct BaseOverrides;

impl ServiceOverrides for BaseOverrides {}

/// `CacheEntry` (private): the actual descriptor and the service.
#[derive(Clone)]
struct CacheEntry {
    actual_descriptor: String,
    service: ServiceObject,
}

/// `LocaleRef` (private): a display-name cache with its locale and
/// comparator.
struct LocaleRef {
    locale: ULocale,
    com: Option<DisplayNameComparator>,
    dn_cache: Arc<Vec<(String, String)>>,
}

/// A display-name comparator (Java's `Comparator<Object>`).
pub type DisplayNameComparator = Arc<dyn Fn(&str, &str) -> std::cmp::Ordering + Send + Sync>;

/// A service that maintains a list of factories, each of which can create
/// objects for ids, with fallback and caching.
///
/// Equivalent to `com.ibm.icu.impl.ICUService` (which extends
/// `ICUNotifier`). Java's subclasses override `createKey`, `handleDefault`
/// and `reInitializeFactories`; here they give a [`ServiceOverrides`].
/// The factory list, which Java guards with `factoryLock`, also sits under
/// a mutex, released while factories run.
#[allow(clippy::type_complexity)] // type mirrors the Java signature
pub struct ICUService {
    name: String,
    factory_lock: ICURWLock,
    factories: Mutex<Vec<Arc<dyn Factory>>>,
    default_size: AtomicUsize,
    cache: Mutex<Option<Arc<Mutex<HashMap<String, CacheEntry>>>>>,
    idcache: Mutex<Option<Arc<VisibleIDMap>>>,
    dnref: Mutex<Option<Arc<LocaleRef>>>,
    notifier: ICUNotifier<dyn ServiceListener>,
    overrides: Box<dyn ServiceOverrides>,
    self_ref: Weak<ICUService>,
}

impl ICUService {
    /// `new ICUService(String name)`.
    pub fn new(name: &str) -> Arc<Self> {
        Self::with_overrides(name, Box::new(BaseOverrides))
    }

    /// A service whose subclass methods are `overrides`.
    pub fn with_overrides(name: &str, overrides: Box<dyn ServiceOverrides>) -> Arc<Self> {
        Arc::new_cyclic(|w| Self {
            name: name.to_string(),
            factory_lock: ICURWLock::new(),
            factories: Mutex::new(Vec::new()),
            default_size: AtomicUsize::new(0),
            cache: Mutex::new(None),
            idcache: Mutex::new(None),
            dnref: Mutex::new(None),
            notifier: ICUNotifier::new(),
            overrides,
            self_ref: w.clone(),
        })
    }

    /// The overrides, for subclasses reading their own state.
    pub fn overrides(&self) -> &dyn ServiceOverrides {
        self.overrides.as_ref()
    }

    fn factories_snapshot(&self) -> Vec<Arc<dyn Factory>> {
        self.factories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `get(String descriptor)`.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get(&self, descriptor: Option<&str>) -> Result<Option<ServiceObject>> {
        let key = self.create_key(descriptor)?;
        self.get_key(key, None, None)
    }

    /// `get(String descriptor, String[] actualReturn)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("descriptor must not be null")`; whatever the
    /// factories throw.
    pub fn get_actual(
        &self,
        descriptor: Option<&str>,
        actual_return: Option<&mut String>,
    ) -> Result<Option<ServiceObject>> {
        let descriptor = descriptor
            .ok_or_else(|| LuceneError::null_pointer_with_message("descriptor must not be null"))?;
        let key = self.create_key(Some(descriptor))?;
        self.get_key(key, actual_return, None)
    }

    /// `getKey(Key key, String[] actualReturn, Factory factory)`: the
    /// service object for the key, trying factories after `factory` when
    /// given, with fallback and caching.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` for an unregistered `factory`; whatever the
    /// factories throw.
    #[allow(clippy::type_complexity)] // type mirrors the Java signature
    pub fn get_key(
        &self,
        key: Option<Box<dyn Key>>,
        actual_return: Option<&mut String>,
        factory: Option<&Arc<dyn Factory>>,
    ) -> Result<Option<ServiceObject>> {
        let factories = self.factories_snapshot();
        if factories.is_empty() {
            return self
                .overrides
                .handle_default(self, key.as_deref(), actual_return);
        }

        if *DEBUG {
            println!(
                "Service: {} key: {}",
                self.name,
                key.as_ref()
                    .and_then(|k| k.canonical_id())
                    .unwrap_or_else(|| "null".into())
            );
        }

        let mut key = key;
        if let Some(key) = key.as_mut() {
            // The factory list can't be modified until we're done,
            // otherwise we might update the cache with an invalid result.
            // The cache has to stay in synch with the factory list.
            self.factory_lock.acquire_read();
            let outcome = (|| -> Result<Option<(CacheEntry, Arc<Mutex<HashMap<String, CacheEntry>>>, bool, Vec<String>)>> {
                let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner()).clone(); // copy so we don't need to sync on this
                let cache = match cache {
                    Some(c) => c,
                    None => {
                        if *DEBUG {
                            println!("Service {} cache was empty", self.name);
                        }
                        // synchronized since additions and queries on the cache must be atomic
                        // they can be interleaved, though
                        Arc::new(Mutex::new(HashMap::new()))
                    }
                };

                let mut result: Option<CacheEntry> = None;
                let mut cache_descriptor_list: Vec<String> = Vec::new();
                let mut put_in_cache = false;
                let mut start_index = 0usize;
                let limit = factories.len();
                let mut cache_result = true;
                if let Some(factory) = factory {
                    for (i, f) in factories.iter().enumerate() {
                        if std::ptr::eq(Arc::as_ptr(f) as *const (), Arc::as_ptr(factory) as *const ()) {
                            start_index = i + 1;
                            break;
                        }
                    }
                    if start_index == 0 {
                        return Err(LuceneError::illegal_state(format!(
                            "Factory {}not registered with service: {}",
                            factory.to_java_string(),
                            self.to_java_string()
                        )));
                    }
                    cache_result = false;
                }

                'outer: loop {
                    let current_descriptor = key.current_descriptor().unwrap_or_else(|| "null".to_string());
                    if let Some(r) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&current_descriptor) {
                        result = Some(r.clone());
                        break 'outer;
                    }

                    // first test of cache failed, so we'll have to update
                    // the cache if we eventually succeed-- that is, if we're
                    // going to update the cache at all.
                    put_in_cache = cache_result;

                    let mut index = start_index;
                    while index < limit {
                        let f = &factories[index];
                        index += 1;
                        if *DEBUG {
                            println!("trying factory[{}] {}", index - 1, f.to_java_string());
                        }
                        if let Some(service) = f.create(key.as_ref(), self)? {
                            result = Some(CacheEntry {
                                actual_descriptor: current_descriptor.clone(),
                                service,
                            });
                            break 'outer;
                        }
                    }

                    // prepare to load the cache with all additional ids that
                    // will resolve to result, assuming we'll succeed.  We
                    // don't want to keep querying on an id that's going to
                    // fallback to the one that succeeded, we want to hit the
                    // cache the first time next goaround.
                    cache_descriptor_list.push(current_descriptor);
                    if !key.fallback() {
                        break;
                    }
                }
                Ok(result.map(|r| (r, cache, put_in_cache, cache_descriptor_list)))
            })();
            let outcome = match outcome {
                Ok(o) => o,
                Err(e) => {
                    self.factory_lock.release_read();
                    return Err(e);
                }
            };
            if let Some((result, cache, put_in_cache, cache_descriptor_list)) = outcome {
                if put_in_cache {
                    {
                        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
                        c.insert(result.actual_descriptor.clone(), result.clone());
                        for desc in cache_descriptor_list {
                            c.insert(desc, result.clone());
                        }
                    }
                    // Atomic update.  We held the read lock all this time
                    // so we know our cache is consistent with the factory list.
                    // We might stomp over a cache that some other thread
                    // rebuilt, but that's the breaks.  They're both good.
                    *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(cache);
                }

                if let Some(actual_return) = actual_return {
                    // strip null prefix
                    *actual_return = match result.actual_descriptor.strip_prefix('/') {
                        Some(s) => s.to_string(),
                        None => result.actual_descriptor.clone(),
                    };
                }

                self.factory_lock.release_read();
                return Ok(Some(result.service));
            }
            self.factory_lock.release_read();
            if *DEBUG {
                println!("not found in service: {}", self.name);
            }
            return self
                .overrides
                .handle_default(self, Some(key.as_ref()), actual_return);
        }

        self.overrides.handle_default(self, None, actual_return)
    }

    /// `getVisibleIDs()`.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_visible_ids(&self) -> Result<HashSet<String>> {
        self.get_visible_ids_matching(None)
    }

    /// `getVisibleIDs(String matchID)`: the visible ids the match id falls
    /// back from (all when `None`).
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_visible_ids_matching(&self, match_id: Option<&str>) -> Result<HashSet<String>> {
        let result: HashSet<String> = self.get_visible_id_map()?.keys().cloned().collect();
        match self.create_key(match_id)? {
            Some(fallback_key) => Ok(result
                .into_iter()
                .filter(|id| fallback_key.is_fallback_of(id))
                .collect()),
            None => Ok(result),
        }
    }

    fn get_visible_id_map(&self) -> Result<Arc<VisibleIDMap>> {
        let mut idcache = self.idcache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = idcache.as_ref() {
            return Ok(m.clone());
        }
        self.factory_lock.acquire_read();
        let built = (|| -> Result<VisibleIDMap> {
            let mut mutable_map: VisibleIDMap = HashMap::new();
            for f in self.factories_snapshot().into_iter().rev() {
                f.update_visible_ids(&mut mutable_map)?;
            }
            Ok(mutable_map)
        })();
        self.factory_lock.release_read();
        let result = Arc::new(built?);
        *idcache = Some(result.clone());
        Ok(result)
    }

    /// `getDisplayName(String id)`: in the default display locale.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_display_name(&self, id: &str) -> Result<Option<String>> {
        self.get_display_name_in(id, Some(&ULocale::get_default_category(Category::Display)))
    }

    /// `getDisplayName(String id, ULocale locale)`.
    ///
    /// # Errors
    ///
    /// Whatever the factories throw.
    pub fn get_display_name_in(
        &self,
        id: &str,
        locale: Option<&ULocale>,
    ) -> Result<Option<String>> {
        let m = self.get_visible_id_map()?;
        if let Some(f) = m.get(id) {
            return f.get_display_name(id, locale);
        }

        let mut key = self
            .create_key(Some(id))?
            .ok_or_else(LuceneError::null_pointer)?;
        while key.fallback() {
            if let Some(f) = key.current_id().and_then(|c| m.get(&c)) {
                return f.get_display_name(id, locale);
            }
        }

        Ok(None)
    }

    /// `getDisplayNames(ULocale locale, Comparator<Object> com, String
    /// matchID)`: display name to id, in the order of `com` (the natural
    /// `String` order when `None`), as a `TreeMap` keeps them (a key equal
    /// under `com` to an earlier one replaces its value); Java's
    /// `put(f.getDisplayName(...), id)` with a `null` name is the
    /// `TreeMap`'s `NullPointerException`. The cache is reused for the
    /// same locale and the same comparator (`Arc` identity, as Java's
    /// `equals` of a comparator without its own).
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a factory display name that is `null`;
    /// whatever the factories throw.
    pub fn get_display_names(
        &self,
        locale: &ULocale,
        com: Option<&DisplayNameComparator>,
        match_id: Option<&str>,
    ) -> Result<Vec<(String, String)>> {
        let mut dncache: Option<Arc<Vec<(String, String)>>> = None;
        let r = self.dnref.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(r) = &r {
            let same_com = match (&r.com, com) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            };
            if r.locale == *locale && same_com {
                dncache = Some(r.dn_cache.clone());
            }
        }

        let cmp = |a: &str, b: &str| -> std::cmp::Ordering {
            match com {
                Some(c) => c(a, b),
                None => rucene::util::java_collections::java_compare_to(a, b),
            }
        };

        let dncache = match dncache {
            Some(d) => d,
            None => {
                let mut map: Vec<(String, String)> = Vec::new();
                let m = self.get_visible_id_map()?;
                for (id, f) in m.iter() {
                    let name = f
                        .get_display_name(id, Some(locale))?
                        .ok_or_else(LuceneError::null_pointer)?;
                    match map.binary_search_by(|(k, _)| cmp(k, &name)) {
                        Ok(i) => map[i].1 = id.clone(),
                        Err(i) => map.insert(i, (name, id.clone())),
                    }
                }
                let d = Arc::new(map);
                *self.dnref.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(LocaleRef {
                    locale: locale.clone(),
                    com: com.cloned(),
                    dn_cache: d.clone(),
                }));
                d
            }
        };

        let match_key = match self.create_key(match_id)? {
            None => return Ok((*dncache).clone()),
            Some(k) => k,
        };

        Ok(dncache
            .iter()
            .filter(|(_, v)| match_key.is_fallback_of(v))
            .cloned()
            .collect())
    }

    /// `getDisplayNames()`: in the default display locale.
    ///
    /// # Errors
    ///
    /// As [`get_display_names`](Self::get_display_names).
    pub fn get_display_names_default(&self) -> Result<Vec<(String, String)>> {
        self.get_display_names(
            &ULocale::get_default_category(Category::Display),
            None,
            None,
        )
    }

    /// `factories()` (final): a snapshot of the factories.
    pub fn factories(&self) -> Vec<Arc<dyn Factory>> {
        self.factory_lock.acquire_read();
        let f = self.factories_snapshot();
        self.factory_lock.release_read();
        f
    }

    /// `registerObject(Object obj, String id, boolean visible)`.
    ///
    /// # Errors
    ///
    /// The errors of `createKey`; `NullPointerException` when it returns
    /// `null`.
    pub fn register_object(
        &self,
        obj: ServiceObject,
        id: &str,
        visible: bool,
    ) -> Result<Arc<dyn Factory>> {
        let canonical_id = self
            .create_key(Some(id))?
            .ok_or_else(LuceneError::null_pointer)?
            .canonical_id()
            .unwrap_or_else(|| "null".to_string());
        self.register_factory(Arc::new(SimpleFactory::with_visible(
            obj,
            &canonical_id,
            visible,
        )))
    }

    /// `registerFactory(Factory factory)` (final): adds the factory first.
    ///
    /// # Errors
    ///
    /// None in practice.
    pub fn register_factory(&self, factory: Arc<dyn Factory>) -> Result<Arc<dyn Factory>> {
        self.factory_lock.acquire_write();
        self.factories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(0, factory.clone());
        self.clear_caches();
        self.factory_lock.release_write();
        self.notify_changed();
        Ok(factory)
    }

    /// `unregisterFactory(Factory factory)` (final).
    pub fn unregister_factory(&self, factory: &Arc<dyn Factory>) -> bool {
        let mut result = false;
        self.factory_lock.acquire_write();
        {
            let mut f = self.factories.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(i) = f.iter().position(|x| {
                std::ptr::eq(
                    Arc::as_ptr(x) as *const (),
                    Arc::as_ptr(factory) as *const (),
                )
            }) {
                f.remove(i);
                result = true;
            }
        }
        if result {
            self.clear_caches();
        }
        self.factory_lock.release_write();
        if result {
            self.notify_changed();
        }
        result
    }

    /// `reset()` (final): reinitializes the factory list.
    pub fn reset(&self) {
        self.factory_lock.acquire_write();
        {
            let mut f = self.factories.lock().unwrap_or_else(|e| e.into_inner());
            self.overrides.re_initialize_factories(self, &mut f);
        }
        self.clear_caches();
        self.factory_lock.release_write();
        self.notify_changed();
    }

    /// `isDefault()`.
    pub fn is_default(&self) -> bool {
        self.factories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
            == self.default_size.load(Ordering::SeqCst)
    }

    /// `markDefault()` (protected).
    pub fn mark_default(&self) {
        let n = self
            .factories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        self.default_size.store(n, Ordering::SeqCst);
    }

    /// `createKey(String id)`.
    ///
    /// # Errors
    ///
    /// Whatever the subclass throws.
    pub fn create_key(&self, id: Option<&str>) -> Result<Option<Box<dyn Key>>> {
        self.overrides.create_key(self, id)
    }

    /// `clearCaches()` (protected).
    pub fn clear_caches(&self) {
        // we don't synchronize on these because methods that use them
        // copy before use, and check for changes if they modify the
        // caches.
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.idcache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.dnref.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// `clearServiceCache()` (protected).
    pub fn clear_service_cache(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// `addListener(EventListener l)` of `ICUNotifier`.
    ///
    /// # Errors
    ///
    /// None: every [`ServiceListener`] is accepted.
    pub fn add_listener(&self, l: Arc<dyn ServiceListener>) -> Result<()> {
        // acceptsListener: l instanceof ServiceListener
        self.notifier.add_listener(l, true)
    }

    /// `removeListener(EventListener l)` of `ICUNotifier`.
    pub fn remove_listener(&self, l: &Arc<dyn ServiceListener>) {
        self.notifier.remove_listener(l);
    }

    /// `notifyChanged()` of `ICUNotifier`, with `notifyListener` calling
    /// `serviceChanged(this)`.
    pub fn notify_changed(&self) {
        let w = self.self_ref.clone();
        self.notifier
            .notify_changed(Arc::new(move |l: &Arc<dyn ServiceListener>| {
                if let Some(s) = w.upgrade() {
                    l.service_changed(&s);
                }
            }));
    }

    /// `stats()`.
    pub fn stats(&self) -> String {
        match self.factory_lock.reset_stats() {
            Some(stats) => stats.to_string(),
            None => "no stats".to_string(),
        }
    }

    /// `getName()`.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// `toString()`: the class name and `{name}` (Java prefixes
    /// `Object.toString()`, whose identity hash has no counterpart).
    pub fn to_java_string(&self) -> String {
        format!("com.ibm.icu.impl.ICUService{{{}}}", self.name)
    }
}
