//! Resource and class loading ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`ResourceLoader`] | `ResourceLoader` |
//! | [`ResourceLoaderAware`] | `ResourceLoaderAware` |
//! | [`ClasspathResourceLoader`] | `ClasspathResourceLoader` |
//! | [`ModuleResourceLoader`] | `ModuleResourceLoader` |
//! | [`ClassRegistry`] | the `findClass`/`newInstance` half of `ResourceLoader` |
//! | [`ClassLoader`] / [`ClassLoaderUtils`] | `ClassLoaderUtils` |
//!
//! # Divergences from Lucene 10.5.0
//!
//! Java's `ResourceLoader` does two things: it opens a named byte stream, and
//! it looks a class up by name and instantiates it. The first has an exact Rust
//! meaning; the second does not, because Rust has no runtime class loading.
//!
//! * The stream half is [`ResourceLoader::open_resource`], returning a
//!   [`std::io::Read`] instead of an `InputStream`.
//! * The class half becomes [`ClassRegistry`], a name-keyed table of factories:
//!   `findClass` is [`ClassRegistry::find_class`] and `newInstance` is
//!   [`ClassRegistry::new_instance`], both reproducing Lucene's error messages.
//!   It is not a method of the trait because a generic method would make
//!   `dyn ResourceLoader` — which [`ResourceLoaderAware`] needs — impossible.
//! * `ClasspathResourceLoader` reads from the JVM classpath. The closest Rust
//!   construct is a set of resources embedded in the binary, so this port takes
//!   a name-to-bytes table (what `include_bytes!` produces).
//! * `ModuleResourceLoader` reads from a JVM module, with paths absolute to the
//!   module root. The closest Rust construct is a directory root, so this port
//!   reads from the filesystem below one.
//! * `ClassLoaderUtils.isParentClassLoader` walks a class loader's parent
//!   chain. There is no such chain in Rust, so [`ClassLoader`] models it
//!   explicitly and [`ClassLoaderUtils::is_parent_class_loader`] walks it with
//!   Lucene's algorithm, comparing loaders by identity as Java's `==` does.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{LuceneError, Result};

// ---------------------------------------------------------------------------
// ResourceLoader
// ---------------------------------------------------------------------------

/// Abstraction for loading resources.
///
/// Port of `org.apache.lucene.util.ResourceLoader`.
pub trait ResourceLoader: Send + Sync {
    /// Opens a named resource.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] when the resource cannot be found or read.
    fn open_resource(&self, resource: &str) -> Result<Box<dyn Read + Send>>;
}

/// A component that must be initialised by a [`ResourceLoader`].
///
/// Port of `org.apache.lucene.util.ResourceLoaderAware`.
pub trait ResourceLoaderAware {
    /// Initialises this component with `loader`, used to load files and
    /// classes.
    ///
    /// # Errors
    ///
    /// Propagates whatever the component's initialisation raised.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()>;
}

// ---------------------------------------------------------------------------
// ClassRegistry
// ---------------------------------------------------------------------------

/// A factory registered under a class name.
pub type ClassFactory<T> = Arc<dyn Fn() -> T + Send + Sync>;

/// A name-keyed table of factories, standing in for `Class.forName` plus
/// `Class.getConstructor().newInstance()`.
///
/// See the module documentation for why the class half of Lucene's
/// `ResourceLoader` takes this shape.
pub struct ClassRegistry<T> {
    factories: HashMap<String, ClassFactory<T>>,
    expected_type: &'static str,
}

impl<T> std::fmt::Debug for ClassRegistry<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassRegistry")
            .field("expected_type", &self.expected_type)
            .field("names", &self.available_classes())
            .finish()
    }
}

impl<T> ClassRegistry<T> {
    /// Creates an empty registry describing itself as producing
    /// `expected_type`, the name Lucene reports in its error messages.
    pub fn new(expected_type: &'static str) -> Self {
        Self {
            factories: HashMap::new(),
            expected_type,
        }
    }

    /// Registers `factory` under `cname`, replacing any previous entry.
    pub fn register(&mut self, cname: impl Into<String>, factory: ClassFactory<T>) {
        self.factories.insert(cname.into(), factory);
    }

    /// Returns the names registered, sorted.
    pub fn available_classes(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.factories.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Finds the factory registered under `cname`.
    ///
    /// Equivalent to `ResourceLoader.findClass(String, Class)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when nothing is registered
    /// under that name, which is the `RuntimeException("Cannot load class: ")`
    /// Java's implementations raise.
    pub fn find_class(&self, cname: &str) -> Result<&ClassFactory<T>> {
        self.factories.get(cname).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "Cannot load class: {cname} (expected type {})",
                self.expected_type
            ))
        })
    }

    /// Creates an instance of the class registered under `cname`.
    ///
    /// Equivalent to `ResourceLoader.newInstance(String, Class)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when nothing is registered
    /// under that name, which is Java's
    /// `RuntimeException("Cannot create instance: ")`.
    pub fn new_instance(&self, cname: &str) -> Result<T> {
        match self.factories.get(cname) {
            Some(factory) => Ok(factory()),
            None => Err(LuceneError::IllegalArgument(format!(
                "Cannot create instance: {cname} (expected type {})",
                self.expected_type
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ClasspathResourceLoader
// ---------------------------------------------------------------------------

/// A [`ResourceLoader`] over resources embedded in the binary.
///
/// Port of `org.apache.lucene.util.ClasspathResourceLoader`; see the module
/// documentation for why the classpath becomes an embedded table.
#[derive(Debug, Default, Clone)]
pub struct ClasspathResourceLoader {
    /// Optional prefix playing the role of Java's `Class` argument, which makes
    /// resource lookups relative to that class's package.
    prefix: Option<String>,
    resources: HashMap<String, Arc<Vec<u8>>>,
}

impl ClasspathResourceLoader {
    /// Creates a loader over `resources`, keyed by absolute resource name.
    ///
    /// Equivalent to `new ClasspathResourceLoader(ClassLoader)`.
    pub fn new(resources: HashMap<String, Arc<Vec<u8>>>) -> Self {
        Self {
            prefix: None,
            resources,
        }
    }

    /// Creates a loader that resolves relative names below `prefix`.
    ///
    /// Equivalent to `new ClasspathResourceLoader(Class)`, which resolves
    /// resources relative to that class's package.
    pub fn with_prefix(
        prefix: impl Into<String>,
        resources: HashMap<String, Arc<Vec<u8>>>,
    ) -> Self {
        Self {
            prefix: Some(prefix.into()),
            resources,
        }
    }

    /// Adds one resource.
    pub fn add_resource(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        self.resources.insert(name.into(), Arc::new(bytes));
    }
}

impl ResourceLoader for ClasspathResourceLoader {
    fn open_resource(&self, resource: &str) -> Result<Box<dyn Read + Send>> {
        let key = match (&self.prefix, resource.starts_with('/')) {
            (Some(prefix), false) => format!("{prefix}/{resource}"),
            _ => resource.to_string(),
        };
        match self.resources.get(&key) {
            Some(bytes) => Ok(Box::new(Cursor::new(bytes.as_ref().clone()))),
            None => Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Resource not found: {resource}"),
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ModuleResourceLoader
// ---------------------------------------------------------------------------

/// A [`ResourceLoader`] reading below a directory root.
///
/// Port of `org.apache.lucene.util.ModuleResourceLoader`; see the module
/// documentation for why a JVM module becomes a directory. As in Java,
/// resource paths are absolute to the root.
#[derive(Debug, Clone)]
pub struct ModuleResourceLoader {
    root: PathBuf,
}

impl ModuleResourceLoader {
    /// Creates a loader rooted at `root`.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Returns the root this loader reads below.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl ResourceLoader for ModuleResourceLoader {
    fn open_resource(&self, resource: &str) -> Result<Box<dyn Read + Send>> {
        let relative = resource.trim_start_matches('/');
        let path = self.root.join(relative);
        // Reject any attempt to escape the root, which a JVM module lookup
        // cannot do either.
        if relative.split('/').any(|c| c == "..") {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Resource path escapes the module root: {resource}"),
            )));
        }
        match std::fs::File::open(&path) {
            Ok(file) => Ok(Box::new(file)),
            Err(e) => Err(LuceneError::Io(std::io::Error::new(
                e.kind(),
                format!("Resource not found: {resource}"),
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ClassLoaderUtils
// ---------------------------------------------------------------------------

/// A node in a class-loader parent chain.
///
/// Rust has no class loaders; this models the only structure
/// [`ClassLoaderUtils`] inspects — the chain of parents — so that Lucene's
/// algorithm can be reproduced. Loaders are compared by identity, matching
/// Java's `==`.
#[derive(Debug, Clone)]
pub struct ClassLoader {
    name: String,
    parent: Option<Arc<ClassLoader>>,
}

impl ClassLoader {
    /// Creates a root loader.
    pub fn new(name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            parent: None,
        })
    }

    /// Creates a loader whose parent is `parent`.
    pub fn with_parent(name: impl Into<String>, parent: Arc<ClassLoader>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            parent: Some(parent),
        })
    }

    /// Returns this loader's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns this loader's parent, if any.
    pub fn get_parent(&self) -> Option<&Arc<ClassLoader>> {
        self.parent.as_ref()
    }
}

/// Helpers for investigating parent/child relationships between class loaders.
///
/// Port of `org.apache.lucene.util.ClassLoaderUtils`, which Lucene 10.5.0
/// declares as an interface holding a single static method.
pub struct ClassLoaderUtils;

impl ClassLoaderUtils {
    /// Returns whether `parent` is `child` or one of its (grand-)parents,
    /// meaning `child` can load everything `parent` can.
    ///
    /// Equivalent to `ClassLoaderUtils.isParentClassLoader`. Java also returns
    /// `false` when a `SecurityException` prevents the walk; Rust has no
    /// security manager, so that branch has nothing to reproduce.
    pub fn is_parent_class_loader(parent: &Arc<ClassLoader>, child: &Arc<ClassLoader>) -> bool {
        let mut cl = Some(child.clone());
        while let Some(current) = cl {
            if Arc::ptr_eq(&current, parent) {
                return true;
            }
            cl = current.parent.clone();
        }
        false
    }
}
