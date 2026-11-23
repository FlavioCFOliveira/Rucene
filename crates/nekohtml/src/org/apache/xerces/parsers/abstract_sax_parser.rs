/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `AbstractSAXParser.java`, of the Java package `org.apache.xerces.parsers` (Xerces-J 2.12.2).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::object::JavaObject;

use crate::java::{
    interface_object, object_as_bool, object_as_interface, string_object, Exception, JResult,
    Object,
};
use crate::org::apache::xerces::parsers::abstract_xml_document_parser::AbstractXMLDocumentParser;
use crate::org::apache::xerces::parsers::xml_parser::{ENTITY_RESOLVER, ERROR_HANDLER};
use crate::org::apache::xerces::r#impl::constants;
use crate::org::apache::xerces::util::entity_resolver2_wrapper::EntityResolver2Wrapper;
use crate::org::apache::xerces::util::entity_resolver_wrapper::EntityResolverWrapper;
use crate::org::apache::xerces::util::error_handler_wrapper::ErrorHandlerWrapper;
use crate::org::apache::xerces::util::sax_message_formatter::SAXMessageFormatter;
use crate::org::apache::xerces::util::symbol_hash::SymbolHash;
use crate::org::apache::xerces::util::xml_symbols;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_configuration_exception::{
    XMLConfigurationException, NOT_RECOGNIZED,
};
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::parser::xml_dtd_content_model_source::XMLDTDContentModelSource;
use crate::org::apache::xerces::xni::parser::xml_dtd_source::XMLDTDSource;
use crate::org::apache::xerces::xni::parser::xml_entity_resolver::XMLEntityResolver;
use crate::org::apache::xerces::xni::parser::xml_error_handler::XMLErrorHandler;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::parser::xml_parse_exception::XMLParseException;
use crate::org::apache::xerces::xni::parser::xml_parser_configuration::XMLParserConfiguration;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_dtd_content_model_handler::XMLDTDContentModelHandler;
use crate::org::apache::xerces::xni::xml_dtd_handler::XMLDTDHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xml_string::XMLString;
use crate::org::apache::xerces::xni::xni_exception::XNIException;
use crate::org::apache::xerces::xs::attribute_psvi::AttributePSVI;
use crate::org::apache::xerces::xs::element_psvi::ElementPSVI;
use crate::org::apache::xerces::xs::psvi_provider::PSVIProvider;
use crate::org::xml::sax::attribute_list::AttributeList;
use crate::org::xml::sax::attributes::Attributes;
use crate::org::xml::sax::content_handler::ContentHandler;
use crate::org::xml::sax::document_handler::DocumentHandler;
use crate::org::xml::sax::dtd_handler::DTDHandler;
use crate::org::xml::sax::entity_resolver::EntityResolver;
use crate::org::xml::sax::error_handler::ErrorHandler;
use crate::org::xml::sax::ext::attributes2::Attributes2;
use crate::org::xml::sax::ext::decl_handler::DeclHandler;
use crate::org::xml::sax::ext::lexical_handler::LexicalHandler;
use crate::org::xml::sax::ext::locator2::Locator2;
use crate::org::xml::sax::ext::locator2_impl::Locator2Impl;
use crate::org::xml::sax::input_source::InputSource;
use crate::org::xml::sax::locator::Locator;
use crate::org::xml::sax::parser::Parser;
use crate::org::xml::sax::sax_exception::SAXException;
use crate::org::xml::sax::sax_not_recognized_exception::SAXNotRecognizedException;
use crate::org::xml::sax::sax_not_supported_exception::SAXNotSupportedException;
use crate::org::xml::sax::sax_parse_exception::SAXParseException;
use crate::org::xml::sax::xml_reader::XMLReader;

/// Feature identifier: namespaces.
pub const NAMESPACES: &str =
    concatcp!(constants::SAX_FEATURE_PREFIX, constants::NAMESPACES_FEATURE);

/// Feature id: string interning.
pub const STRING_INTERNING: &str = concatcp!(
    constants::SAX_FEATURE_PREFIX,
    constants::STRING_INTERNING_FEATURE
);

/// Feature identifier: allow notation and unparsed entity events to be
/// sent out of order.
pub const ALLOW_UE_AND_NOTATION_EVENTS: &str = concatcp!(
    constants::SAX_FEATURE_PREFIX,
    constants::ALLOW_DTD_EVENTS_AFTER_ENDDTD_FEATURE
);

/// Recognized features.
const RECOGNIZED_FEATURES: [&str; 2] = [NAMESPACES, STRING_INTERNING];

/// Property id: lexical handler.
pub const LEXICAL_HANDLER: &str = concatcp!(
    constants::SAX_PROPERTY_PREFIX,
    constants::LEXICAL_HANDLER_PROPERTY
);

/// Property id: declaration handler.
pub const DECLARATION_HANDLER: &str = concatcp!(
    constants::SAX_PROPERTY_PREFIX,
    constants::DECLARATION_HANDLER_PROPERTY
);

/// Property id: DOM node.
pub const DOM_NODE: &str = concatcp!(constants::SAX_PROPERTY_PREFIX, constants::DOM_NODE_PROPERTY);

/// Recognized properties.
const RECOGNIZED_PROPERTIES: [&str; 3] = [LEXICAL_HANDLER, DECLARATION_HANDLER, DOM_NODE];

/// What `fAugmentations` held for `getElementPSVI()`: `None` when it was
/// `null`, else the `ELEMENT_PSVI` item the augmentations had.
type AugmentationsSnapshot = Option<Option<Object>>;

/// What `fAttributesProxy.fAttributes` held for `getAttributePSVI`: each
/// attribute's namespace URI, local name and augmentations.
type AttributesSnapshot = Vec<(
    Option<String>,
    Option<String>,
    Option<Rc<dyn Augmentations>>,
)>;

/// This is the base class of all SAX parsers. It implements both the SAX1
/// and SAX2 parser functionality, while the actual pipeline is defined in
/// the parser configuration.
///
/// Equivalent to the abstract class
/// `org.apache.xerces.parsers.AbstractSAXParser` (a subclass is a type that
/// holds an `Rc` of it, as `SAXParser` does).
///
/// **Divergence.** Java's constructor registers `this` with the
/// configuration as its document, DTD and DTD content model handler, and
/// the configuration (and the pipeline's last stage) then holds the parser
/// while the parser holds the configuration. Reference counting does not
/// collect that cycle, so the configuration holds a handler that refers to
/// the parser weakly ([`AbstractSAXParserHandler`]) and forwards every call
/// to it; `getDocumentHandler()` on the configuration returns that handler
/// rather than the parser itself.
///
/// **Divergence.** Java keeps the last element's `Augmentations` and the
/// last element's `XMLAttributes` (`fAugmentations`,
/// `fAttributesProxy.fAttributes`) for the PSVI getters, which read them
/// later from within a handler callback. A Rust callback borrows them only
/// for its duration, so the parser keeps what the getters read: the
/// `ELEMENT_PSVI` item, and each attribute's name and augmentations (the
/// same shared object).
pub struct AbstractSAXParser {
    /// The `AbstractXMLDocumentParser` this class extends.
    pub base: AbstractXMLDocumentParser,
    /// Namespaces.
    namespaces: Cell<bool>,
    /// Namespace prefixes.
    namespace_prefixes: Cell<bool>,
    /// Lexical handler parameter entities.
    lexical_handler_parameter_entities: Cell<bool>,
    /// Standalone document declaration.
    standalone: Cell<bool>,
    /// Resolve DTD URIs.
    resolve_dtd_uris: Cell<bool>,
    /// Use EntityResolver2.
    use_entity_resolver2: Cell<bool>,
    /// XMLNS URIs: Namespace declarations in the
    /// http://www.w3.org/2000/xmlns/ namespace.
    xmlns_uris: Cell<bool>,
    /// Content handler.
    content_handler: RefCell<Option<Rc<dyn ContentHandler>>>,
    /// Document handler.
    document_handler: RefCell<Option<Rc<dyn DocumentHandler>>>,
    /// `fNamespaceContext`.
    namespace_context: RefCell<Option<Rc<dyn NamespaceContext>>>,
    /// DTD handler.
    dtd_handler: RefCell<Option<Rc<dyn DTDHandler>>>,
    /// Decl handler.
    decl_handler: RefCell<Option<Rc<dyn DeclHandler>>>,
    /// Lexical handler.
    lexical_handler: RefCell<Option<Rc<dyn LexicalHandler>>>,
    /// `fQName`.
    qname: RefCell<QName>,
    /// Parse in progress.
    parse_in_progress: Cell<bool>,
    /// The XML version of the document.
    version: RefCell<Option<String>>,
    /// See the type's divergence.
    augmentations: RefCell<AugmentationsSnapshot>,
    /// See the type's divergence.
    attributes: RefCell<AttributesSnapshot>,
    /// Symbol hash of declared attributes.
    declared_attrs: RefCell<Option<SymbolHash>>,
}

impl std::fmt::Debug for AbstractSAXParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbstractSAXParser")
            .field("namespaces", &self.namespaces.get())
            .field("version", &self.version.borrow())
            .finish_non_exhaustive()
    }
}

/// The handler the configuration holds for an [`AbstractSAXParser`]: it
/// refers to the parser weakly and forwards every call to it (see the
/// parser's divergence); a call after the parser is dropped does nothing.
pub struct AbstractSAXParserHandler(Weak<AbstractSAXParser>);

fn xni(e: Exception) -> Exception {
    match e {
        Exception::Sax(e) => XNIException::with_exception(Exception::Sax(e)).into(),
        other => other,
    }
}

fn is_true(augs: Option<&dyn Augmentations>, key: &str) -> bool {
    augs.and_then(|a| a.get_item(key))
        .and_then(|o| object_as_bool(&o))
        .unwrap_or(false)
}

impl AbstractSAXParser {
    /// Default constructor.
    ///
    /// Equivalent to `AbstractSAXParser(XMLParserConfiguration)`.
    pub fn new(config: Rc<dyn XMLParserConfiguration>) -> Rc<Self> {
        let parser = Rc::new(Self {
            base: AbstractXMLDocumentParser::new(Rc::clone(&config)),
            namespaces: Cell::new(false),
            namespace_prefixes: Cell::new(false),
            lexical_handler_parameter_entities: Cell::new(true),
            standalone: Cell::new(false),
            resolve_dtd_uris: Cell::new(true),
            use_entity_resolver2: Cell::new(true),
            xmlns_uris: Cell::new(false),
            content_handler: RefCell::new(None),
            document_handler: RefCell::new(None),
            namespace_context: RefCell::new(None),
            dtd_handler: RefCell::new(None),
            decl_handler: RefCell::new(None),
            lexical_handler: RefCell::new(None),
            qname: RefCell::new(QName::new()),
            parse_in_progress: Cell::new(false),
            version: RefCell::new(None),
            augmentations: RefCell::new(None),
            attributes: RefCell::new(Vec::new()),
            declared_attrs: RefCell::new(None),
        });
        // AbstractXMLDocumentParser's constructor
        let handler = Rc::new(AbstractSAXParserHandler(Rc::downgrade(&parser)));
        config.set_document_handler(Some(Rc::clone(&handler) as Rc<dyn XMLDocumentHandler>));
        config.set_dtd_handler(Some(Rc::clone(&handler) as Rc<dyn XMLDTDHandler>));
        config.set_dtd_content_model_handler(Some(handler as Rc<dyn XMLDTDContentModelHandler>));

        let features: Vec<String> = RECOGNIZED_FEATURES.iter().map(|s| s.to_string()).collect();
        config.add_recognized_features(Some(&features));
        let properties: Vec<String> = RECOGNIZED_PROPERTIES
            .iter()
            .map(|s| s.to_string())
            .collect();
        config.add_recognized_properties(Some(&properties));

        // XMLConfigurationException: it wasn't a recognized feature, so we
        // don't worry about it
        let _ = config.set_feature(ALLOW_UE_AND_NOTATION_EVENTS, false);
        parser
    }

    /// The parser configuration (`fConfiguration`).
    pub fn configuration(&self) -> &Rc<dyn XMLParserConfiguration> {
        &self.base.base.configuration
    }

    fn locale(&self) -> Locale {
        self.configuration().get_locale()
    }

    fn format(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<String> {
        SAXMessageFormatter::format_message(Some(&self.locale()), key, args)
    }

    fn not_supported(&self, key: &str, args: &[&str]) -> Exception {
        let args: Vec<JavaObject> = args
            .iter()
            .map(|a| JavaObject::String(a.to_string()))
            .collect();
        match self.format(key, Some(&args)) {
            Ok(m) => SAXNotSupportedException::with_message(Some(&m)).into(),
            Err(e) => e,
        }
    }

    fn not_recognized(&self, key: &str, args: &[&str]) -> Exception {
        let args: Vec<JavaObject> = args
            .iter()
            .map(|a| JavaObject::String(a.to_string()))
            .collect();
        match self.format(key, Some(&args)) {
            Ok(m) => SAXNotRecognizedException::with_message(Some(&m)).into(),
            Err(e) => e,
        }
    }

    /// `catch (XMLConfigurationException e)` of the feature and property
    /// accessors: a recognition failure becomes a
    /// `SAXNotRecognizedException`, a support failure a
    /// `SAXNotSupportedException`, with the message of `kind`.
    fn convert_configuration_exception(&self, e: Exception, kind: &str) -> Exception {
        match XMLConfigurationException::of(&e) {
            Some((type_, identifier)) => {
                let identifier = identifier.unwrap_or("null").to_string();
                if type_ == NOT_RECOGNIZED {
                    self.not_recognized(&format!("{kind}-not-recognized"), &[&identifier])
                } else {
                    self.not_supported(&format!("{kind}-not-supported"), &[&identifier])
                }
            }
            None => e,
        }
    }

    //
    // Parser and XMLReader methods
    //

    /// Parses the input source specified by the given system identifier.
    ///
    /// Equivalent to `parse(String)`.
    ///
    /// # Errors
    ///
    /// The `SAXException` or `IOException` of the parse.
    pub fn parse_system_id(&self, system_id: Option<&str>) -> JResult<()> {
        // parse document
        let source = XMLInputSource::new(None, system_id, None);
        self.parse_and_convert(&source)
    }

    /// Equivalent to `parse(InputSource)`.
    ///
    /// # Errors
    ///
    /// The `SAXException` or `IOException` of the parse.
    pub fn parse(&self, input_source: &InputSource) -> JResult<()> {
        // parse document
        let mut xml_input_source = XMLInputSource::new(
            input_source.get_public_id(),
            input_source.get_system_id(),
            None,
        );
        xml_input_source.set_byte_stream(input_source.get_byte_stream());
        xml_input_source.set_character_stream(input_source.get_character_stream());
        xml_input_source.set_encoding(input_source.get_encoding());
        self.parse_and_convert(&xml_input_source)
    }

    /// The common `try { parse(source); } catch (XMLParseException e) ...
    /// catch (XNIException e) ...` of the two `parse` methods.
    fn parse_and_convert(&self, source: &XMLInputSource) -> JResult<()> {
        let e = match self.parse_xml_input_source(source) {
            Ok(()) => return Ok(()),
            Err(Exception::Xni(e)) => e,
            Err(other) => return Err(other),
        };
        // wrap XNI exceptions as SAX exceptions
        if let Some(pe) = XMLParseException::from_xni_exception(&e) {
            let ex = pe.get_exception().cloned();
            let is_char_conversion = matches!(&ex, Some(Exception::Io(io)) if io.java_simple_name() == "CharConversionException");
            if ex.is_none() || is_char_conversion {
                // must be a parser exception; mine it for locator info
                // and throw a SAXParseException
                let mut locator_impl = Locator2Impl::new();
                locator_impl.set_xml_version(self.version.borrow().as_deref());
                locator_impl.base.set_public_id(pe.get_public_id());
                locator_impl.base.set_system_id(pe.get_expanded_system_id());
                locator_impl.base.set_line_number(pe.get_line_number());
                locator_impl.base.set_column_number(pe.get_column_number());
                return Err(match ex {
                    None => SAXParseException::with_locator(pe.get_message(), Some(&locator_impl))
                        .into(),
                    Some(ex) => SAXParseException::with_locator_and_exception(
                        pe.get_message(),
                        Some(&locator_impl),
                        Some(ex),
                    )
                    .into(),
                });
            }
            return Err(match ex.expect("INVARIANT: checked above") {
                ex @ Exception::Sax(_) => ex,
                ex @ Exception::Io(_) => ex,
                ex => SAXException::with_exception(ex).into(),
            });
        }
        match e.get_exception().cloned() {
            None => Err(SAXException::with_message(e.get_message()).into()),
            Some(ex @ Exception::Sax(_)) => Err(ex),
            Some(ex @ Exception::Io(_)) => Err(ex),
            Some(ex) => Err(SAXException::with_exception(ex).into()),
        }
    }

    /// Equivalent to `XMLParser.parse(XMLInputSource)`: `reset()`, then the
    /// configuration's parse.
    ///
    /// # Errors
    ///
    /// What the parse throws.
    pub fn parse_xml_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        self.reset()?;
        self.base.base.parse_after_reset(input_source)
    }

    /// Sets the resolver used to resolve external entities.
    ///
    /// Equivalent to `setEntityResolver(EntityResolver)`.
    pub fn set_entity_resolver(&self, resolver: Option<Rc<dyn EntityResolver>>) {
        let Ok(xer) = self.configuration().get_property(ENTITY_RESOLVER) else {
            // do nothing
            return;
        };
        let xer: Option<Rc<dyn XMLEntityResolver>> = xer.as_ref().and_then(object_as_interface);
        let is_resolver2 = resolver
            .as_ref()
            .is_some_and(|r| r.as_entity_resolver2().is_some());
        if self.use_entity_resolver2.get() && is_resolver2 {
            if let Some(er2w) = xer
                .as_ref()
                .and_then(|x| x.as_any().downcast_ref::<EntityResolver2Wrapper>())
            {
                er2w.set_entity_resolver(resolver);
            } else {
                let w: Rc<dyn XMLEntityResolver> =
                    Rc::new(EntityResolver2Wrapper::with_resolver(resolver));
                let _ = self
                    .configuration()
                    .set_property(ENTITY_RESOLVER, Some(interface_object(w)));
            }
        } else if let Some(erw) = xer
            .as_ref()
            .and_then(|x| x.as_any().downcast_ref::<EntityResolverWrapper>())
        {
            erw.set_entity_resolver(resolver);
        } else {
            let w: Rc<dyn XMLEntityResolver> =
                Rc::new(EntityResolverWrapper::with_resolver(resolver));
            let _ = self
                .configuration()
                .set_property(ENTITY_RESOLVER, Some(interface_object(w)));
        }
    }

    /// Return the current entity resolver.
    ///
    /// Equivalent to `getEntityResolver()`.
    pub fn get_entity_resolver(&self) -> Option<Rc<dyn EntityResolver>> {
        let Ok(Some(o)) = self.configuration().get_property(ENTITY_RESOLVER) else {
            return None;
        };
        let xml_entity_resolver: Rc<dyn XMLEntityResolver> = object_as_interface(&o)?;
        let any = xml_entity_resolver.as_any();
        if let Some(w) = any.downcast_ref::<EntityResolverWrapper>() {
            w.get_entity_resolver()
        } else if let Some(w) = any.downcast_ref::<EntityResolver2Wrapper>() {
            w.get_entity_resolver()
        } else {
            None
        }
    }

    /// Allow an application to register an error event handler.
    ///
    /// Equivalent to `setErrorHandler(ErrorHandler)`.
    pub fn set_error_handler(&self, error_handler: Option<Rc<dyn ErrorHandler>>) {
        let Ok(xeh) = self.configuration().get_property(ERROR_HANDLER) else {
            // do nothing
            return;
        };
        let xeh: Option<Rc<dyn XMLErrorHandler>> = xeh.as_ref().and_then(object_as_interface);
        if let Some(ehw) = xeh
            .as_ref()
            .and_then(|x| x.as_any().downcast_ref::<ErrorHandlerWrapper>())
        {
            ehw.set_error_handler(error_handler);
        } else {
            let w: Rc<dyn XMLErrorHandler> =
                Rc::new(ErrorHandlerWrapper::with_handler(error_handler));
            let _ = self
                .configuration()
                .set_property(ERROR_HANDLER, Some(interface_object(w)));
        }
    }

    /// Return the current error handler.
    ///
    /// Equivalent to `getErrorHandler()`.
    pub fn get_error_handler(&self) -> Option<Rc<dyn ErrorHandler>> {
        let Ok(Some(o)) = self.configuration().get_property(ERROR_HANDLER) else {
            return None;
        };
        let xml_error_handler: Rc<dyn XMLErrorHandler> = object_as_interface(&o)?;
        xml_error_handler
            .as_any()
            .downcast_ref::<ErrorHandlerWrapper>()
            .and_then(ErrorHandlerWrapper::get_error_handler)
    }

    /// Set the locale to use for messages.
    ///
    /// Equivalent to `setLocale(Locale)`.
    ///
    /// # Errors
    ///
    /// What the configuration throws.
    pub fn set_locale(&self, locale: Option<Locale>) -> JResult<()> {
        //REVISIT:this methods is not part of SAX2 interfaces, we should
        //throw exception if feature is not supported
        self.configuration().set_locale(locale)
    }

    /// Allow an application to register a DTD event handler.
    ///
    /// Equivalent to `setDTDHandler(DTDHandler)`.
    pub fn set_dtd_handler(&self, dtd_handler: Option<Rc<dyn DTDHandler>>) {
        *self.dtd_handler.borrow_mut() = dtd_handler;
    }

    /// Set the document handler (SAX 1).
    ///
    /// Equivalent to `setDocumentHandler(DocumentHandler)`.
    pub fn set_document_handler(&self, document_handler: Option<Rc<dyn DocumentHandler>>) {
        *self.document_handler.borrow_mut() = document_handler;
    }

    /// Allow an application to register a content event handler.
    ///
    /// Equivalent to `setContentHandler(ContentHandler)`.
    pub fn set_content_handler(&self, content_handler: Option<Rc<dyn ContentHandler>>) {
        *self.content_handler.borrow_mut() = content_handler;
    }

    /// Return the current content handler.
    ///
    /// Equivalent to `getContentHandler()`.
    pub fn get_content_handler(&self) -> Option<Rc<dyn ContentHandler>> {
        self.content_handler.borrow().clone()
    }

    /// Return the current DTD handler.
    ///
    /// Equivalent to `getDTDHandler()`.
    pub fn get_dtd_handler(&self) -> Option<Rc<dyn DTDHandler>> {
        self.dtd_handler.borrow().clone()
    }

    fn sax_suffix<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
        id.strip_prefix(prefix)
    }

    /// Set the state of any feature in a SAX2 parser.
    ///
    /// Equivalent to `setFeature(String, boolean)`.
    ///
    /// # Errors
    ///
    /// `SAXNotRecognizedException` or `SAXNotSupportedException`.
    pub fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()> {
        let r = self.set_feature0(feature_id, state);
        r.map_err(|e| self.convert_configuration_exception(e, "feature"))
    }

    fn set_feature0(&self, feature_id: &str, state: bool) -> JResult<()> {
        // SAX2 Features
        if let Some(suffix) = Self::sax_suffix(feature_id, constants::SAX_FEATURE_PREFIX) {
            // http://xml.org/sax/features/namespaces
            if suffix == constants::NAMESPACES_FEATURE {
                self.configuration().set_feature(feature_id, state)?;
                self.namespaces.set(state);
                return Ok(());
            }
            // http://xml.org/sax/features/namespace-prefixes
            //   controls the reporting of raw prefixed names and Namespace
            //   declarations (xmlns* attributes): when this feature is false
            //   (the default), raw prefixed names may optionally be reported,
            //   and xmlns* attributes must not be reported.
            if suffix == constants::NAMESPACE_PREFIXES_FEATURE {
                self.namespace_prefixes.set(state);
                return Ok(());
            }
            // http://xml.org/sax/features/string-interning
            //   controls the use of java.lang.String#intern() for strings
            //   passed to SAX handlers.
            if suffix == constants::STRING_INTERNING_FEATURE {
                if !state {
                    return Err(self.not_supported("false-not-supported", &[feature_id]));
                }
                return Ok(());
            }
            // http://xml.org/sax/features/lexical-handler/parameter-entities
            //   controls whether the beginning and end of parameter entities
            //   will be reported to the LexicalHandler.
            if suffix == constants::LEXICAL_HANDLER_PARAMETER_ENTITIES_FEATURE {
                self.lexical_handler_parameter_entities.set(state);
                return Ok(());
            }
            // http://xml.org/sax/features/resolve-dtd-uris
            //   controls whether system identifiers will be absolutized
            //   relative to their base URIs before reporting.
            if suffix == constants::RESOLVE_DTD_URIS_FEATURE {
                self.resolve_dtd_uris.set(state);
                return Ok(());
            }
            // http://xml.org/sax/features/unicode-normalization-checking
            //   controls whether Unicode normalization checking is
            //   performed as per Appendix B of the XML 1.1 specification
            if suffix == constants::UNICODE_NORMALIZATION_CHECKING_FEATURE {
                // REVISIT: Allow this feature to be set once Unicode
                // normalization checking is supported -- mrglavas.
                if state {
                    return Err(self.not_supported("true-not-supported", &[feature_id]));
                }
                return Ok(());
            }
            // http://xml.org/sax/features/xmlns-uris
            //   controls whether the parser reports that namespace
            //   declaration attributes as being in the namespace:
            //   http://www.w3.org/2000/xmlns/
            if suffix == constants::XMLNS_URIS_FEATURE {
                self.xmlns_uris.set(state);
                return Ok(());
            }
            // http://xml.org/sax/features/use-entity-resolver2
            //   controls whether the methods of an object implementing
            //   org.xml.sax.ext.EntityResolver2 will be used by the parser.
            if suffix == constants::USE_ENTITY_RESOLVER2_FEATURE {
                if state != self.use_entity_resolver2.get() {
                    self.use_entity_resolver2.set(state);
                    // Refresh EntityResolver wrapper.
                    self.set_entity_resolver(self.get_entity_resolver());
                }
                return Ok(());
            }
            //
            // Read only features.
            //

            // http://xml.org/sax/features/is-standalone
            //   reports whether the document specified a standalone
            //   document declaration.
            // http://xml.org/sax/features/use-attributes2
            //   reports whether Attributes objects passed to startElement
            //   also implement the org.xml.sax.ext.Attributes2 interface.
            // http://xml.org/sax/features/use-locator2
            //   reports whether Locator objects passed to setDocumentLocator
            //   also implement the org.xml.sax.ext.Locator2 interface.
            // http://xml.org/sax/features/xml-1.1
            //   reports whether the parser supports both XML 1.1 and XML 1.0.
            if suffix == constants::IS_STANDALONE_FEATURE
                || suffix == constants::USE_ATTRIBUTES2_FEATURE
                || suffix == constants::USE_LOCATOR2_FEATURE
                || suffix == constants::XML_11_FEATURE
            {
                return Err(self.not_supported("feature-read-only", &[feature_id]));
            }

            //
            // Drop through and perform default processing
            //
        }

        //
        // Xerces Features
        //

        // Default handling
        self.configuration().set_feature(feature_id, state)
    }

    /// Query the state of a feature.
    ///
    /// Equivalent to `getFeature(String)`.
    ///
    /// # Errors
    ///
    /// `SAXNotRecognizedException` or `SAXNotSupportedException`.
    pub fn get_feature(&self, feature_id: &str) -> JResult<bool> {
        let r = self.get_feature0(feature_id);
        r.map_err(|e| self.convert_configuration_exception(e, "feature"))
    }

    fn get_feature0(&self, feature_id: &str) -> JResult<bool> {
        // SAX2 Features
        if let Some(suffix) = Self::sax_suffix(feature_id, constants::SAX_FEATURE_PREFIX) {
            // http://xml.org/sax/features/namespace-prefixes
            if suffix == constants::NAMESPACE_PREFIXES_FEATURE {
                return Ok(self.namespace_prefixes.get());
            }
            // http://xml.org/sax/features/string-interning
            if suffix == constants::STRING_INTERNING_FEATURE {
                return Ok(true);
            }
            // http://xml.org/sax/features/is-standalone
            if suffix == constants::IS_STANDALONE_FEATURE {
                return Ok(self.standalone.get());
            }
            // http://xml.org/sax/features/xml-1.1
            if suffix == constants::XML_11_FEATURE {
                return Ok(self.configuration().is_xml11_configurable());
            }
            // http://xml.org/sax/features/lexical-handler/parameter-entities
            if suffix == constants::LEXICAL_HANDLER_PARAMETER_ENTITIES_FEATURE {
                return Ok(self.lexical_handler_parameter_entities.get());
            }
            // http://xml.org/sax/features/resolve-dtd-uris
            if suffix == constants::RESOLVE_DTD_URIS_FEATURE {
                return Ok(self.resolve_dtd_uris.get());
            }
            // http://xml.org/sax/features/xmlns-uris
            if suffix == constants::XMLNS_URIS_FEATURE {
                return Ok(self.xmlns_uris.get());
            }
            // http://xml.org/sax/features/unicode-normalization-checking
            if suffix == constants::UNICODE_NORMALIZATION_CHECKING_FEATURE {
                // REVISIT: Allow this feature to be set once Unicode
                // normalization checking is supported -- mrglavas.
                return Ok(false);
            }
            // http://xml.org/sax/features/use-entity-resolver2
            if suffix == constants::USE_ENTITY_RESOLVER2_FEATURE {
                return Ok(self.use_entity_resolver2.get());
            }
            // http://xml.org/sax/features/use-attributes2
            // http://xml.org/sax/features/use-locator2
            if suffix == constants::USE_ATTRIBUTES2_FEATURE
                || suffix == constants::USE_LOCATOR2_FEATURE
            {
                return Ok(true);
            }

            //
            // Drop through and perform default processing
            //
        }

        //
        // Xerces Features
        //
        self.configuration().get_feature(feature_id)
    }

    /// Set the value of any property in a SAX2 parser.
    ///
    /// Equivalent to `setProperty(String, Object)`.
    ///
    /// # Errors
    ///
    /// `SAXNotRecognizedException` or `SAXNotSupportedException`.
    pub fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        let r = self.set_property0(property_id, value);
        r.map_err(|e| self.convert_configuration_exception(e, "property"))
    }

    fn set_property0(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        // SAX2 core properties
        if let Some(suffix) = Self::sax_suffix(property_id, constants::SAX_PROPERTY_PREFIX) {
            // http://xml.org/sax/properties/lexical-handler
            // Value type: org.xml.sax.ext.LexicalHandler
            // Access: read/write, pre-parse only
            //   Set the lexical event handler.
            if suffix == constants::LEXICAL_HANDLER_PROPERTY {
                let handler = match &value {
                    None => None,
                    Some(v) => match object_as_interface::<dyn LexicalHandler>(v) {
                        Some(h) => Some(h),
                        None => {
                            return Err(self.not_supported(
                                "incompatible-class",
                                &[property_id, "org.xml.sax.ext.LexicalHandler"],
                            ))
                        }
                    },
                };
                return self.set_lexical_handler(handler);
            }
            // http://xml.org/sax/properties/declaration-handler
            // Value type: org.xml.sax.ext.DeclHandler
            // Access: read/write, pre-parse only
            //   Set the DTD declaration event handler.
            if suffix == constants::DECLARATION_HANDLER_PROPERTY {
                let handler = match &value {
                    None => None,
                    Some(v) => match object_as_interface::<dyn DeclHandler>(v) {
                        Some(h) => Some(h),
                        None => {
                            return Err(self.not_supported(
                                "incompatible-class",
                                &[property_id, "org.xml.sax.ext.DeclHandler"],
                            ))
                        }
                    },
                };
                return self.set_decl_handler(handler);
            }
            // http://xml.org/sax/properties/dom-node
            // Value type: DOM Node
            // Access: read-only
            //   Get the DOM node currently being visited, if the SAX parser
            //   is iterating over a DOM tree.  If the parser recognises and
            //   supports this property but is not currently visiting a DOM
            //   node, it should return null (this is a good way to check
            //   for availability before the parse begins).
            // http://xml.org/sax/properties/document-xml-version
            // Value type: java.lang.String
            // Access: read-only
            //   The literal string describing the actual XML version of the
            //   document.
            if suffix == constants::DOM_NODE_PROPERTY
                || suffix == constants::DOCUMENT_XML_VERSION_PROPERTY
            {
                return Err(self.not_supported("property-read-only", &[property_id]));
            }
            //
            // Drop through and perform default processing
            //
        }

        //
        // Xerces Properties
        //

        //
        // Perform default processing
        //
        self.configuration().set_property(property_id, value)
    }

    /// Query the value of a property.
    ///
    /// Equivalent to `getProperty(String)`.
    ///
    /// # Errors
    ///
    /// `SAXNotRecognizedException` or `SAXNotSupportedException`.
    pub fn get_property(&self, property_id: &str) -> JResult<Option<Object>> {
        let r = self.get_property0(property_id);
        r.map_err(|e| self.convert_configuration_exception(e, "property"))
    }

    fn get_property0(&self, property_id: &str) -> JResult<Option<Object>> {
        // SAX2 core properties
        if let Some(suffix) = Self::sax_suffix(property_id, constants::SAX_PROPERTY_PREFIX) {
            // http://xml.org/sax/properties/document-xml-version
            // Value type: java.lang.String
            // Access: read-only
            //   The literal string describing the actual XML version of the
            //   document.
            if suffix == constants::DOCUMENT_XML_VERSION_PROPERTY {
                return Ok(self.version.borrow().as_deref().map(string_object));
            }
            // http://xml.org/sax/properties/lexical-handler
            // Value type: org.xml.sax.ext.LexicalHandler
            // Access: read/write, pre-parse only
            //   Set the lexical event handler.
            if suffix == constants::LEXICAL_HANDLER_PROPERTY {
                return Ok(self.get_lexical_handler().map(interface_object));
            }
            // http://xml.org/sax/properties/declaration-handler
            // Value type: org.xml.sax.ext.DeclHandler
            // Access: read/write, pre-parse only
            //   Set the DTD declaration event handler.
            if suffix == constants::DECLARATION_HANDLER_PROPERTY {
                return Ok(self.get_decl_handler().map(interface_object));
            }
            // http://xml.org/sax/properties/dom-node
            // Value type: DOM Node
            // Access: read-only
            //   Get the DOM node currently being visited, if the SAX parser
            //   is iterating over a DOM tree.  If the parser recognises and
            //   supports this property but is not currently visiting a DOM
            //   node, it should return null (this is a good way to check
            //   for availability before the parse begins).
            if suffix == constants::DOM_NODE_PROPERTY {
                // we are not iterating a DOM tree
                return Err(match self.format("dom-node-read-not-supported", None) {
                    Ok(m) => SAXNotSupportedException::with_message(Some(&m)).into(),
                    Err(e) => e,
                });
            }
            //
            // Drop through and perform default processing
            //
        }

        //
        // Xerces properties
        //

        //
        // Perform default processing
        //
        self.configuration().get_property(property_id)
    }

    /// Set the DTD declaration event handler.
    ///
    /// Equivalent to the protected `setDeclHandler(DeclHandler)`.
    ///
    /// # Errors
    ///
    /// `SAXNotSupportedException` while a parse is in progress.
    pub fn set_decl_handler(&self, handler: Option<Rc<dyn DeclHandler>>) -> JResult<()> {
        if self.parse_in_progress.get() {
            return Err(self.not_supported(
                "property-not-parsing-supported",
                &["http://xml.org/sax/properties/declaration-handler"],
            ));
        }
        *self.decl_handler.borrow_mut() = handler;
        Ok(())
    }

    /// Returns the DTD declaration event handler.
    ///
    /// Equivalent to the protected `getDeclHandler()`.
    pub fn get_decl_handler(&self) -> Option<Rc<dyn DeclHandler>> {
        self.decl_handler.borrow().clone()
    }

    /// Set the lexical event handler.
    ///
    /// Equivalent to the protected `setLexicalHandler(LexicalHandler)`.
    ///
    /// # Errors
    ///
    /// `SAXNotSupportedException` while a parse is in progress.
    pub fn set_lexical_handler(&self, handler: Option<Rc<dyn LexicalHandler>>) -> JResult<()> {
        if self.parse_in_progress.get() {
            return Err(self.not_supported(
                "property-not-parsing-supported",
                &["http://xml.org/sax/properties/lexical-handler"],
            ));
        }
        *self.lexical_handler.borrow_mut() = handler;
        Ok(())
    }

    /// Returns the lexical handler.
    ///
    /// Equivalent to the protected `getLexicalHandler()`.
    pub fn get_lexical_handler(&self) -> Option<Rc<dyn LexicalHandler>> {
        self.lexical_handler.borrow().clone()
    }

    /// Send startPrefixMapping events.
    ///
    /// Equivalent to the protected `startNamespaceMapping()`.
    fn start_namespace_mapping(&self, content_handler: &dyn ContentHandler) -> JResult<()> {
        let nc = self
            .namespace_context
            .borrow()
            .clone()
            .ok_or_else(|| Exception::runtime(rucene::error::LuceneError::null_pointer()))?;
        let count = nc.get_declared_prefix_count();
        for i in 0..count {
            let prefix = nc.get_declared_prefix_at(i);
            let uri = nc.get_uri(&prefix);
            content_handler.start_prefix_mapping(&prefix, uri.as_deref().unwrap_or(""))?;
        }
        Ok(())
    }

    /// Send endPrefixMapping events.
    ///
    /// Equivalent to the protected `endNamespaceMapping()`.
    fn end_namespace_mapping(&self, content_handler: &dyn ContentHandler) -> JResult<()> {
        let nc = self
            .namespace_context
            .borrow()
            .clone()
            .ok_or_else(|| Exception::runtime(rucene::error::LuceneError::null_pointer()))?;
        let count = nc.get_declared_prefix_count();
        for i in 0..count {
            content_handler.end_prefix_mapping(&nc.get_declared_prefix_at(i))?;
        }
        Ok(())
    }

    /// Reset all components before parsing.
    ///
    /// Equivalent to `reset()`.
    ///
    /// # Errors
    ///
    /// The `XMLConfigurationException` of reading the namespaces feature.
    pub fn reset(&self) -> JResult<()> {
        self.base.reset();

        // reset state
        self.base.in_dtd.set(false);
        *self.version.borrow_mut() = Some("1.0".to_string());
        self.standalone.set(false);

        // features
        self.namespaces
            .set(self.configuration().get_feature(NAMESPACES)?);
        *self.augmentations.borrow_mut() = None;
        *self.declared_attrs.borrow_mut() = None;
        Ok(())
    }

    fn snapshot_attributes(&self, attributes: &dyn XMLAttributes) {
        let mut snapshot = self.attributes.borrow_mut();
        snapshot.clear();
        for i in 0..attributes.get_length() {
            snapshot.push((
                attributes.get_uri(i),
                attributes.get_local_name(i),
                attributes.get_augmentations(i),
            ));
        }
    }

    fn snapshot_augmentations(&self, augs: Option<&dyn Augmentations>) {
        *self.augmentations.borrow_mut() = augs.map(|a| a.get_item(constants::ELEMENT_PSVI));
    }
}

//
// XMLDocumentHandler methods
//

impl XMLDocumentHandler for AbstractSAXParser {
    /// The start of the document.
    fn start_document(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        _encoding: Option<&str>,
        namespace_context: Option<Rc<dyn NamespaceContext>>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        *self.namespace_context.borrow_mut() = namespace_context;
        (|| -> JResult<()> {
            // SAX1
            let document_handler = self.document_handler.borrow().clone();
            if let Some(dh) = document_handler {
                if let Some(locator) = &locator {
                    dh.set_document_locator(Rc::new(LocatorProxy::new(Rc::clone(locator))));
                }
                // The application may have set the DocumentHandler to null
                // within setDocumentLocator() so we need to check again.
                let dh_handler = self.document_handler.borrow().clone();
                if let Some(dh) = dh_handler {
                    dh.start_document()?;
                }
            }

            // SAX2
            let content_handler = self.content_handler.borrow().clone();
            if let Some(ch) = content_handler {
                if let Some(locator) = &locator {
                    ch.set_document_locator(Rc::new(LocatorProxy::new(Rc::clone(locator))));
                }
                // The application may have set the ContentHandler to null
                // within setDocumentLocator() so we need to check again.
                let ch_handler = self.content_handler.borrow().clone();
                if let Some(ch) = ch_handler {
                    ch.start_document()?;
                }
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// Notifies of the presence of an XMLDecl line in the document.
    fn xml_decl(
        &self,
        version: Option<&str>,
        _encoding: Option<&str>,
        standalone: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        // the version need only be set once; if
        // document's XML 1.0|1.1, that's how it'll stay
        *self.version.borrow_mut() = version.map(str::to_string);
        self.standalone.set(standalone == Some("yes"));
        Ok(())
    }

    /// Notifies of the presence of the DOCTYPE line in the document.
    fn doctype_decl(
        &self,
        root_element: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.in_dtd.set(true);

        let lh = self.lexical_handler.borrow().clone();
        if let Some(lh) = lh {
            lh.start_dtd(root_element.unwrap_or(""), public_id, system_id)
                .map_err(xni)?;
        }

        // is there a DeclHandler?
        if self.decl_handler.borrow().is_some() {
            *self.declared_attrs.borrow_mut() = Some(SymbolHash::with_size(25));
        }
        Ok(())
    }

    fn comment(&self, text: &XMLString, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        let lh = self.lexical_handler.borrow().clone();
        if let Some(lh) = lh {
            let ch = text.ch.as_deref().unwrap_or(&[]);
            lh.comment(ch, 0, text.length).map_err(xni)?;
        }
        Ok(())
    }

    fn processing_instruction(
        &self,
        target: &str,
        data: Option<&XMLString>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        //
        // REVISIT - I keep running into SAX apps that expect
        //   null data to be an empty string, which is contrary
        //   to the comment for this method in the SAX API.
        //
        let data = data.map(XMLString::to_java_string);
        (|| -> JResult<()> {
            let dh_handler = self.document_handler.borrow().clone();
            if let Some(dh) = dh_handler {
                dh.processing_instruction(target, data.as_deref())?;
            }
            let ch_handler = self.content_handler.borrow().clone();
            if let Some(ch) = ch_handler {
                ch.processing_instruction(target, data.as_deref())?;
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// The start of an element.
    fn start_element(
        &self,
        element: &mut QName,
        attributes: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        (|| -> JResult<()> {
            // SAX1
            let dh_handler = self.document_handler.borrow().clone();
            if let Some(dh) = dh_handler {
                // REVISIT: should we support schema-normalized-value for SAX1
                // events?
                self.snapshot_attributes(&*attributes);
                let mut proxy = AttributesProxy::new();
                proxy.set_attributes(&*attributes);
                dh.start_element(element.rawname.as_deref().unwrap_or(""), &proxy)?;
            }

            // SAX2
            let ch_handler = self.content_handler.borrow().clone();
            if let Some(ch) = ch_handler {
                if self.namespaces.get() {
                    // send prefix mapping events
                    self.start_namespace_mapping(&*ch)?;

                    // REVISIT: It should not be necessary to iterate over the
                    // attribute list when the set of [namespace attributes] is
                    // empty for this element. This should be computable from the
                    // NamespaceContext, but since we currently don't report the
                    // mappings for the xml prefix we cannot use the declared
                    // prefix count for the current context to skip this section.
                    // -- mrglavas
                    let len = attributes.get_length();
                    if !self.namespace_prefixes.get() {
                        let mut qname = self.qname.borrow_mut();
                        for i in (0..len).rev() {
                            attributes.get_name(i, &mut qname);
                            if qname.prefix.as_deref() == Some(xml_symbols::PREFIX_XMLNS)
                                || qname.rawname.as_deref() == Some(xml_symbols::PREFIX_XMLNS)
                            {
                                // remove namespace declaration attributes
                                attributes.remove_attribute_at(i);
                            }
                        }
                    } else if !self.xmlns_uris.get() {
                        let mut qname = self.qname.borrow_mut();
                        for i in (0..len).rev() {
                            attributes.get_name(i, &mut qname);
                            if qname.prefix.as_deref() == Some(xml_symbols::PREFIX_XMLNS)
                                || qname.rawname.as_deref() == Some(xml_symbols::PREFIX_XMLNS)
                            {
                                // localpart should be empty string as per SAX
                                // documentation:
                                // http://www.saxproject.org/?selected=namespaces
                                qname.prefix = Some(String::new());
                                qname.uri = Some(String::new());
                                qname.localpart = Some(String::new());
                                attributes.set_name(i, &qname);
                            }
                        }
                    }
                }

                self.snapshot_augmentations(augs);

                let uri = element.uri.clone().unwrap_or_default();
                let localpart = if self.namespaces.get() {
                    element.localpart.clone().unwrap_or_default()
                } else {
                    String::new()
                };
                self.snapshot_attributes(&*attributes);
                let mut proxy = AttributesProxy::new();
                proxy.set_attributes(&*attributes);
                ch.start_element(
                    &uri,
                    &localpart,
                    element.rawname.as_deref().unwrap_or(""),
                    &proxy,
                )?;
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// An empty element: `AbstractXMLDocumentParser`'s body.
    fn empty_element(
        &self,
        element: &mut QName,
        attributes: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        AbstractXMLDocumentParser::empty_element(self, element, attributes, augs)
    }

    /// This method notifies of the start of an entity.
    fn start_general_entity(
        &self,
        name: &str,
        _identifier: Option<&dyn XMLResourceIdentifier>,
        _encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        (|| -> JResult<()> {
            // Only report startEntity if this entity was actually read.
            if is_true(augs, constants::ENTITY_SKIPPED) {
                // report skipped entity to content handler
                let ch_handler = self.content_handler.borrow().clone();
                if let Some(ch) = ch_handler {
                    ch.skipped_entity(name)?;
                }
            } else {
                // SAX2 extension
                let lh_handler = self.lexical_handler.borrow().clone();
                if let Some(lh) = lh_handler {
                    lh.start_entity(name)?;
                }
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn text_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.text_decl(version, encoding, augs)
    }

    /// This method notifies the end of an entity.
    fn end_general_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()> {
        (|| -> JResult<()> {
            // Only report endEntity if this entity was actually read.
            if !is_true(augs, constants::ENTITY_SKIPPED) {
                // SAX2 extension
                let lh_handler = self.lexical_handler.borrow().clone();
                if let Some(lh) = lh_handler {
                    lh.end_entity(name)?;
                }
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// Character content.
    fn characters(&self, text: &XMLString, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        // if type is union (XML Schema) it is possible that we receive
        // character call with empty data
        if text.length == 0 {
            return Ok(());
        }
        let ch = text.ch.as_deref().unwrap_or(&[]);
        (|| -> JResult<()> {
            // SAX1
            let dh_handler = self.document_handler.borrow().clone();
            if let Some(dh) = dh_handler {
                // REVISIT: should we support schema-normalized-value for SAX1
                // events?
                dh.characters(ch, text.offset, text.length)?;
            }

            // SAX2
            let h_handler = self.content_handler.borrow().clone();
            if let Some(h) = h_handler {
                h.characters(ch, text.offset, text.length)?;
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// Ignorable whitespace.
    fn ignorable_whitespace(
        &self,
        text: &XMLString,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let ch = text.ch.as_deref().unwrap_or(&[]);
        (|| -> JResult<()> {
            // SAX1
            let dh_handler = self.document_handler.borrow().clone();
            if let Some(dh) = dh_handler {
                dh.ignorable_whitespace(ch, text.offset, text.length)?;
            }

            // SAX2
            let h_handler = self.content_handler.borrow().clone();
            if let Some(h) = h_handler {
                h.ignorable_whitespace(ch, text.offset, text.length)?;
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// The end of an element.
    fn end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        (|| -> JResult<()> {
            // SAX1
            let dh_handler = self.document_handler.borrow().clone();
            if let Some(dh) = dh_handler {
                dh.end_element(element.rawname.as_deref().unwrap_or(""))?;
            }

            // SAX2
            let ch_handler = self.content_handler.borrow().clone();
            if let Some(ch) = ch_handler {
                self.snapshot_augmentations(augs);
                let uri = element.uri.clone().unwrap_or_default();
                let localpart = if self.namespaces.get() {
                    element.localpart.clone().unwrap_or_default()
                } else {
                    String::new()
                };
                ch.end_element(&uri, &localpart, element.rawname.as_deref().unwrap_or(""))?;
                if self.namespaces.get() {
                    self.end_namespace_mapping(&*ch)?;
                }
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// The start of a CDATA section.
    fn start_cdata(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        let lh_handler = self.lexical_handler.borrow().clone();
        if let Some(lh) = lh_handler {
            lh.start_cdata().map_err(xni)?;
        }
        Ok(())
    }

    /// The end of a CDATA section.
    fn end_cdata(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        let lh_handler = self.lexical_handler.borrow().clone();
        if let Some(lh) = lh_handler {
            lh.end_cdata().map_err(xni)?;
        }
        Ok(())
    }

    /// The end of the document.
    fn end_document(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        (|| -> JResult<()> {
            // SAX1
            let dh_handler = self.document_handler.borrow().clone();
            if let Some(dh) = dh_handler {
                dh.end_document()?;
            }

            // SAX2
            let ch_handler = self.content_handler.borrow().clone();
            if let Some(ch) = ch_handler {
                ch.end_document()?;
            }
            Ok(())
        })()
        .map_err(xni)
    }

    fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>) {
        self.base.set_document_source(source);
    }

    fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>> {
        self.base.get_document_source()
    }
}

//
// XMLDTDHandler methods
//

impl XMLDTDHandler for AbstractSAXParser {
    /// `AbstractXMLDocumentParser`'s body.
    fn start_dtd(
        &self,
        _locator: Option<Rc<dyn XMLLocator>>,
        _augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.start_dtd();
        Ok(())
    }

    /// This method notifies of the start of parameter entity.
    fn start_parameter_entity(
        &self,
        name: &str,
        _identifier: Option<&dyn XMLResourceIdentifier>,
        _encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        (|| -> JResult<()> {
            // Only report startEntity if this entity was actually read.
            if is_true(augs, constants::ENTITY_SKIPPED) {
                // report skipped entity to content handler
                let ch_handler = self.content_handler.borrow().clone();
                if let Some(ch) = ch_handler {
                    ch.skipped_entity(name)?;
                }
            } else {
                // SAX2 extension
                let lh_handler = self.lexical_handler.borrow().clone();
                if let Some(lh) = lh_handler {
                    if self.lexical_handler_parameter_entities.get() {
                        lh.start_entity(name)?;
                    }
                }
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn text_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.text_decl(version, encoding, augmentations)
    }

    /// This method notifies the end of a parameter entity.
    fn end_parameter_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()> {
        (|| -> JResult<()> {
            // Only report endEntity if this entity was actually read.
            if !is_true(augs, constants::ENTITY_SKIPPED) {
                // SAX2 extension
                let lh_handler = self.lexical_handler.borrow().clone();
                if let Some(lh) = lh_handler {
                    if self.lexical_handler_parameter_entities.get() {
                        lh.end_entity(name)?;
                    }
                }
            }
            Ok(())
        })()
        .map_err(xni)
    }

    /// The start of the DTD external subset.
    fn start_external_subset(
        &self,
        _identifier: Option<&dyn XMLResourceIdentifier>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        XMLDTDHandler::start_parameter_entity(self, "[dtd]", None, None, augs)
    }

    /// The end of the DTD external subset.
    fn end_external_subset(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        XMLDTDHandler::end_parameter_entity(self, "[dtd]", augs)
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn comment(
        &self,
        _text: &XMLString,
        _augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn processing_instruction(
        &self,
        _target: &str,
        _data: Option<&XMLString>,
        _augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        Ok(())
    }

    /// An element declaration.
    fn element_decl(
        &self,
        name: &str,
        content_model: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let dh_handler = self.decl_handler.borrow().clone();
        if let Some(dh) = dh_handler {
            dh.element_decl(name, content_model).map_err(xni)?;
        }
        Ok(())
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn start_attlist(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.start_attlist(element_name, augmentations)
    }

    /// An attribute declaration.
    fn attribute_decl(
        &self,
        element_name: &str,
        attribute_name: &str,
        type_: &str,
        enumeration: Option<&[String]>,
        default_type: Option<&str>,
        default_value: Option<&XMLString>,
        _non_normalized_default_value: Option<&XMLString>,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let Some(dh) = self.decl_handler.borrow().clone() else {
            return Ok(());
        };
        // used as a key to detect duplicate attribute definitions.
        let elem_attr = format!("{element_name}<{attribute_name}");
        {
            let mut declared = self.declared_attrs.borrow_mut();
            let declared = declared
                .as_mut()
                .ok_or_else(|| Exception::runtime(rucene::error::LuceneError::null_pointer()))?;
            if declared.get(&elem_attr).is_some() {
                // we aren't permitted to return duplicate attribute
                // definitions
                return Ok(());
            }
            declared.put(&elem_attr, Some(Rc::new(true)));
        }
        let mut type_ = type_.to_string();
        if type_ == "NOTATION" || type_ == "ENUMERATION" {
            let mut str = String::new();
            if type_ == "NOTATION" {
                str.push_str(&type_);
                str.push_str(" (");
            } else {
                str.push('(');
            }
            let enumeration = enumeration.unwrap_or(&[]);
            for (i, e) in enumeration.iter().enumerate() {
                str.push_str(e);
                if i < enumeration.len() - 1 {
                    str.push('|');
                }
            }
            str.push(')');
            type_ = str;
        }
        let value = default_value.map(XMLString::to_java_string);
        dh.attribute_decl(
            element_name,
            attribute_name,
            &type_,
            default_type,
            value.as_deref(),
        )
        .map_err(xni)
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn end_attlist(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_attlist(augmentations)
    }

    /// An internal entity declaration.
    fn internal_entity_decl(
        &self,
        name: &str,
        text: &XMLString,
        _non_normalized_text: &XMLString,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let dh_handler = self.decl_handler.borrow().clone();
        if let Some(dh) = dh_handler {
            dh.internal_entity_decl(name, &text.to_java_string())
                .map_err(xni)?;
        }
        Ok(())
    }

    /// An external entity declaration.
    fn external_entity_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let dh_handler = self.decl_handler.borrow().clone();
        if let Some(dh) = dh_handler {
            let public_id = identifier.get_public_id();
            let system_id = if self.resolve_dtd_uris.get() {
                identifier.get_expanded_system_id()
            } else {
                identifier.get_literal_system_id()
            };
            dh.external_entity_decl(name, public_id.as_deref(), system_id.as_deref())
                .map_err(xni)?;
        }
        Ok(())
    }

    /// An unparsed entity declaration.
    fn unparsed_entity_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        notation: &str,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let dh_handler = self.dtd_handler.borrow().clone();
        if let Some(dh) = dh_handler {
            let public_id = identifier.get_public_id();
            let system_id = if self.resolve_dtd_uris.get() {
                identifier.get_expanded_system_id()
            } else {
                identifier.get_literal_system_id()
            };
            dh.unparsed_entity_decl(name, public_id.as_deref(), system_id.as_deref(), notation)
                .map_err(xni)?;
        }
        Ok(())
    }

    /// A notation declaration.
    fn notation_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        _augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        let dh_handler = self.dtd_handler.borrow().clone();
        if let Some(dh) = dh_handler {
            let public_id = identifier.get_public_id();
            let system_id = if self.resolve_dtd_uris.get() {
                identifier.get_expanded_system_id()
            } else {
                identifier.get_literal_system_id()
            };
            dh.notation_decl(name, public_id.as_deref(), system_id.as_deref())
                .map_err(xni)?;
        }
        Ok(())
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn start_conditional(
        &self,
        type_: i16,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.start_conditional(type_, augmentations)
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn ignored_characters(
        &self,
        text: &XMLString,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.ignored_characters(text, augmentations)
    }

    /// `AbstractXMLDocumentParser`'s empty body.
    fn end_conditional(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_conditional(augmentations)
    }

    /// The end of the DTD.
    fn end_dtd(&self, _augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.in_dtd.set(false);

        let lh_handler = self.lexical_handler.borrow().clone();

        if let Some(lh) = lh_handler {
            lh.end_dtd().map_err(xni)?;
        }
        if let Some(declared) = self.declared_attrs.borrow_mut().as_mut() {
            // help out the GC
            declared.clear();
        }
        Ok(())
    }

    fn set_dtd_source(&self, source: Option<Rc<dyn XMLDTDSource>>) {
        self.base.set_dtd_source(source);
    }

    fn get_dtd_source(&self) -> Option<Rc<dyn XMLDTDSource>> {
        self.base.get_dtd_source()
    }
}

//
// XMLDTDContentModelHandler methods: AbstractXMLDocumentParser's empty
// bodies
//

impl XMLDTDContentModelHandler for AbstractSAXParser {
    fn start_content_model(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.start_content_model(element_name, augmentations)
    }

    fn any(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.any(augmentations)
    }

    fn empty(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.empty(augmentations)
    }

    fn start_group(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.start_group(augmentations)
    }

    fn pcdata(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.pcdata(augmentations)
    }

    fn element(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.element(element_name, augmentations)
    }

    fn separator(&self, separator: i16, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.separator(separator, augmentations)
    }

    fn occurrence(
        &self,
        occurrence: i16,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.occurrence(occurrence, augmentations)
    }

    fn end_group(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_group(augmentations)
    }

    fn end_content_model(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_content_model(augmentations)
    }

    fn set_dtd_content_model_source(&self, source: Option<Rc<dyn XMLDTDContentModelSource>>) {
        self.base.set_dtd_content_model_source(source);
    }

    fn get_dtd_content_model_source(&self) -> Option<Rc<dyn XMLDTDContentModelSource>> {
        self.base.get_dtd_content_model_source()
    }
}

//
// PSVIProvider methods
//

impl PSVIProvider for AbstractSAXParser {
    fn get_element_psvi(&self) -> Option<Rc<dyn ElementPSVI>> {
        self.augmentations
            .borrow()
            .as_ref()
            .and_then(|item| item.as_ref())
            .and_then(object_as_interface::<dyn ElementPSVI>)
    }

    /// # Panics
    ///
    /// For an index without attribute, where Java throws
    /// `NullPointerException`.
    fn get_attribute_psvi(&self, index: i32) -> Option<Rc<dyn AttributePSVI>> {
        let attributes = self.attributes.borrow();
        let augs = attributes
            .get(index as usize)
            .and_then(|a| a.2.clone())
            .expect("java.lang.NullPointerException");
        augs.get_item(constants::ATTRIBUTE_PSVI)
            .as_ref()
            .and_then(object_as_interface::<dyn AttributePSVI>)
    }

    /// # Panics
    ///
    /// For a name without attribute, where Java throws
    /// `NullPointerException`.
    fn get_attribute_psvi_by_name(
        &self,
        uri: Option<&str>,
        localname: Option<&str>,
    ) -> Option<Rc<dyn AttributePSVI>> {
        let attributes = self.attributes.borrow();
        let augs = attributes
            .iter()
            .find(|a| a.1.as_deref() == localname && a.0.as_deref() == uri)
            .and_then(|a| a.2.clone())
            .expect("java.lang.NullPointerException");
        augs.get_item(constants::ATTRIBUTE_PSVI)
            .as_ref()
            .and_then(object_as_interface::<dyn AttributePSVI>)
    }
}

impl Parser for AbstractSAXParser {
    fn set_locale(&self, locale: Option<Locale>) -> JResult<()> {
        AbstractSAXParser::set_locale(self, locale)
    }

    fn set_entity_resolver(&self, resolver: Option<Rc<dyn EntityResolver>>) {
        AbstractSAXParser::set_entity_resolver(self, resolver);
    }

    fn set_dtd_handler(&self, handler: Option<Rc<dyn DTDHandler>>) {
        AbstractSAXParser::set_dtd_handler(self, handler);
    }

    fn set_document_handler(&self, handler: Option<Rc<dyn DocumentHandler>>) {
        AbstractSAXParser::set_document_handler(self, handler);
    }

    fn set_error_handler(&self, handler: Option<Rc<dyn ErrorHandler>>) {
        AbstractSAXParser::set_error_handler(self, handler);
    }

    fn parse(&self, source: &InputSource) -> JResult<()> {
        AbstractSAXParser::parse(self, source)
    }

    fn parse_system_id(&self, system_id: Option<&str>) -> JResult<()> {
        AbstractSAXParser::parse_system_id(self, system_id)
    }
}

impl XMLReader for AbstractSAXParser {
    fn get_feature(&self, name: &str) -> JResult<bool> {
        AbstractSAXParser::get_feature(self, name)
    }

    fn set_feature(&self, name: &str, value: bool) -> JResult<()> {
        AbstractSAXParser::set_feature(self, name, value)
    }

    fn get_property(&self, name: &str) -> JResult<Option<Object>> {
        AbstractSAXParser::get_property(self, name)
    }

    fn set_property(&self, name: &str, value: Option<Object>) -> JResult<()> {
        AbstractSAXParser::set_property(self, name, value)
    }

    fn set_entity_resolver(&self, resolver: Option<Rc<dyn EntityResolver>>) {
        AbstractSAXParser::set_entity_resolver(self, resolver);
    }

    fn get_entity_resolver(&self) -> Option<Rc<dyn EntityResolver>> {
        AbstractSAXParser::get_entity_resolver(self)
    }

    fn set_dtd_handler(&self, handler: Option<Rc<dyn DTDHandler>>) {
        AbstractSAXParser::set_dtd_handler(self, handler);
    }

    fn get_dtd_handler(&self) -> Option<Rc<dyn DTDHandler>> {
        AbstractSAXParser::get_dtd_handler(self)
    }

    fn set_content_handler(&self, handler: Option<Rc<dyn ContentHandler>>) {
        AbstractSAXParser::set_content_handler(self, handler);
    }

    fn get_content_handler(&self) -> Option<Rc<dyn ContentHandler>> {
        AbstractSAXParser::get_content_handler(self)
    }

    fn set_error_handler(&self, handler: Option<Rc<dyn ErrorHandler>>) {
        AbstractSAXParser::set_error_handler(self, handler);
    }

    fn get_error_handler(&self) -> Option<Rc<dyn ErrorHandler>> {
        AbstractSAXParser::get_error_handler(self)
    }

    fn parse(&self, input: &InputSource) -> JResult<()> {
        AbstractSAXParser::parse(self, input)
    }

    fn parse_system_id(&self, system_id: Option<&str>) -> JResult<()> {
        AbstractSAXParser::parse_system_id(self, system_id)
    }
}

//
// AbstractSAXParserHandler: the weak forwarding handler
//

impl AbstractSAXParserHandler {
    fn parser(&self) -> Option<Rc<AbstractSAXParser>> {
        self.0.upgrade()
    }
}

macro_rules! forward {
    ($self:ident, $trait:ident :: $m:ident ( $($a:expr),* )) => {
        match $self.parser() {
            Some(p) => $trait::$m(&*p, $($a),*),
            None => Ok(()),
        }
    };
}

impl XMLDocumentHandler for AbstractSAXParserHandler {
    fn start_document(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        namespace_context: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::start_document(locator, encoding, namespace_context, augs)
        )
    }

    fn xml_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        standalone: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::xml_decl(version, encoding, standalone, augs)
        )
    }

    fn doctype_decl(
        &self,
        root_element: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::doctype_decl(root_element, public_id, system_id, augs)
        )
    }

    fn comment(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::comment(text, augs))
    }

    fn processing_instruction(
        &self,
        target: &str,
        data: Option<&XMLString>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::processing_instruction(target, data, augs)
        )
    }

    fn start_element(
        &self,
        element: &mut QName,
        attributes: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::start_element(element, attributes, augs)
        )
    }

    fn empty_element(
        &self,
        element: &mut QName,
        attributes: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::empty_element(element, attributes, augs)
        )
    }

    fn start_general_entity(
        &self,
        name: &str,
        identifier: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDocumentHandler::start_general_entity(name, identifier, encoding, augs)
        )
    }

    fn text_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(self, XMLDocumentHandler::text_decl(version, encoding, augs))
    }

    fn end_general_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::end_general_entity(name, augs))
    }

    fn characters(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::characters(text, augs))
    }

    fn ignorable_whitespace(
        &self,
        text: &XMLString,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(self, XMLDocumentHandler::ignorable_whitespace(text, augs))
    }

    fn end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::end_element(element, augs))
    }

    fn start_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::start_cdata(augs))
    }

    fn end_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::end_cdata(augs))
    }

    fn end_document(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDocumentHandler::end_document(augs))
    }

    fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>) {
        if let Some(p) = self.parser() {
            XMLDocumentHandler::set_document_source(&*p, source);
        }
    }

    fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>> {
        self.parser()
            .and_then(|p| XMLDocumentHandler::get_document_source(&*p))
    }
}

impl XMLDTDHandler for AbstractSAXParserHandler {
    fn start_dtd(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(self, XMLDTDHandler::start_dtd(locator, augmentations))
    }

    fn start_parameter_entity(
        &self,
        name: &str,
        identifier: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::start_parameter_entity(name, identifier, encoding, augmentations)
        )
    }

    fn text_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::text_decl(version, encoding, augmentations)
        )
    }

    fn end_parameter_entity(
        &self,
        name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::end_parameter_entity(name, augmentations)
        )
    }

    fn start_external_subset(
        &self,
        identifier: Option<&dyn XMLResourceIdentifier>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::start_external_subset(identifier, augmentations)
        )
    }

    fn end_external_subset(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDHandler::end_external_subset(augmentations))
    }

    fn comment(&self, text: &XMLString, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDHandler::comment(text, augmentations))
    }

    fn processing_instruction(
        &self,
        target: &str,
        data: Option<&XMLString>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::processing_instruction(target, data, augmentations)
        )
    }

    fn element_decl(
        &self,
        name: &str,
        content_model: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::element_decl(name, content_model, augmentations)
        )
    }

    fn start_attlist(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::start_attlist(element_name, augmentations)
        )
    }

    fn attribute_decl(
        &self,
        element_name: &str,
        attribute_name: &str,
        type_: &str,
        enumeration: Option<&[String]>,
        default_type: Option<&str>,
        default_value: Option<&XMLString>,
        non_normalized_default_value: Option<&XMLString>,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::attribute_decl(
                element_name,
                attribute_name,
                type_,
                enumeration,
                default_type,
                default_value,
                non_normalized_default_value,
                augmentations
            )
        )
    }

    fn end_attlist(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDHandler::end_attlist(augmentations))
    }

    fn internal_entity_decl(
        &self,
        name: &str,
        text: &XMLString,
        non_normalized_text: &XMLString,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::internal_entity_decl(name, text, non_normalized_text, augmentations)
        )
    }

    fn external_entity_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::external_entity_decl(name, identifier, augmentations)
        )
    }

    fn unparsed_entity_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        notation: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::unparsed_entity_decl(name, identifier, notation, augmentations)
        )
    }

    fn notation_decl(
        &self,
        name: &str,
        identifier: &dyn XMLResourceIdentifier,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDHandler::notation_decl(name, identifier, augmentations)
        )
    }

    fn start_conditional(
        &self,
        type_: i16,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(self, XMLDTDHandler::start_conditional(type_, augmentations))
    }

    fn ignored_characters(
        &self,
        text: &XMLString,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(self, XMLDTDHandler::ignored_characters(text, augmentations))
    }

    fn end_conditional(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDHandler::end_conditional(augmentations))
    }

    fn end_dtd(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDHandler::end_dtd(augmentations))
    }

    fn set_dtd_source(&self, source: Option<Rc<dyn XMLDTDSource>>) {
        if let Some(p) = self.parser() {
            XMLDTDHandler::set_dtd_source(&*p, source);
        }
    }

    fn get_dtd_source(&self) -> Option<Rc<dyn XMLDTDSource>> {
        self.parser()
            .and_then(|p| XMLDTDHandler::get_dtd_source(&*p))
    }
}

impl XMLDTDContentModelHandler for AbstractSAXParserHandler {
    fn start_content_model(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDContentModelHandler::start_content_model(element_name, augmentations)
        )
    }

    fn any(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDContentModelHandler::any(augmentations))
    }

    fn empty(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDContentModelHandler::empty(augmentations))
    }

    fn start_group(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDContentModelHandler::start_group(augmentations))
    }

    fn pcdata(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDContentModelHandler::pcdata(augmentations))
    }

    fn element(
        &self,
        element_name: &str,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDContentModelHandler::element(element_name, augmentations)
        )
    }

    fn separator(&self, separator: i16, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(
            self,
            XMLDTDContentModelHandler::separator(separator, augmentations)
        )
    }

    fn occurrence(
        &self,
        occurrence: i16,
        augmentations: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        forward!(
            self,
            XMLDTDContentModelHandler::occurrence(occurrence, augmentations)
        )
    }

    fn end_group(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(self, XMLDTDContentModelHandler::end_group(augmentations))
    }

    fn end_content_model(&self, augmentations: Option<&dyn Augmentations>) -> JResult<()> {
        forward!(
            self,
            XMLDTDContentModelHandler::end_content_model(augmentations)
        )
    }

    fn set_dtd_content_model_source(&self, source: Option<Rc<dyn XMLDTDContentModelSource>>) {
        if let Some(p) = self.parser() {
            XMLDTDContentModelHandler::set_dtd_content_model_source(&*p, source);
        }
    }

    fn get_dtd_content_model_source(&self) -> Option<Rc<dyn XMLDTDContentModelSource>> {
        self.parser()
            .and_then(|p| XMLDTDContentModelHandler::get_dtd_content_model_source(&*p))
    }
}

//
// Classes
//

/// Locator proxy: the SAX `Locator2` view of an XNI `XMLLocator`.
///
/// Equivalent to the nested class `AbstractSAXParser.LocatorProxy`.
pub struct LocatorProxy {
    /// XML locator.
    locator: Rc<dyn XMLLocator>,
}

impl LocatorProxy {
    /// Constructs an XML locator proxy.
    ///
    /// Equivalent to `new LocatorProxy(XMLLocator)`.
    pub fn new(locator: Rc<dyn XMLLocator>) -> Self {
        Self { locator }
    }
}

impl Locator for LocatorProxy {
    fn get_public_id(&self) -> Option<String> {
        self.locator.get_public_id()
    }

    fn get_system_id(&self) -> Option<String> {
        self.locator.get_expanded_system_id()
    }

    fn get_line_number(&self) -> i32 {
        self.locator.get_line_number()
    }

    fn get_column_number(&self) -> i32 {
        self.locator.get_column_number()
    }

    fn as_locator2(&self) -> Option<&dyn Locator2> {
        Some(self)
    }
}

impl Locator2 for LocatorProxy {
    fn get_xml_version(&self) -> Option<String> {
        self.locator.get_xml_version()
    }

    fn get_encoding(&self) -> Option<String> {
        self.locator.get_encoding()
    }
}

/// The SAX `AttributeList`, `Attributes` and `Attributes2` view of the
/// XNI attributes of the element being reported.
///
/// Equivalent to the nested class `AbstractSAXParser.AttributesProxy`; it
/// borrows the attributes for the duration of the callback (see the
/// parser's divergence).
pub struct AttributesProxy<'a> {
    /// XML attributes (`fAttributes`), `None` (Java `null`) until
    /// [`set_attributes`](Self::set_attributes).
    attributes: Option<&'a dyn XMLAttributes>,
}

impl<'a> AttributesProxy<'a> {
    /// Equivalent to `new AttributesProxy()`.
    pub fn new() -> Self {
        Self { attributes: None }
    }

    /// Sets the XML attributes.
    ///
    /// Equivalent to `AttributesProxy.setAttributes(XMLAttributes)`.
    pub fn set_attributes(&mut self, attributes: &'a dyn XMLAttributes) {
        self.attributes = Some(attributes);
    }

    /// `fAttributes`, dereferenced.
    fn attributes(&self) -> &'a dyn XMLAttributes {
        self.attributes
            .expect("java.lang.NullPointerException: fAttributes")
    }
}

impl Default for AttributesProxy<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl AttributesProxy<'_> {
    fn ns_uri(uri: &str) -> Option<&str> {
        if uri.is_empty() {
            None
        } else {
            Some(uri)
        }
    }

    fn declared_at(&self, index: i32) -> bool {
        self.attributes()
            .get_augmentations(index)
            .and_then(|a| a.get_item(constants::ATTRIBUTE_DECLARED))
            .and_then(|o| object_as_bool(&o))
            .unwrap_or(false)
    }
}

impl AttributeList for AttributesProxy<'_> {
    fn get_length(&self) -> i32 {
        self.attributes().get_length()
    }

    fn get_name(&self, i: i32) -> Option<String> {
        self.attributes().get_qname(i)
    }

    fn get_type(&self, i: i32) -> Option<String> {
        self.attributes().get_type(i)
    }

    fn get_value(&self, i: i32) -> Option<String> {
        self.attributes().get_value(i)
    }

    fn get_type_by_name(&self, name: &str) -> Option<String> {
        self.attributes().get_type_by_qname(name)
    }

    fn get_value_by_name(&self, name: &str) -> Option<String> {
        self.attributes().get_value_by_qname(name)
    }
}

impl Attributes for AttributesProxy<'_> {
    fn get_length(&self) -> i32 {
        self.attributes().get_length()
    }

    fn get_uri(&self, index: i32) -> Option<String> {
        // REVISIT: this hides the fact that internally we use
        //          null instead of empty string
        //          SAX requires URI to be a string or an empty string
        Some(self.attributes().get_uri(index).unwrap_or_default())
    }

    fn get_local_name(&self, index: i32) -> Option<String> {
        self.attributes().get_local_name(index)
    }

    fn get_qname(&self, index: i32) -> Option<String> {
        self.attributes().get_qname(index)
    }

    fn get_type(&self, i: i32) -> Option<String> {
        self.attributes().get_type(i)
    }

    fn get_value(&self, i: i32) -> Option<String> {
        self.attributes().get_value(i)
    }

    fn get_index_ns(&self, uri: &str, local_part: &str) -> i32 {
        self.attributes()
            .get_index_ns(Self::ns_uri(uri), local_part)
    }

    fn get_index(&self, qname: &str) -> i32 {
        self.attributes().get_index(qname)
    }

    fn get_type_ns(&self, uri: &str, local_name: &str) -> Option<String> {
        self.attributes().get_type_ns(Self::ns_uri(uri), local_name)
    }

    fn get_type_by_qname(&self, qname: &str) -> Option<String> {
        self.attributes().get_type_by_qname(qname)
    }

    fn get_value_ns(&self, uri: &str, local_name: &str) -> Option<String> {
        self.attributes()
            .get_value_ns(Self::ns_uri(uri), local_name)
    }

    fn get_value_by_qname(&self, qname: &str) -> Option<String> {
        self.attributes().get_value_by_qname(qname)
    }
}

impl Attributes2 for AttributesProxy<'_> {
    fn is_declared(&self, index: i32) -> JResult<bool> {
        if index < 0 || index >= self.attributes().get_length() {
            return Err(Exception::runtime(rucene::error::LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Array index out of range: {index}"),
            )));
        }
        Ok(self.declared_at(index))
    }

    fn is_declared_by_qname(&self, qname: &str) -> JResult<bool> {
        let index = Attributes::get_index(self, qname);
        if index == -1 {
            return Err(Exception::runtime(
                rucene::error::LuceneError::illegal_argument(qname),
            ));
        }
        Ok(self.declared_at(index))
    }

    fn is_declared_ns(&self, uri: &str, local_name: &str) -> JResult<bool> {
        let index = Attributes::get_index_ns(self, uri, local_name);
        if index == -1 {
            return Err(Exception::runtime(
                rucene::error::LuceneError::illegal_argument(local_name),
            ));
        }
        Ok(self.declared_at(index))
    }

    fn is_specified(&self, index: i32) -> JResult<bool> {
        if index < 0 || index >= self.attributes().get_length() {
            return Err(Exception::runtime(rucene::error::LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Array index out of range: {index}"),
            )));
        }
        Ok(self.attributes().is_specified(index))
    }

    fn is_specified_ns(&self, uri: &str, local_name: &str) -> JResult<bool> {
        let index = Attributes::get_index_ns(self, uri, local_name);
        if index == -1 {
            return Err(Exception::runtime(
                rucene::error::LuceneError::illegal_argument(local_name),
            ));
        }
        Ok(self.attributes().is_specified(index))
    }

    fn is_specified_by_qname(&self, qname: &str) -> JResult<bool> {
        let index = Attributes::get_index(self, qname);
        if index == -1 {
            return Err(Exception::runtime(
                rucene::error::LuceneError::illegal_argument(qname),
            ));
        }
        Ok(self.attributes().is_specified(index))
    }
}
