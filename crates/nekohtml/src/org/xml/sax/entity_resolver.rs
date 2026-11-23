// SAX entity resolver.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: EntityResolver.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `EntityResolver.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use std::any::Any;

use crate::java::JResult;
use crate::org::xml::sax::ext::entity_resolver2::EntityResolver2;
use crate::org::xml::sax::input_source::InputSource;

/// Basic interface for resolving entities.
///
/// Equivalent to `org.xml.sax.EntityResolver`.
pub trait EntityResolver: Any {
    /// Equivalent to `resolveEntity(String, String)`; `Ok(None)` is Java's
    /// `null` (use the system identifier).
    fn resolve_entity(&self, public_id: Option<&str>, system_id: Option<&str>) -> JResult<Option<InputSource>>;

    /// `this instanceof EntityResolver2 ? (EntityResolver2) this : null`.
    fn as_entity_resolver2(&self) -> Option<&dyn EntityResolver2> {
        None
    }
}
