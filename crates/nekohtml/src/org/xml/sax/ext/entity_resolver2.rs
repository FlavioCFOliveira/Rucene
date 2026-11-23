// EntityResolver2.java - Extended SAX entity resolver.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: EntityResolver2.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `EntityResolver2.java`, of the Java package `org.xml.sax.ext` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::JResult;
use crate::org::xml::sax::entity_resolver::EntityResolver;
use crate::org::xml::sax::input_source::InputSource;

/// Extended interface for mapping external entity references to input
/// sources, or providing a missing external subset.
///
/// Equivalent to `org.xml.sax.ext.EntityResolver2`; an implementation
/// answers `Some(self)` from
/// [`EntityResolver::as_entity_resolver2`].
pub trait EntityResolver2: EntityResolver {
    /// Equivalent to `getExternalSubset(String, String)`.
    fn get_external_subset(&self, name: &str, base_uri: Option<&str>) -> JResult<Option<InputSource>>;
    /// Equivalent to `resolveEntity(String, String, String, String)`.
    fn resolve_entity2(
        &self,
        name: Option<&str>,
        public_id: Option<&str>,
        base_uri: Option<&str>,
        system_id: Option<&str>,
    ) -> JResult<Option<InputSource>>;
}
