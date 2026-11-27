// SAX DTD handler.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: DTDHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `DTDHandler.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::JResult;

/// Receive notification of basic DTD-related events.
///
/// Equivalent to `org.xml.sax.DTDHandler`.
pub trait DTDHandler {
    /// Equivalent to `notationDecl(String, String, String)`.
    fn notation_decl(
        &self,
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
    ) -> JResult<()>;
    /// Equivalent to `unparsedEntityDecl(String, String, String, String)`.
    fn unparsed_entity_decl(
        &self,
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
        notation_name: &str,
    ) -> JResult<()>;
}
