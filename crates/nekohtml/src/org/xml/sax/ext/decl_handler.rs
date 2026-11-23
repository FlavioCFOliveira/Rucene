// DeclHandler.java - Optional handler for DTD declaration events.
// http://www.saxproject.org
// Public Domain: no warranty.
// $Id: DeclHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `DeclHandler.java`, of the Java package `org.xml.sax.ext` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::JResult;

/// SAX2 extension handler for DTD declaration events.
///
/// Equivalent to `org.xml.sax.ext.DeclHandler`.
pub trait DeclHandler {
    /// Equivalent to `elementDecl(String, String)`.
    fn element_decl(&self, name: &str, model: &str) -> JResult<()>;
    /// Equivalent to `attributeDecl(String, String, String, String,
    /// String)`.
    fn attribute_decl(
        &self,
        e_name: &str,
        a_name: &str,
        type_: &str,
        mode: Option<&str>,
        value: Option<&str>,
    ) -> JResult<()>;
    /// Equivalent to `internalEntityDecl(String, String)`.
    fn internal_entity_decl(&self, name: &str, value: &str) -> JResult<()>;
    /// Equivalent to `externalEntityDecl(String, String, String)`.
    fn external_entity_decl(&self, name: &str, public_id: Option<&str>, system_id: Option<&str>) -> JResult<()>;
}
