// LexicalHandler.java - optional handler for lexical parse events.
// http://www.saxproject.org
// Public Domain: no warranty.
// $Id: LexicalHandler.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `LexicalHandler.java`, of the Java package `org.xml.sax.ext` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::JResult;

/// SAX2 extension handler for lexical events.
///
/// Equivalent to `org.xml.sax.ext.LexicalHandler`.
pub trait LexicalHandler {
    /// Equivalent to `startDTD(String, String, String)`.
    fn start_dtd(
        &self,
        name: &str,
        public_id: Option<&str>,
        system_id: Option<&str>,
    ) -> JResult<()>;
    /// Equivalent to `endDTD()`.
    fn end_dtd(&self) -> JResult<()>;
    /// Equivalent to `startEntity(String)`.
    fn start_entity(&self, name: &str) -> JResult<()>;
    /// Equivalent to `endEntity(String)`.
    fn end_entity(&self, name: &str) -> JResult<()>;
    /// Equivalent to `startCDATA()`.
    fn start_cdata(&self) -> JResult<()>;
    /// Equivalent to `endCDATA()`.
    fn end_cdata(&self) -> JResult<()>;
    /// Equivalent to `comment(char[], int, int)`.
    fn comment(&self, ch: &[u16], start: i32, length: i32) -> JResult<()>;
}
