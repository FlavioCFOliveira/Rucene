//! JVM introspection ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`HotspotVMOptions`] | `HotspotVMOptions` |
//! | [`MethodClass`] / [`VirtualMethod`] | `VirtualMethod<C>` |
//!
//! Both classes exist to ask the running JVM questions about itself. Rust has
//! neither a JVM nor runtime reflection, so each is ported to the construct
//! that answers the same question here; the divergences are stated on the
//! items themselves.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{LuceneError, Result};

// ---------------------------------------------------------------------------
// HotspotVMOptions
// ---------------------------------------------------------------------------

/// Access to HotSpot VM options.
///
/// Port of the package-private `org.apache.lucene.util.HotspotVMOptions`,
/// public here because Rust has no package visibility between sibling modules.
///
/// **Divergence from Lucene 10.5.0.** Java reflects on
/// `com.sun.management.HotSpotDiagnosticMXBean` to read `-XX:` options, and
/// installs `name -> Optional.empty()` plus `IS_HOTSPOT_VM = false` whenever
/// that bean is absent — logging a warning that "Lucene cannot optimize
/// algorithms or calculate object sizes for JVMs that are not based on Hotspot
/// or a compatible implementation". A Rust binary is permanently in that
/// branch: it is not running on a HotSpot VM, and there are no `-XX:` options
/// to read. This port therefore *is* Java's fallback path, reproduced exactly,
/// including the warning, rather than inventing a source of VM options that
/// Lucene never had.
pub struct HotspotVMOptions;

/// Emits the warning at most once, as a static initialiser would.
static WARNED: OnceLock<()> = OnceLock::new();

impl HotspotVMOptions {
    /// The warning Java logs when the HotSpot diagnostic bean is unavailable.
    pub const NOT_HOTSPOT_WARNING: &'static str =
        "Lucene cannot optimize algorithms or calculate object sizes for JVMs that are not based \
         on Hotspot or a compatible implementation.";

    /// Whether the process runs on a HotSpot-compatible VM.
    ///
    /// Equivalent to `HotspotVMOptions.IS_HOTSPOT_VM`; always `false` here.
    pub fn is_hotspot_vm() -> bool {
        Self::warn_once();
        false
    }

    /// Returns the value of a `-XX:` VM option.
    ///
    /// Equivalent to `HotspotVMOptions.get(String)`; always `None` here.
    ///
    /// # Panics
    ///
    /// Panics when `name` is empty, standing in for Java's
    /// `Objects.requireNonNull(name, "name")`.
    pub fn get(name: &str) -> Option<String> {
        assert!(!name.is_empty(), "name");
        Self::warn_once();
        None
    }

    fn warn_once() {
        WARNED.get_or_init(|| {
            log::warn!("{}", Self::NOT_HOTSPOT_WARNING);
        });
    }
}

// ---------------------------------------------------------------------------
// VirtualMethod
// ---------------------------------------------------------------------------

/// A node in a class hierarchy, carrying the methods it declares.
///
/// Rust has neither inheritance nor reflection, so [`VirtualMethod`] cannot
/// interrogate real types. This models the two things it inspects — the
/// superclass chain and the set of methods each class declares — so that
/// Lucene's algorithm can be reproduced on it verbatim. Classes are compared by
/// identity, matching Java's `==` on `Class` objects.
#[derive(Debug)]
pub struct MethodClass {
    name: String,
    super_class: Option<Arc<MethodClass>>,
    declared_methods: HashSet<String>,
}

impl MethodClass {
    /// Creates a root class declaring `declared_methods`.
    pub fn new<I, S>(name: impl Into<String>, declared_methods: I) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Arc::new(Self {
            name: name.into(),
            super_class: None,
            declared_methods: declared_methods.into_iter().map(Into::into).collect(),
        })
    }

    /// Creates a class extending `super_class` and declaring
    /// `declared_methods`.
    pub fn extending<I, S>(
        name: impl Into<String>,
        super_class: Arc<MethodClass>,
        declared_methods: I,
    ) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Arc::new(Self {
            name: name.into(),
            super_class: Some(super_class),
            declared_methods: declared_methods.into_iter().map(Into::into).collect(),
        })
    }

    /// Returns this class's name.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Returns this class's superclass, if any.
    ///
    /// Equivalent to `Class.getSuperclass()`.
    pub fn get_superclass(&self) -> Option<&Arc<MethodClass>> {
        self.super_class.as_ref()
    }

    /// Returns whether this class declares `method`.
    ///
    /// Equivalent to `Class.getDeclaredMethod(String, Class...)` succeeding.
    pub fn has_declared_method(&self, method: &str) -> bool {
        self.declared_methods.contains(method)
    }

    /// Returns whether `subclazz` is this class or descends from it.
    ///
    /// Equivalent to `Class.isAssignableFrom(Class)`.
    pub fn is_assignable_from(self: &Arc<Self>, subclazz: &Arc<MethodClass>) -> bool {
        let mut current = Some(subclazz.clone());
        while let Some(c) = current {
            if Arc::ptr_eq(&c, self) {
                return true;
            }
            current = c.super_class.clone();
        }
        false
    }
}

/// The set of `(base class, method)` pairs already claimed, enforcing the
/// singleton rule. Java uses a synchronised `HashSet<Method>`.
static SINGLETON_SET: Mutex<Option<HashSet<(String, String)>>> = Mutex::new(None);

/// Measures how deeply a method is overridden below a base class.
///
/// Port of `org.apache.lucene.util.VirtualMethod`. Lucene uses it to decide
/// whether a subclass overrides a deprecated method, so that the correct
/// variant is called; see the class javadoc for why the instance must be a
/// singleton assigned to a static final member.
pub struct VirtualMethod {
    base_class: Arc<MethodClass>,
    method: String,
    /// Java memoises with a `ClassValue`; a map keyed by class identity is the
    /// direct equivalent.
    distance_of_class: Mutex<HashMap<usize, i32>>,
}

impl std::fmt::Debug for VirtualMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualMethod")
            .field("base_class", &self.base_class.name)
            .field("method", &self.method)
            .finish()
    }
}

impl VirtualMethod {
    /// Creates a `VirtualMethod` instance for `method` declared by
    /// `base_class`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `base_class` does not
    /// declare `method`, and [`LuceneError::UnsupportedOperation`] when an
    /// instance for the same pair already exists — both reproducing Lucene's
    /// messages, which enforce that instances are singletons.
    pub fn new(base_class: Arc<MethodClass>, method: impl Into<String>) -> Result<Self> {
        let method = method.into();
        if !base_class.has_declared_method(&method) {
            return Err(LuceneError::IllegalArgument(format!(
                "{} has no such method: {method}",
                base_class.name
            )));
        }
        let key = (base_class.name.clone(), method.clone());
        {
            let mut guard = SINGLETON_SET
                .lock()
                .expect("INVARIANT: the singleton set mutex is never poisoned");
            let set = guard.get_or_insert_with(HashSet::new);
            if !set.insert(key) {
                return Err(LuceneError::UnsupportedOperation(
                    "VirtualMethod instances must be singletons and therefore assigned to static \
                     final members in the same class, they use as baseClass ctor param."
                        .to_string(),
                ));
            }
        }
        Ok(Self {
            base_class,
            method,
            distance_of_class: Mutex::new(HashMap::new()),
        })
    }

    /// Returns how many classes between `subclazz` and the base class override
    /// the method, memoising the answer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `subclazz` does not
    /// descend from the base class.
    pub fn get_implementation_distance(&self, subclazz: &Arc<MethodClass>) -> Result<i32> {
        let key = Arc::as_ptr(subclazz) as *const () as usize;
        if let Some(&d) = self
            .distance_of_class
            .lock()
            .expect("INVARIANT: the distance cache mutex is never poisoned")
            .get(&key)
        {
            return Ok(d);
        }
        let distance = self.reflect_implementation_distance(subclazz)?;
        self.distance_of_class
            .lock()
            .expect("INVARIANT: the distance cache mutex is never poisoned")
            .insert(key, distance);
        Ok(distance)
    }

    /// Returns whether `subclazz` overrides the method.
    ///
    /// # Errors
    ///
    /// As [`VirtualMethod::get_implementation_distance`].
    pub fn is_overridden_as_of(&self, subclazz: &Arc<MethodClass>) -> Result<bool> {
        Ok(self.get_implementation_distance(subclazz)? > 0)
    }

    /// Computes the implementation distance without consulting the cache.
    ///
    /// Equivalent to the package-private
    /// `VirtualMethod.reflectImplementationDistance`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `subclazz` does not
    /// descend from the base class.
    pub fn reflect_implementation_distance(&self, subclazz: &Arc<MethodClass>) -> Result<i32> {
        if !self.base_class.is_assignable_from(subclazz) {
            return Err(LuceneError::IllegalArgument(format!(
                "{} is not a subclass of {}",
                subclazz.name, self.base_class.name
            )));
        }
        let mut overridden = false;
        let mut distance = 0i32;
        let mut clazz = Some(subclazz.clone());
        while let Some(current) = clazz {
            if Arc::ptr_eq(&current, &self.base_class) {
                break;
            }
            // Look the method up; mark as overridden on success.
            if !overridden && current.has_declared_method(&self.method) {
                overridden = true;
            }
            // Increment the distance once the method is known to be overridden.
            if overridden {
                distance += 1;
            }
            clazz = current.super_class.clone();
        }
        Ok(distance)
    }

    /// Compares the implementation distance of `clazz` under `m1` and `m2`.
    ///
    /// Equivalent to
    /// `VirtualMethod.compareImplementationDistance(Class, VirtualMethod, VirtualMethod)`.
    ///
    /// # Errors
    ///
    /// As [`VirtualMethod::get_implementation_distance`].
    pub fn compare_implementation_distance(
        clazz: &Arc<MethodClass>,
        m1: &VirtualMethod,
        m2: &VirtualMethod,
    ) -> Result<std::cmp::Ordering> {
        Ok(m1
            .get_implementation_distance(clazz)?
            .cmp(&m2.get_implementation_distance(clazz)?))
    }
}
