// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ResourceBundleWrapper`.

use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;

use crate::jdk::{ClassLoader, InputStream};
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::class_loader_util::ClassLoaderUtil;
use crate::r#impl::icu_debug::ICUDebug;
use crate::r#impl::soft_cache::SoftCache;
use crate::util::u_locale::ULocale;
use crate::util::u_resource_bundle::{
    is_missing_resource, missing_resource, ResourceObject, UResourceBundle,
};

/// `DEBUG`: flag for enabling/disabling debugging code.
static DEBUG: LazyLock<bool> = LazyLock::new(|| ICUDebug::enabled_arg("resourceBundleWrapper"));

/// `ResourceBundleWrapper.Loader` (private).
type Loader = Box<dyn FnOnce() -> Result<Option<Arc<ResourceBundleWrapper>>>>;

/// `BUNDLE_CACHE`.
static BUNDLE_CACHE: LazyLock<SoftCache<String, Arc<ResourceBundleWrapper>, Loader>> =
    LazyLock::new(|| SoftCache::new(|_unused_key: &String, loader: Loader| loader()));

/// The `java.util.PropertyResourceBundle` a wrapper holds: the key-value
/// pairs of a `.properties` resource.
///
/// Equivalent to the subset of `java.util.PropertyResourceBundle` the
/// wrapper calls: `getObject` and `getKeys`. Java's `lookup` is
/// `new HashMap(properties)`, filled in the iteration order of the
/// `Properties` it loaded (a `ConcurrentHashMap`, reproduced by
/// [`ChmOrder`]); [`JavaHashMap`] reproduces the `HashMap`'s key order.
#[derive(Debug)]
pub struct PropertyResourceBundle {
    lookup: JavaHashMap<String>,
}

impl PropertyResourceBundle {
    /// `new PropertyResourceBundle(InputStream stream)`.
    ///
    /// # Errors
    ///
    /// The errors of `Properties.load` and of the stream.
    pub fn new(stream: &mut dyn InputStream) -> Result<Self> {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = stream.read(&mut buf, 0, 8192)?;
            if n < 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n as usize]);
        }
        // PropertyResourceBundle reads UTF-8, ISO-8859-1 when malformed.
        let text: Vec<u16> = match std::str::from_utf8(&bytes) {
            Ok(t) => t.encode_utf16().collect(),
            Err(_) => bytes.iter().map(|&b| b as u16).collect(),
        };
        let mut properties = ChmOrder::new();
        for (k, v) in rucene_queryparser::jdk::resource_bundle::load_pairs(&text)? {
            properties.put(k, v);
        }
        // HashMap(Map m): putMapEntries sizes the table for m.size().
        let ft = (properties.count as f32 / 0.75f32) + 1.0f32;
        let mut lookup = JavaHashMap::with_initial_capacity(ft as i32);
        for (k, v) in properties.into_entries() {
            lookup.insert(k, v);
        }
        Ok(Self { lookup })
    }

    /// `getObject(String key)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the key is absent.
    pub fn get_object(&self, key: &str) -> Result<String> {
        match self.lookup.get(key) {
            Some(v) => Ok(v.clone()),
            None => Err(missing_resource(
                format!(
                    "Can't find resource for bundle java.util.PropertyResourceBundle, key {key}"
                ),
                "java.util.PropertyResourceBundle",
                key,
            )),
        }
    }

    /// `getKeys()`.
    pub fn get_keys(&self) -> Vec<String> {
        self.lookup.keys().cloned().collect()
    }
}

/// The iteration order of `java.util.Properties`, whose entries live in a
/// `new ConcurrentHashMap<>(8)` (JDK 9 and later): a single-writer model of
/// everything that decides `ConcurrentHashMap`'s order. `spread` over
/// `String.hashCode()`, tail insertion in a list bin and head insertion in a
/// tree bin, `treeifyBin`'s pre-sizing below 64 bins, the resize threshold
/// `n - (n >>> 2)`, and `transfer`'s bin split (the `lastRun` tail kept, the
/// nodes before it prepended in reverse; tree bins split in order).
///
/// Adapted from the workspace's `rucene-benchmark` `jdk::properties`
/// model of the same JDK 25 class (this crate cannot depend on that crate).
struct ChmOrder {
    table: Vec<ChmBin>,
    size_ctl: usize,
    count: usize,
}

#[derive(Clone, Default)]
struct ChmBin {
    nodes: Vec<(i32, String, String)>,
    tree: bool,
}

impl ChmOrder {
    /// `new ConcurrentHashMap<>(8)`: `sizeCtl` = `tableSizeFor((int) (1.0 + 8 / 0.75f))` = 16.
    fn new() -> Self {
        Self {
            table: Vec::new(),
            size_ctl: ((1.0 + 8.0 / 0.75f32 as f64) as usize)
                .max(1)
                .next_power_of_two(),
            count: 0,
        }
    }

    fn spread(h: i32) -> i32 {
        (h ^ ((h as u32) >> 16) as i32) & 0x7fff_ffff
    }

    /// `putVal(key, value, false)` by a single thread.
    fn put(&mut self, key: String, value: String) {
        let hash = Self::spread(rucene::util::java_collections::java_string_hash_code(&key));
        if self.table.is_empty() {
            let n = self.size_ctl.max(1);
            self.table = vec![ChmBin::default(); n];
            self.size_ctl = n - (n >> 2);
        }
        let n = self.table.len();
        let i = (hash as usize) & (n - 1);
        let bin = &mut self.table[i];
        let mut bin_count = 0usize;
        if let Some(node) = bin
            .nodes
            .iter_mut()
            .find(|node| node.0 == hash && node.1 == key)
        {
            node.2 = value;
            return;
        }
        if bin.nodes.is_empty() {
            bin.nodes.push((hash, key, value));
        } else if bin.tree {
            bin.nodes.insert(0, (hash, key, value));
            bin_count = 2;
        } else {
            bin_count = bin.nodes.len();
            bin.nodes.push((hash, key, value));
        }
        if bin_count >= 8 {
            // treeifyBin
            if n < 64 {
                let c = (n << 1) + (n << 1 >> 1) + 1;
                let c = c.max(1).next_power_of_two();
                while c > self.size_ctl {
                    self.transfer();
                }
            } else if !self.table[i].nodes.is_empty() && !self.table[i].tree {
                self.table[i].tree = true;
            }
        }
        self.count += 1;
        while self.count >= self.size_ctl {
            self.transfer();
        }
    }

    /// `transfer(tab, null)` by a single thread.
    fn transfer(&mut self) {
        let n = self.table.len();
        let mut next = vec![ChmBin::default(); n << 1];
        let old = std::mem::take(&mut self.table);
        for (i, bin) in old.into_iter().enumerate().rev() {
            if bin.nodes.is_empty() {
                continue;
            }
            if bin.tree {
                let (lo, hi): (Vec<_>, Vec<_>) = bin
                    .nodes
                    .into_iter()
                    .partition(|node| (node.0 as usize) & n == 0);
                let lo_tree = lo.len() > 6;
                let hi_tree = hi.len() > 6;
                next[i] = ChmBin {
                    nodes: lo,
                    tree: lo_tree,
                };
                next[i + n] = ChmBin {
                    nodes: hi,
                    tree: hi_tree,
                };
            } else {
                let nodes = bin.nodes;
                let mut run_bit = (nodes[0].0 as usize) & n;
                let mut last_run = 0;
                for (p, node) in nodes.iter().enumerate().skip(1) {
                    let b = (node.0 as usize) & n;
                    if b != run_bit {
                        run_bit = b;
                        last_run = p;
                    }
                }
                let tail = nodes[last_run..].to_vec();
                let (mut ln, mut hn) = if run_bit == 0 {
                    (tail, Vec::new())
                } else {
                    (Vec::new(), tail)
                };
                for node in &nodes[..last_run] {
                    if (node.0 as usize) & n == 0 {
                        ln.insert(0, node.clone());
                    } else {
                        hn.insert(0, node.clone());
                    }
                }
                next[i] = ChmBin {
                    nodes: ln,
                    tree: false,
                };
                next[i + n] = ChmBin {
                    nodes: hn,
                    tree: false,
                };
            }
        }
        self.table = next;
        self.size_ctl = (n << 1) - (n >> 1);
    }

    /// `entrySet()` in iteration order.
    fn into_entries(self) -> impl Iterator<Item = (String, String)> {
        self.table
            .into_iter()
            .flat_map(|bin| bin.nodes.into_iter())
            .map(|(_, k, v)| (k, v))
    }
}

/// A `UResourceBundle` over a plain Java resource bundle, for base names
/// that are not ICU data.
///
/// Equivalent to `com.ibm.icu.impl.ResourceBundleWrapper`. Java first
/// tries to load the bundle as a class (`root.loadClass(name)`, a
/// `ListResourceBundle` subclass); a Rust crate loads no classes by name,
/// so that always takes Java's `ClassNotFoundException` branch and the
/// bundle is read from the `.properties` resource, as Java then does.
#[derive(Debug)]
pub struct ResourceBundleWrapper {
    bundle: PropertyResourceBundle,
    locale_id: String,
    base_name: String,
    keys: Vec<String>,
    parent: Option<Arc<ResourceBundleWrapper>>,
}

impl ResourceBundleWrapper {
    /// `handleGetObject(String aKey)` (protected): the object of `aKey` in
    /// this bundle or its parents.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the key is found nowhere.
    pub fn handle_get_object(&self, a_key: &str) -> Result<ResourceObject> {
        let mut current: Option<&ResourceBundleWrapper> = Some(self);
        let mut obj: Option<String> = None;
        while let Some(c) = current {
            match c.bundle.get_object(a_key) {
                Ok(o) => {
                    obj = Some(o);
                    break;
                }
                Err(ex) if is_missing_resource(&ex) => current = c.parent.as_deref(),
                Err(ex) => return Err(ex),
            }
        }
        match obj {
            Some(o) => Ok(ResourceObject::String(o.encode_utf16().collect())),
            None => Err(missing_resource(
                format!(
                    "Can't find resource for bundle {}, key {}",
                    self.base_name, a_key
                ),
                "com.ibm.icu.impl.ResourceBundleWrapper",
                a_key,
            )),
        }
    }

    /// `getKeys()`.
    pub fn get_keys(&self) -> Vec<String> {
        self.keys.clone()
    }

    fn init_keys_vector(&mut self) {
        let mut keys: Vec<String> = Vec::new();
        for elem in self.bundle.get_keys() {
            if !keys.contains(&elem) {
                keys.push(elem);
            }
        }
        let mut current = self.parent.clone();
        while let Some(c) = current {
            for elem in c.bundle.get_keys() {
                if !keys.contains(&elem) {
                    keys.push(elem);
                }
            }
            current = c.parent.clone();
        }
        self.keys = keys;
    }

    /// `getLocaleID()` (protected).
    pub fn get_locale_id(&self) -> String {
        self.locale_id.clone()
    }

    /// `getBaseName()` (protected): the wrapped bundle's class name with
    /// `/` separators.
    pub fn get_base_name(&self) -> String {
        "java.util.PropertyResourceBundle".replace('.', "/")
    }

    /// `getULocale()`: `new ULocale(localeID)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("variants is too long")`.
    pub fn get_u_locale(&self) -> Result<ULocale> {
        ULocale::new(&self.locale_id)
    }

    /// `getParent()`.
    pub fn get_parent(&self) -> Option<UResourceBundle> {
        self.parent.clone().map(UResourceBundle::Wrapper)
    }

    /// `getBundleInstance(String baseName, String localeID, ClassLoader
    /// root, boolean disableFallback)`: for the super class's
    /// `instantiateBundle` method.
    ///
    /// # Errors
    ///
    /// `MissingResourceException("Could not find the bundle ...")`.
    pub fn get_bundle_instance(
        base_name: &str,
        locale_id: &str,
        root: Option<ClassLoader>,
        disable_fallback: bool,
    ) -> Result<Arc<ResourceBundleWrapper>> {
        let root = root.unwrap_or_else(ClassLoaderUtil::get_class_loader);
        let b = if disable_fallback {
            Self::instantiate_bundle(base_name, locale_id, None, root, disable_fallback)?
        } else {
            Self::instantiate_bundle(
                base_name,
                locale_id,
                Some(&ULocale::get_default().get_base_name()),
                root,
                disable_fallback,
            )?
        };
        match b {
            Some(b) => Ok(b),
            None => {
                let separator = if base_name.contains('/') { "/" } else { "_" };
                Err(missing_resource(
                    format!("Could not find the bundle {base_name}{separator}{locale_id}"),
                    "",
                    "",
                ))
            }
        }
    }

    fn locale_id_starts_with_lang_subtag(locale_id: &str, lang: &str) -> bool {
        locale_id.starts_with(lang)
            && (locale_id.len() == lang.len() || locale_id.as_bytes()[lang.len()] == b'_')
    }

    fn instantiate_bundle(
        base_name: &str,
        locale_id: &str,
        default_id: Option<&str>,
        root: ClassLoader,
        disable_fallback: bool,
    ) -> Result<Option<Arc<ResourceBundleWrapper>>> {
        let name = if locale_id.is_empty() {
            base_name.to_string()
        } else {
            format!("{base_name}_{locale_id}")
        };
        let cache_key = if disable_fallback {
            name.clone()
        } else {
            format!("{}#{}", name, default_id.unwrap_or("null"))
        };
        let base_name = base_name.to_string();
        let locale_id = locale_id.to_string();
        let default_id = default_id.map(str::to_string);
        let loader: Loader = Box::new(move || -> Result<Option<Arc<ResourceBundleWrapper>>> {
            let mut parent: Option<Arc<ResourceBundleWrapper>> = None;
            let i = locale_id.rfind('_');
            let mut parent_is_root = false;
            if let Some(i) = i {
                let loc_name = &locale_id[..i];
                parent = Self::instantiate_bundle(
                    &base_name,
                    loc_name,
                    default_id.as_deref(),
                    root,
                    disable_fallback,
                )?;
            } else if !locale_id.is_empty() {
                parent = Self::instantiate_bundle(
                    &base_name,
                    "",
                    default_id.as_deref(),
                    root,
                    disable_fallback,
                )?;
                parent_is_root = true;
            }
            let mut b: Option<Arc<ResourceBundleWrapper>> = None;
            // root.loadClass(name): ClassNotFoundException.
            let load_from_properties = true;
            if load_from_properties {
                let res_name = format!("{}.properties", name.replace('.', "/"));
                let attempt = (|| -> Result<()> {
                    if let Some(mut stream) = root.get_resource_as_stream(&res_name) {
                        // make sure it is buffered
                        match PropertyResourceBundle::new(&mut stream) {
                            Ok(prb) => {
                                let mut w = ResourceBundleWrapper {
                                    bundle: prb,
                                    locale_id: locale_id.clone(),
                                    base_name: base_name.clone(),
                                    keys: Vec::new(),
                                    parent: parent.clone(),
                                };
                                w.init_keys_vector();
                                b = Some(Arc::new(w));
                            }
                            Err(_ex) => {
                                // throw away exception
                            }
                        }
                        let _ = stream.close();
                    }

                    // if a bogus locale is passed then the parent should be
                    // the default locale not the root locale!
                    if b.is_none()
                        && !disable_fallback
                        && !locale_id.is_empty()
                        && !locale_id.contains('_')
                        && !Self::locale_id_starts_with_lang_subtag(
                            default_id.as_deref().unwrap_or("null"),
                            &locale_id,
                        )
                    {
                        // localeID is only a language subtag, different from the default language.
                        b = Self::instantiate_bundle(
                            &base_name,
                            default_id.as_deref().unwrap_or("null"),
                            default_id.as_deref(),
                            root,
                            disable_fallback,
                        )?;
                    }
                    // if still could not find the bundle then return the parent
                    if b.is_none() && (!parent_is_root || !disable_fallback) {
                        b = parent.clone();
                    }
                    Ok(())
                })();
                if let Err(e) = attempt {
                    if *DEBUG {
                        println!("failure");
                        println!("{e}");
                    }
                }
            }
            if b.is_none() && *DEBUG {
                println!("Returning null for {base_name}_{locale_id}");
            }
            Ok(b)
        });
        BUNDLE_CACHE.get_instance(cache_key, loader)
    }
}
