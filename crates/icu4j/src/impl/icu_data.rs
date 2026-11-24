// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUData`.

use rucene::error::Result;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::jdk::{ClassLoader, JavaClass, ResourceInputStream};
use crate::util::version_info::VersionInfo;

/// Provides access to ICU data files as streams.
///
/// Equivalent to `com.ibm.icu.impl.ICUData`. The resources are the ones the
/// crate embeds (see [`ClassLoader`]); the security-manager branches of
/// Java (`AccessController.doPrivileged`) have no counterpart, a Rust
/// process having no security manager, so the direct lookups run.
pub struct ICUData;

impl ICUData {
    /// `ICU_DATA_PATH` (package-private).
    pub const ICU_DATA_PATH: &'static str = "com/ibm/icu/impl/";

    /// The class `ICUData` itself, `ICUData.class`.
    pub const CLASS: JavaClass = JavaClass::new("com.ibm.icu.impl.ICUData");

    /// `ICU_BUNDLE`.
    pub const ICU_BUNDLE: &'static str = "data/icudata";

    /// `ICU_BASE_NAME`.
    pub const ICU_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata";

    /// `ICU_COLLATION_BASE_NAME`.
    pub const ICU_COLLATION_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/coll";

    /// `ICU_BRKITR_NAME`.
    pub const ICU_BRKITR_NAME: &'static str = "brkitr";

    /// `ICU_BRKITR_BASE_NAME`.
    pub const ICU_BRKITR_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/brkitr";

    /// `ICU_RBNF_BASE_NAME`.
    pub const ICU_RBNF_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/rbnf";

    /// `ICU_TRANSLIT_BASE_NAME`.
    pub const ICU_TRANSLIT_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/translit";

    /// `ICU_LANG_BASE_NAME`.
    pub const ICU_LANG_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/lang";

    /// `ICU_CURR_BASE_NAME`.
    pub const ICU_CURR_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/curr";

    /// `ICU_REGION_BASE_NAME`.
    pub const ICU_REGION_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/region";

    /// `ICU_ZONE_BASE_NAME`.
    pub const ICU_ZONE_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/zone";

    /// `ICU_UNIT_BASE_NAME`.
    pub const ICU_UNIT_BASE_NAME: &'static str = "com/ibm/icu/impl/data/icudata/unit";

    /// `logBinaryDataFromInputStream` (private, `false`): the logging of
    /// [`check_stream_for_binary_data`](Self::check_stream_for_binary_data)
    /// is compiled out, as in Java.
    const LOG_BINARY_DATA_FROM_INPUT_STREAM: bool = false;

    /// `PACKAGE_NAME` (package-private): `"icudt" + ICU_DATA_VERSION_PATH`.
    pub(crate) fn package_name() -> String {
        format!("icudt{}", VersionInfo::ICU_DATA_VERSION_PATH)
    }

    /// `exists(String resourceName)`: whether `ICUData.class.getResource`
    /// finds the resource.
    pub fn exists(resource_name: &str) -> bool {
        Self::CLASS.has_resource(resource_name)
    }

    /// `getStream(Class<?> root, String resourceName, boolean required)`
    /// (private).
    fn get_stream_class(
        root: &JavaClass,
        resource_name: &str,
        required: bool,
    ) -> Result<Option<ResourceInputStream>> {
        let i = root.get_resource_as_stream(resource_name);
        if i.is_none() && required {
            return Err(MissingResourceException::new(
                format!("could not locate data {resource_name}"),
                root.get_package_name(),
                resource_name,
            )
            .error());
        }
        Self::check_stream_for_binary_data(i.as_ref(), resource_name);
        Ok(i)
    }

    /// `getStream(ClassLoader loader, String resourceName, boolean required)`
    /// (package-private).
    pub(crate) fn get_stream_loader_required(
        loader: &ClassLoader,
        resource_name: &str,
        required: bool,
    ) -> Result<Option<ResourceInputStream>> {
        let i = loader.get_resource_as_stream(resource_name);
        if i.is_none() && required {
            return Err(MissingResourceException::new(
                "could not locate data",
                loader.to_java_string(),
                resource_name,
            )
            .error());
        }
        Self::check_stream_for_binary_data(i.as_ref(), resource_name);
        Ok(i)
    }

    /// `checkStreamForBinaryData(InputStream, String)` (private): logs the
    /// header of an ICU binary data file read as a stream when
    /// `logBinaryDataFromInputStream` is on; it is off, so this does
    /// nothing, as in Java.
    fn check_stream_for_binary_data(is: Option<&ResourceInputStream>, resource_name: &str) {
        if Self::LOG_BINARY_DATA_FROM_INPUT_STREAM
            && is.is_some()
            && resource_name.contains(&Self::package_name())
        {
            unreachable!("logBinaryDataFromInputStream is false");
        }
    }

    /// `getStream(ClassLoader loader, String resourceName)`: the stream,
    /// `None` when the resource is missing.
    pub fn get_stream_loader(
        loader: &ClassLoader,
        resource_name: &str,
    ) -> Option<ResourceInputStream> {
        Self::get_stream_loader_required(loader, resource_name, false)
            .expect("a lookup that is not required never throws")
    }

    /// `getRequiredStream(ClassLoader loader, String resourceName)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException("could not locate data", loader, name)`
    /// when the resource is missing.
    pub fn get_required_stream_loader(
        loader: &ClassLoader,
        resource_name: &str,
    ) -> Result<ResourceInputStream> {
        Self::get_stream_loader_required(loader, resource_name, true)
            .map(|s| s.expect("a required lookup returns a stream or throws"))
    }

    /// `getStream(String resourceName)`: relative to `ICUData.class`.
    pub fn get_stream(resource_name: &str) -> Option<ResourceInputStream> {
        Self::get_stream_class(&Self::CLASS, resource_name, false)
            .expect("a lookup that is not required never throws")
    }

    /// `getRequiredStream(String resourceName)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException("could not locate data <name>", package,
    /// name)` when the resource is missing.
    pub fn get_required_stream(resource_name: &str) -> Result<ResourceInputStream> {
        Self::get_stream_class(&Self::CLASS, resource_name, true)
            .map(|s| s.expect("a required lookup returns a stream or throws"))
    }

    /// `getStream(Class<?> root, String resourceName)`.
    pub fn get_stream_from(root: &JavaClass, resource_name: &str) -> Option<ResourceInputStream> {
        Self::get_stream_class(root, resource_name, false)
            .expect("a lookup that is not required never throws")
    }

    /// `getRequiredStream(Class<?> root, String resourceName)`.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the resource is missing.
    pub fn get_required_stream_from(
        root: &JavaClass,
        resource_name: &str,
    ) -> Result<ResourceInputStream> {
        Self::get_stream_class(root, resource_name, true)
            .map(|s| s.expect("a required lookup returns a stream or throws"))
    }
}
