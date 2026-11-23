// Locator2.java - extended Locator
// http://www.saxproject.org
// Public Domain: no warranty.
// $Id: Locator2.java 226236 2005-06-17 03:10:33Z mrglavas $

//! Port of `Locator2.java`, of the Java package `org.xml.sax.ext` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::locator::Locator;

/// SAX2 extension to augment the entity information provided through a
/// `Locator`.
///
/// Equivalent to `org.xml.sax.ext.Locator2`; an implementation answers
/// `Some(self)` from [`Locator::as_locator2`].
pub trait Locator2: Locator {
    /// Equivalent to `getXMLVersion()`.
    fn get_xml_version(&self) -> Option<String>;
    /// Equivalent to `getEncoding()`.
    fn get_encoding(&self) -> Option<String>;
}
