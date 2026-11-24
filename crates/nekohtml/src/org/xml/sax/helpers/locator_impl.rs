// SAX default implementation for Locator.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: LocatorImpl.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `LocatorImpl.java`, of the Java package `org.xml.sax.helpers` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::locator::Locator;

/// Provide an optional convenience implementation of `Locator`.
///
/// Equivalent to `org.xml.sax.helpers.LocatorImpl`.
#[derive(Debug, Clone, Default)]
pub struct LocatorImpl {
    public_id: Option<String>,
    system_id: Option<String>,
    line_number: i32,
    column_number: i32,
}

impl LocatorImpl {
    /// Equivalent to `new LocatorImpl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new LocatorImpl(Locator)`.
    pub fn from_locator(locator: &dyn Locator) -> Self {
        let mut s = Self::new();
        s.set_public_id(locator.get_public_id().as_deref());
        s.set_system_id(locator.get_system_id().as_deref());
        s.set_line_number(locator.get_line_number());
        s.set_column_number(locator.get_column_number());
        s
    }

    /// Equivalent to `setPublicId(String)`.
    pub fn set_public_id(&mut self, public_id: Option<&str>) {
        self.public_id = public_id.map(str::to_string);
    }

    /// Equivalent to `setSystemId(String)`.
    pub fn set_system_id(&mut self, system_id: Option<&str>) {
        self.system_id = system_id.map(str::to_string);
    }

    /// Equivalent to `setLineNumber(int)`.
    pub fn set_line_number(&mut self, line_number: i32) {
        self.line_number = line_number;
    }

    /// Equivalent to `setColumnNumber(int)`.
    pub fn set_column_number(&mut self, column_number: i32) {
        self.column_number = column_number;
    }
}

impl Locator for LocatorImpl {
    fn get_public_id(&self) -> Option<String> {
        self.public_id.clone()
    }

    fn get_system_id(&self) -> Option<String> {
        self.system_id.clone()
    }

    fn get_line_number(&self) -> i32 {
        self.line_number
    }

    fn get_column_number(&self) -> i32 {
        self.column_number
    }
}
