// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ClassLoaderUtil`.

use crate::jdk::{ClassLoader, JavaClass};

/// Utilities for finding a non-null class loader.
///
/// Equivalent to `com.ibm.icu.impl.ClassLoaderUtil`. The JVM's loaders are
/// the [`ClassLoader`] values of this crate: every ICU class, the thread
/// context loader and the system loader are the application loader, so the
/// fallbacks of Java's methods (to the system loader, then to a loader whose
/// parent is the bootstrap loader) are kept but never taken.
pub struct ClassLoaderUtil;

impl ClassLoaderUtil {
    /// `getBootstrapClassLoader()` (private): the lazily created
    /// `BootstrapClassLoader`, a loader whose parent is the bootstrap loader.
    fn get_bootstrap_class_loader() -> ClassLoader {
        ClassLoader::Bootstrap
    }

    /// `getClassLoader(Class<?> cls)`: the class's loader, or
    /// [`get_class_loader`](Self::get_class_loader) when it has none.
    pub fn get_class_loader_of(cls: &JavaClass) -> ClassLoader {
        match cls.get_class_loader() {
            Some(cl) => cl,
            None => Self::get_class_loader(),
        }
    }

    /// `getClassLoader()`: the thread context loader, else the system
    /// loader, else the bootstrap-parented loader.
    pub fn get_class_loader() -> ClassLoader {
        // Thread.currentThread().getContextClassLoader() is the system loader.
        let context: Option<ClassLoader> = Some(ClassLoader::system());
        match context {
            Some(cl) => cl,
            None => {
                let system: Option<ClassLoader> = Some(ClassLoader::system());
                match system {
                    Some(cl) => cl,
                    None => Self::get_bootstrap_class_loader(),
                }
            }
        }
    }
}
