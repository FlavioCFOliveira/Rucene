// SAX locator interface for document events.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: Locator.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `Locator.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::ext::locator2::Locator2;

/// Interface for associating a SAX event with a document location.
///
/// Equivalent to `org.xml.sax.Locator`.
pub trait Locator {
    /// Equivalent to `getPublicId()`.
    fn get_public_id(&self) -> Option<String>;
    /// Equivalent to `getSystemId()`.
    fn get_system_id(&self) -> Option<String>;
    /// Equivalent to `getLineNumber()`.
    fn get_line_number(&self) -> i32;
    /// Equivalent to `getColumnNumber()`.
    fn get_column_number(&self) -> i32;

    /// `this instanceof Locator2 ? (Locator2) this : null`.
    fn as_locator2(&self) -> Option<&dyn Locator2> {
        None
    }
}
