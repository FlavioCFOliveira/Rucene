// SAX Attribute List Interface.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: AttributeList.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `AttributeList.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

/// Interface for an element's attribute specifications (SAX 1).
///
/// Equivalent to `org.xml.sax.AttributeList`.
pub trait AttributeList {
    /// Equivalent to `getLength()`.
    fn get_length(&self) -> i32;
    /// Equivalent to `getName(int)`.
    fn get_name(&self, i: i32) -> Option<String>;
    /// Equivalent to `getType(int)`.
    fn get_type(&self, i: i32) -> Option<String>;
    /// Equivalent to `getValue(int)`.
    fn get_value(&self, i: i32) -> Option<String>;
    /// Equivalent to `getType(String)`.
    fn get_type_by_name(&self, name: &str) -> Option<String>;
    /// Equivalent to `getValue(String)`.
    fn get_value_by_name(&self, name: &str) -> Option<String>;
}
