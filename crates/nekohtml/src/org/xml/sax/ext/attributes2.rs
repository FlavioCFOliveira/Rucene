// Attributes2.java - extended Attributes
// http://www.saxproject.org
// Public Domain: no warranty.
// $Id: Attributes2.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `Attributes2.java`, of the Java package `org.xml.sax.ext` (SAX 2.0.2, xml-apis 1.4.01).

use crate::org::xml::sax::attributes::Attributes;

/// SAX2 extension to augment the per-attribute information provided through
/// `Attributes`.
///
/// Equivalent to `org.xml.sax.ext.Attributes2`.
pub trait Attributes2: Attributes {
    /// Equivalent to `isDeclared(int)`.
    fn is_declared(&self, index: i32) -> crate::java::JResult<bool>;
    /// Equivalent to `isDeclared(String)`.
    fn is_declared_by_qname(&self, qname: &str) -> crate::java::JResult<bool>;
    /// Equivalent to `isDeclared(String, String)`.
    fn is_declared_ns(&self, uri: &str, local_name: &str) -> crate::java::JResult<bool>;
    /// Equivalent to `isSpecified(int)`.
    fn is_specified(&self, index: i32) -> crate::java::JResult<bool>;
    /// Equivalent to `isSpecified(String, String)`.
    fn is_specified_ns(&self, uri: &str, local_name: &str) -> crate::java::JResult<bool>;
    /// Equivalent to `isSpecified(String)`.
    fn is_specified_by_qname(&self, qname: &str) -> crate::java::JResult<bool>;
}
