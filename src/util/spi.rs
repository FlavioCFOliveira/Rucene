//! Named service-provider loading ported from
//! `org.apache.lucene.util.NamedSPILoader`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`NamedSPI`] | `NamedSPILoader.NamedSPI` |
//! | [`NamedSPILoader`] | `NamedSPILoader<S>` |
//!
//! **Divergence from Lucene 10.5.0.** Java discovers implementations with
//! `java.util.ServiceLoader`, which scans `META-INF/services` entries on a
//! class loader. Rust has no runtime service discovery: a crate knows its
//! providers at compile time. [`NamedSPILoader::reload`] therefore takes the
//! providers explicitly instead of a `ClassLoader`, and the constructor takes
//! the initial set. Everything the loader then does — first registration for a
//! name wins, the service-name validation, the lookup error listing the
//! available names, and the insertion-ordered iteration — is reproduced
//! exactly.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{LuceneError, Result};

/// A service provider that carries a name.
///
/// Port of the nested interface `NamedSPILoader.NamedSPI`.
pub trait NamedSPI {
    /// Returns the name this provider is registered under.
    fn get_name(&self) -> &str;
}

/// Holds the providers of one SPI, keyed by name and kept in insertion order.
///
/// Port of `org.apache.lucene.util.NamedSPILoader`.
pub struct NamedSPILoader<S: NamedSPI + ?Sized> {
    /// Insertion-ordered providers, matching Java's `LinkedHashMap`.
    services: Vec<Arc<S>>,
    index: HashMap<String, usize>,
    /// The SPI name reported in the lookup error, standing in for
    /// `clazz.getName()`.
    clazz_name: &'static str,
}

impl<S: NamedSPI + ?Sized> std::fmt::Debug for NamedSPILoader<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedSPILoader")
            .field("clazz_name", &self.clazz_name)
            .field("available_services", &self.available_services())
            .finish()
    }
}

impl<S: NamedSPI + ?Sized> NamedSPILoader<S> {
    /// Creates an empty loader for the SPI named `clazz_name`.
    pub fn new(clazz_name: &'static str) -> Self {
        Self {
            services: Vec::new(),
            index: HashMap::new(),
            clazz_name,
        }
    }

    /// Creates a loader for the SPI named `clazz_name`, populated with
    /// `providers`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a provider's name is not a
    /// legal service name.
    pub fn with_providers<I>(clazz_name: &'static str, providers: I) -> Result<Self>
    where
        I: IntoIterator<Item = Arc<S>>,
    {
        let mut loader = Self::new(clazz_name);
        loader.reload(providers)?;
        Ok(loader)
    }

    /// Adds `providers` to this loader.
    ///
    /// Only the first provider registered for a name is kept, so a provider
    /// placed earlier wins over a later one — the behaviour Java gets from
    /// classpath ordering.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a provider's name is not a
    /// legal service name.
    pub fn reload<I>(&mut self, providers: I) -> Result<()>
    where
        I: IntoIterator<Item = Arc<S>>,
    {
        for service in providers {
            let name = service.get_name().to_string();
            if self.index.contains_key(&name) {
                continue;
            }
            Self::check_service_name(&name)?;
            self.index.insert(name, self.services.len());
            self.services.push(service);
        }
        Ok(())
    }

    /// Validates a service name.
    ///
    /// Equivalent to `NamedSPILoader.checkServiceName`: names must be shorter
    /// than 128 characters and purely ASCII alphanumeric.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] with Lucene's message.
    pub fn check_service_name(name: &str) -> Result<()> {
        // Based on harmony charset.java.
        if name.chars().count() >= 128 {
            return Err(LuceneError::IllegalArgument(format!(
                "Illegal service name: '{name}' is too long (must be < 128 chars)."
            )));
        }
        for c in name.chars() {
            if !Self::is_letter_or_digit(c) {
                return Err(LuceneError::IllegalArgument(format!(
                    "Illegal service name: '{name}' must be simple ascii alphanumeric."
                )));
            }
        }
        Ok(())
    }

    fn is_letter_or_digit(c: char) -> bool {
        c.is_ascii_lowercase() || c.is_ascii_uppercase() || c.is_ascii_digit()
    }

    /// Returns the provider registered under `name`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] listing the available names,
    /// reproducing Lucene's message.
    pub fn lookup(&self, name: &str) -> Result<Arc<S>> {
        match self.index.get(name) {
            Some(&i) => Ok(Arc::clone(&self.services[i])),
            None => Err(LuceneError::IllegalArgument(format!(
                "An SPI class of type {} with name '{name}' does not exist. \
                 You need to add the corresponding JAR file supporting this SPI to your classpath. \
                 The current classpath supports the following names: {:?}",
                self.clazz_name,
                self.available_services()
            ))),
        }
    }

    /// Returns the registered names, in insertion order.
    pub fn available_services(&self) -> Vec<&str> {
        self.services.iter().map(|s| s.get_name()).collect()
    }

    /// Iterates over the providers in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<S>> {
        self.services.iter()
    }
}

impl<'a, S: NamedSPI + ?Sized> IntoIterator for &'a NamedSPILoader<S> {
    type Item = &'a Arc<S>;
    type IntoIter = std::slice::Iter<'a, Arc<S>>;

    fn into_iter(self) -> Self::IntoIter {
        self.services.iter()
    }
}
