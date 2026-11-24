// Attributes.java - attribute list with Namespace support
// http://www.saxproject.org
// Written by David Megginson
// NO WARRANTY!  This class is in the public domain.
// $Id: Attributes.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `Attributes.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

/// Interface for a list of XML attributes.
///
/// Equivalent to `org.xml.sax.Attributes`. Java's `null` results are `None`.
pub trait Attributes {
    /// Equivalent to `getLength()`.
    fn get_length(&self) -> i32;
    /// Equivalent to `getURI(int)`.
    fn get_uri(&self, index: i32) -> Option<String>;
    /// Equivalent to `getLocalName(int)`.
    fn get_local_name(&self, index: i32) -> Option<String>;
    /// Equivalent to `getQName(int)`.
    fn get_qname(&self, index: i32) -> Option<String>;
    /// Equivalent to `getType(int)`.
    fn get_type(&self, index: i32) -> Option<String>;
    /// Equivalent to `getValue(int)`.
    fn get_value(&self, index: i32) -> Option<String>;
    /// Equivalent to `getIndex(String, String)`.
    fn get_index_ns(&self, uri: &str, local_name: &str) -> i32;
    /// Equivalent to `getIndex(String)`.
    fn get_index(&self, qname: &str) -> i32;
    /// Equivalent to `getType(String, String)`.
    fn get_type_ns(&self, uri: &str, local_name: &str) -> Option<String>;
    /// Equivalent to `getType(String)`.
    fn get_type_by_qname(&self, qname: &str) -> Option<String>;
    /// Equivalent to `getValue(String, String)`.
    fn get_value_ns(&self, uri: &str, local_name: &str) -> Option<String>;
    /// Equivalent to `getValue(String)`.
    fn get_value_by_qname(&self, qname: &str) -> Option<String>;
}
