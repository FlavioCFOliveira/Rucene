// Locator2Impl.java - extended LocatorImpl
// http://www.saxproject.org
// Public Domain: no warranty.
// $Id: Locator2Impl.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `Locator2Impl.java`, of the Java package `org.xml.sax.ext` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::ext::locator2::Locator2;
use crate::org::xml::sax::helpers::locator_impl::LocatorImpl;
use crate::org::xml::sax::locator::Locator;

/// SAX2 extension helper for holding additional Entity information,
/// implementing the `Locator2` interface.
///
/// Equivalent to `org.xml.sax.ext.Locator2Impl` (a `LocatorImpl`, which it
/// holds as `base`).
#[derive(Debug, Clone, Default)]
pub struct Locator2Impl {
    /// The `LocatorImpl` this class extends.
    pub base: LocatorImpl,
    encoding: Option<String>,
    version: Option<String>,
}

impl Locator2Impl {
    /// Equivalent to `new Locator2Impl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new Locator2Impl(Locator)`.
    pub fn from_locator(locator: &dyn Locator) -> Self {
        let mut s = Self {
            base: LocatorImpl::from_locator(locator),
            encoding: None,
            version: None,
        };
        if let Some(l2) = locator.as_locator2() {
            s.version = l2.get_xml_version();
            s.encoding = l2.get_encoding();
        }
        s
    }

    /// Equivalent to `setXMLVersion(String)`.
    pub fn set_xml_version(&mut self, version: Option<&str>) {
        self.version = version.map(str::to_string);
    }

    /// Equivalent to `setEncoding(String)`.
    pub fn set_encoding(&mut self, encoding: Option<&str>) {
        self.encoding = encoding.map(str::to_string);
    }
}

impl Locator for Locator2Impl {
    fn get_public_id(&self) -> Option<String> {
        self.base.get_public_id()
    }

    fn get_system_id(&self) -> Option<String> {
        self.base.get_system_id()
    }

    fn get_line_number(&self) -> i32 {
        self.base.get_line_number()
    }

    fn get_column_number(&self) -> i32 {
        self.base.get_column_number()
    }

    fn as_locator2(&self) -> Option<&dyn Locator2> {
        Some(self)
    }
}

impl Locator2 for Locator2Impl {
    fn get_xml_version(&self) -> Option<String> {
        self.version.clone()
    }

    fn get_encoding(&self) -> Option<String> {
        self.encoding.clone()
    }
}
