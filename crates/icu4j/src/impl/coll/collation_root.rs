// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationRoot`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_data_reader::CollationDataReader;
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::r#impl::coll::collation_tailoring::CollationTailoring;
use crate::r#impl::icu_binary::ICUBinary;
use crate::r#impl::icu_data::ICUData;

/// Collation root provider.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationRoot` (purely static).
/// The static initializer's outcome, the root tailoring or the
/// `RuntimeException` of loading it, is kept and returned on every call,
/// as Java's `rootSingleton`/`exception` pair.
#[derive(Debug)]
pub struct CollationRoot;

static ROOT: LazyLock<std::result::Result<Arc<CollationTailoring>, Arc<LuceneError>>> = LazyLock::new(load);

/// Corresponds to C++ load() function.
fn load() -> std::result::Result<Arc<CollationTailoring>, Arc<LuceneError>> {
    let read = || -> Result<CollationTailoring> {
        let mut bytes = ICUBinary::get_required_data("coll/ucadata.icu")?;
        let mut t2 = CollationTailoring::new(None);
        CollationDataReader::read(None, &mut bytes, &mut t2)?;
        Ok(t2)
    };
    // Keep t=null until after the root data has been read completely.
    // Otherwise we would set a non-null root object if the data reader throws an exception.
    match read() {
        Ok(t) => Ok(Arc::new(t)),
        Err(e) if e.is_io_exception() => Err(Arc::new(
            MissingResourceException::new(
                "IOException while reading CLDR root data",
                "CollationRoot",
                format!("{}/coll/ucadata.icu", ICUData::ICU_BUNDLE),
            )
            .error(),
        )),
        Err(e) => Err(Arc::new(e)),
    }
}

impl CollationRoot {
    /// `getRoot()`.
    ///
    /// # Errors
    ///
    /// The exception of loading the root data.
    pub fn get_root() -> Result<Arc<CollationTailoring>> {
        match &*ROOT {
            Ok(t) => Ok(t.clone()),
            Err(e) => Err(e.duplicate()),
        }
    }

    /// `getData()`.
    ///
    /// # Errors
    ///
    /// The exception of loading the root data.
    pub fn get_data() -> Result<Arc<CollationData>> {
        let root = Self::get_root()?;
        Ok(root.data().clone())
    }

    /// `getSettings()`.
    ///
    /// # Errors
    ///
    /// The exception of loading the root data.
    pub fn get_settings() -> Result<Arc<CollationSettings>> {
        let root = Self::get_root()?;
        Ok(root.settings.read_only_arc().unwrap_or_else(|| panic!("java.lang.NullPointerException")).clone())
    }
}
