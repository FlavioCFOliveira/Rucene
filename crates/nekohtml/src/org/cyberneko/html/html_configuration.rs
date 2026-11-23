/* 
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 * 
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLConfiguration.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Once;

use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::message_format::MessageFormat;
use rucene_queryparser::jdk::object::JavaObject;
use rucene_queryparser::jdk::resource_bundle::ResourceBundle;

use crate::java::{interface_object, string_object, JResult, Object};
use crate::org::apache::xerces::r#impl::version::F_VERSION;
use crate::org::apache::xerces::util::default_error_handler::DefaultErrorHandler;
use crate::org::apache::xerces::util::parser_configuration_settings::ParserConfigurationSettings;
use crate::org::apache::xerces::util::symbol_table::SymbolTable;
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;
use crate::org::apache::xerces::xni::parser::xml_document_filter::XMLDocumentFilter;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::parser::xml_entity_resolver::XMLEntityResolver;
use crate::org::apache::xerces::xni::parser::xml_error_handler::XMLErrorHandler;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::parser::xml_parse_exception::XMLParseException;
use crate::org::apache::xerces::xni::parser::xml_parser_configuration::XMLParserConfiguration;
use crate::org::apache::xerces::xni::parser::xml_pull_parser_configuration::XMLPullParserConfiguration;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_dtd_content_model_handler::XMLDTDContentModelHandler;
use crate::org::apache::xerces::xni::xml_dtd_handler::XMLDTDHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::cyberneko::html::filters::namespace_binder::NamespaceBinder;
use crate::org::cyberneko::html::html_component::HTMLComponent;
use crate::org::cyberneko::html::html_error_reporter::HTMLErrorReporter;
use crate::org::cyberneko::html::html_scanner::HTMLScanner;
use crate::org::cyberneko::html::html_tag_balancer::HTMLTagBalancer;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge;

// features

/// Namespaces.
pub const NAMESPACES: &str = "http://xml.org/sax/features/namespaces";
/// Include infoset augmentations.
pub const AUGMENTATIONS: &str = "http://cyberneko.org/html/features/augmentations";
/// Report errors.
pub const REPORT_ERRORS: &str = "http://cyberneko.org/html/features/report-errors";
/// Simple report format.
pub const SIMPLE_ERROR_FORMAT: &str = "http://cyberneko.org/html/features/report-errors/simple";
/// Balance tags.
pub const BALANCE_TAGS: &str = "http://cyberneko.org/html/features/balance-tags";

// properties

/// Modify HTML element names: { "upper", "lower", "default" }.
pub const NAMES_ELEMS: &str = "http://cyberneko.org/html/properties/names/elems";
/// Modify HTML attribute names: { "upper", "lower", "default" }.
pub const NAMES_ATTRS: &str = "http://cyberneko.org/html/properties/names/attrs";
/// Pipeline filters (an `Rc<Vec<Rc<dyn XMLDocumentFilter>>>` object).
pub const FILTERS: &str = "http://cyberneko.org/html/properties/filters";
/// Error reporter.
pub const ERROR_REPORTER: &str = "http://cyberneko.org/html/properties/error-reporter";

// other

/// Error domain.
pub const ERROR_DOMAIN: &str = "http://cyberneko.org/html";

/// The `XMLDocumentFilter[]` value of the `FILTERS` property.
pub type Filters = Vec<Rc<dyn XMLDocumentFilter>>;

// HACK: workarounds Xerces 2.0.x problems

/// Parser version is Xerces 2.0.0 (the static initializer reads
/// `org.apache.xerces.impl.Version.fVersion`).
pub fn xerces_2_0_0() -> bool {
    F_VERSION == "Xerces-J 2.0.0"
}

/// Parser version is Xerces 2.0.1.
pub fn xerces_2_0_1() -> bool {
    F_VERSION == "Xerces-J 2.0.1"
}

/// Parser version is XML4J 4.0.x.
pub fn xml4j_4_0_x() -> bool {
    F_VERSION.starts_with("XML4J 4.0.")
}

/// An XNI-based parser configuration that can be used to parse HTML
/// documents. This configuration can be used directly in order to parse
/// HTML documents or can be used in conjunction with any XNI based tools,
/// such as the Xerces2 implementation.
///
/// Equivalent to `org.cyberneko.html.HTMLConfiguration` (a
/// `ParserConfigurationSettings`, held as `base`). Java's static
/// initializer reflects on `org.apache.xerces.impl.Version` through
/// `ObjectFactory`; the port reads [`F_VERSION`] directly (no class
/// loading in Rust), with the same outcome: against Xerces 2.12.2 none of
/// the Xerces 2.0.x hacks applies.
pub struct HTMLConfiguration {
    /// The `ParserConfigurationSettings` this class extends.
    pub base: ParserConfigurationSettings,
    #[allow(dead_code)]
    self_ref: Weak<HTMLConfiguration>,

    // handlers
    /// Document handler.
    document_handler: RefCell<Option<Rc<dyn XMLDocumentHandler>>>,
    /// DTD handler.
    dtd_handler: RefCell<Option<Rc<dyn XMLDTDHandler>>>,
    /// DTD content model handler.
    dtd_content_model_handler: RefCell<Option<Rc<dyn XMLDTDContentModelHandler>>>,
    /// Error handler.
    error_handler: RefCell<Option<Rc<dyn XMLErrorHandler>>>,

    // other settings
    /// Entity resolver.
    entity_resolver: RefCell<Option<Rc<dyn XMLEntityResolver>>>,
    /// Locale.
    locale: RefCell<Locale>,

    // state
    /// Stream opened by parser. Therefore, must close stream manually upon
    /// termination of parsing.
    close_stream: Cell<bool>,

    // components
    /// Components.
    html_components: RefCell<Vec<Rc<dyn HTMLComponent>>>,

    // pipeline
    /// Document scanner.
    document_scanner: Rc<HTMLScanner>,
    /// HTML tag balancer.
    tag_balancer: Rc<HTMLTagBalancer>,
    /// Namespace binder.
    namespace_binder: Rc<NamespaceBinder>,

    // other components
    /// Error reporter.
    error_reporter: Rc<ErrorReporter>,
}

impl std::fmt::Debug for HTMLConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HTMLConfiguration").field("base", &self.base).finish_non_exhaustive()
    }
}

fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

impl HTMLConfiguration {
    /// Default constructor.
    ///
    /// Equivalent to `new HTMLConfiguration()`.
    pub fn new() -> Rc<Self> {
        Self::with_document_scanner(HTMLScanner::new())
    }

    /// The constructor of a subclass whose `createDocumentScanner()`
    /// returns `document_scanner`.
    ///
    /// Equivalent to `new HTMLConfiguration()` with an overridden protected
    /// `createDocumentScanner()`.
    pub fn with_document_scanner(document_scanner: Rc<HTMLScanner>) -> Rc<Self> {
        let config = Rc::new_cyclic(|weak: &Weak<HTMLConfiguration>| Self {
            base: ParserConfigurationSettings::new(),
            self_ref: weak.clone(),
            document_handler: RefCell::new(None),
            dtd_handler: RefCell::new(None),
            dtd_content_model_handler: RefCell::new(None),
            error_handler: RefCell::new(Some(Rc::new(DefaultErrorHandler::new()) as Rc<dyn XMLErrorHandler>)),
            entity_resolver: RefCell::new(None),
            locale: RefCell::new(Locale::get_default()),
            close_stream: Cell::new(false),
            html_components: RefCell::new(Vec::with_capacity(2)),
            document_scanner,
            tag_balancer: HTMLTagBalancer::new(),
            namespace_binder: Rc::new(NamespaceBinder::new()),
            error_reporter: Rc::new(ErrorReporter {
                configuration: weak.clone(),
                last_locale: RefCell::new(None),
                error_messages: RefCell::new(None),
            }),
        });
        config.init().expect("INVARIANT: the configuration recognizes its own features and properties");
        config
    }

    /// The body of the Java constructor.
    fn init(&self) -> JResult<()> {
        // add components
        self.add_component(Rc::clone(&self.document_scanner) as Rc<dyn HTMLComponent>)?;
        self.add_component(Rc::clone(&self.tag_balancer) as Rc<dyn HTMLComponent>)?;
        self.add_component(Rc::clone(&self.namespace_binder) as Rc<dyn HTMLComponent>)?;

        //
        // features
        //

        // recognized features
        let validation = "http://xml.org/sax/features/validation";
        let recognized_features = strings(&[
            AUGMENTATIONS,
            NAMESPACES,
            validation,
            REPORT_ERRORS,
            SIMPLE_ERROR_FORMAT,
            BALANCE_TAGS,
        ]);
        self.add_recognized_features(Some(&recognized_features));
        self.set_feature(AUGMENTATIONS, false)?;
        self.set_feature(NAMESPACES, true)?;
        self.set_feature(validation, false)?;
        self.set_feature(REPORT_ERRORS, false)?;
        self.set_feature(SIMPLE_ERROR_FORMAT, false)?;
        self.set_feature(BALANCE_TAGS, true)?;

        // HACK: Xerces 2.0.0
        if xerces_2_0_0() {
            // NOTE: These features should not be required but it causes a
            //       problem if they're not there. This will be fixed in
            //       subsequent releases of Xerces.
            let recognized_features = strings(&["http://apache.org/xml/features/scanner/notify-builtin-refs"]);
            self.add_recognized_features(Some(&recognized_features));
        }

        // HACK: Xerces 2.0.1
        if xerces_2_0_0() || xerces_2_0_1() || xml4j_4_0_x() {
            // NOTE: These features should not be required but it causes a
            //       problem if they're not there. This should be fixed in
            //       subsequent releases of Xerces.
            let recognized_features = strings(&[
                "http://apache.org/xml/features/validation/schema/normalized-value",
                "http://apache.org/xml/features/scanner/notify-char-refs",
            ]);
            self.add_recognized_features(Some(&recognized_features));
        }

        //
        // properties
        //

        // recognized properties
        let recognized_properties = strings(&[NAMES_ELEMS, NAMES_ATTRS, FILTERS, ERROR_REPORTER]);
        self.add_recognized_properties(Some(&recognized_properties));
        self.set_property(NAMES_ELEMS, Some(string_object("upper")))?;
        self.set_property(NAMES_ATTRS, Some(string_object("lower")))?;
        let reporter: Rc<dyn HTMLErrorReporter> = Rc::clone(&self.error_reporter) as Rc<dyn HTMLErrorReporter>;
        self.set_property(ERROR_REPORTER, Some(interface_object(reporter)))?;

        // HACK: Xerces 2.0.0
        if xerces_2_0_0() {
            // NOTE: This is a hack to get around a problem in the Xerces
            //       2.0.0 AbstractSAXParser. If it uses a parser
            //       configuration that does not have a SymbolTable, then it
            //       will remove *all* attributes. This will be fixed in
            //       subsequent releases of Xerces.
            let symbol_table = "http://apache.org/xml/properties/internal/symbol-table";
            let recognized_properties = strings(&[symbol_table]);
            self.add_recognized_properties(Some(&recognized_properties));
            self.set_property(symbol_table, Some(Rc::new(SymbolTable::new()) as Object))?;
        }
        Ok(())
    }

    /// Returns the document scanner (`fDocumentScanner`).
    pub fn document_scanner(&self) -> &Rc<HTMLScanner> {
        &self.document_scanner
    }

    //
    // Public methods
    //

    /// Pushes an input source onto the current entity stack. This enables
    /// the scanner to transparently scan new content (e.g. the output
    /// written by an embedded script). At the end of the current entity,
    /// the scanner returns where it left off at the time this entity
    /// source was pushed.
    ///
    /// Equivalent to `pushInputSource(XMLInputSource)`.
    ///
    /// # Errors
    ///
    /// The charset errors of a byte stream's reader.
    pub fn push_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        self.document_scanner.push_input_source(input_source)
    }

    /// **EXPERIMENTAL: may change in next release**
    /// Immediately evaluates an input source and add the new content (e.g.
    /// the output written by an embedded script).
    ///
    /// Equivalent to `evaluateInputSource(XMLInputSource)`.
    ///
    /// # Errors
    ///
    /// The XNI exceptions of the handlers.
    pub fn evaluate_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        self.document_scanner.evaluate_input_source(input_source)
    }

    //
    // Protected methods
    //

    /// Adds a component.
    ///
    /// Equivalent to the protected `addComponent(HTMLComponent)`.
    ///
    /// # Errors
    ///
    /// The `XMLConfigurationException`s of the defaults set.
    pub fn add_component(&self, component: Rc<dyn HTMLComponent>) -> JResult<()> {
        // add component to list
        self.html_components.borrow_mut().push(Rc::clone(&component));

        // add recognized features and set default states
        let features = component.get_recognized_features();
        self.add_recognized_features(features.as_deref());
        for feature in features.as_deref().unwrap_or(&[]) {
            if let Some(state) = component.get_feature_default(feature) {
                self.set_feature(feature, state)?;
            }
        }

        // add recognized properties and set default values
        let properties = component.get_recognized_properties();
        self.add_recognized_properties(properties.as_deref());
        for property in properties.as_deref().unwrap_or(&[]) {
            if let Some(value) = component.get_property_default(property) {
                self.set_property(property, Some(value))?;
            }
        }
        Ok(())
    }

    /// Resets the parser configuration.
    ///
    /// Equivalent to the protected `reset()`.
    ///
    /// # Errors
    ///
    /// The `XMLConfigurationException`s of the components.
    pub fn reset(&self) -> JResult<()> {
        // reset components
        let components = self.html_components.borrow().clone();
        for component in &components {
            component.reset(self)?;
        }

        // configure pipeline
        let mut last_source: Rc<dyn XMLDocumentSource> = Rc::clone(&self.document_scanner) as Rc<dyn XMLDocumentSource>;
        if self.get_feature(NAMESPACES)? {
            last_source.set_document_handler(Some(Rc::clone(&self.namespace_binder) as Rc<dyn XMLDocumentHandler>));
            XMLDocumentHandler::set_document_source(
                &*self.namespace_binder,
                Some(Rc::clone(&self.tag_balancer) as Rc<dyn XMLDocumentSource>),
            );
            last_source = Rc::clone(&self.namespace_binder) as Rc<dyn XMLDocumentSource>;
        }
        if self.get_feature(BALANCE_TAGS)? {
            last_source.set_document_handler(Some(Rc::clone(&self.tag_balancer) as Rc<dyn XMLDocumentHandler>));
            XMLDocumentHandler::set_document_source(
                &*self.tag_balancer,
                Some(Rc::clone(&self.document_scanner) as Rc<dyn XMLDocumentSource>),
            );
            last_source = Rc::clone(&self.tag_balancer) as Rc<dyn XMLDocumentSource>;
        }
        let filters = self.get_property(FILTERS)?;
        if let Some(filters) = filters.and_then(|f| f.downcast::<Filters>().ok()) {
            for filter in filters.iter() {
                xerces_bridge::get_instance().xml_document_filter_set_document_source(&**filter, Some(Rc::clone(&last_source)));
                last_source.set_document_handler(Some(Rc::clone(filter).as_document_handler()));
                last_source = Rc::clone(filter).as_document_source();
            }
        }
        last_source.set_document_handler(self.document_handler.borrow().clone());
        Ok(())
    }
}

impl XMLComponentManager for HTMLConfiguration {
    fn get_feature(&self, feature_id: &str) -> JResult<bool> {
        self.base.get_feature(feature_id)
    }

    fn get_property(&self, property_id: &str) -> JResult<Option<Object>> {
        self.base.get_property(property_id)
    }
}

impl XMLParserConfiguration for HTMLConfiguration {
    /// Parses a document.
    fn parse(&self, source: &XMLInputSource) -> JResult<()> {
        self.set_input_source(source)?;
        self.parse_complete(true)?;
        Ok(())
    }

    fn add_recognized_features(&self, feature_ids: Option<&[String]>) {
        self.base.add_recognized_features(feature_ids);
    }

    /// Sets a feature.
    fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()> {
        self.base.set_feature(feature_id, state)?;
        let components = self.html_components.borrow().clone();
        for component in &components {
            component.set_feature(feature_id, state)?;
        }
        Ok(())
    }

    fn add_recognized_properties(&self, property_ids: Option<&[String]>) {
        self.base.add_recognized_properties(property_ids);
    }

    /// Sets a property.
    fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        self.base.set_property(property_id, value.clone())?;

        if property_id == FILTERS {
            let filters = self.get_property(FILTERS)?;
            if let Some(filters) = filters.and_then(|f| f.downcast::<Filters>().ok()) {
                for filter in filters.iter() {
                    if let Some(component) = Rc::clone(filter).as_html_component() {
                        self.add_component(component)?;
                    }
                }
            }
        }

        let components = self.html_components.borrow().clone();
        for component in &components {
            component.set_property(property_id, value.clone())?;
        }
        Ok(())
    }

    /// Sets the document handler.
    fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>) {
        *self.document_handler.borrow_mut() = handler.clone();
        if let Some(listener) = handler.and_then(|h| h.as_tag_balancing_listener()) {
            self.tag_balancer.set_tag_balancing_listener(Some(listener));
        }
    }

    /// Returns the document handler.
    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }

    /// Sets the DTD handler.
    fn set_dtd_handler(&self, handler: Option<Rc<dyn XMLDTDHandler>>) {
        *self.dtd_handler.borrow_mut() = handler;
    }

    /// Returns the DTD handler.
    fn get_dtd_handler(&self) -> Option<Rc<dyn XMLDTDHandler>> {
        self.dtd_handler.borrow().clone()
    }

    /// Sets the DTD content model handler.
    fn set_dtd_content_model_handler(&self, handler: Option<Rc<dyn XMLDTDContentModelHandler>>) {
        *self.dtd_content_model_handler.borrow_mut() = handler;
    }

    /// Returns the DTD content model handler.
    fn get_dtd_content_model_handler(&self) -> Option<Rc<dyn XMLDTDContentModelHandler>> {
        self.dtd_content_model_handler.borrow().clone()
    }

    /// Sets the error handler.
    fn set_error_handler(&self, handler: Option<Rc<dyn XMLErrorHandler>>) {
        *self.error_handler.borrow_mut() = handler;
    }

    /// Returns the error handler.
    fn get_error_handler(&self) -> Option<Rc<dyn XMLErrorHandler>> {
        self.error_handler.borrow().clone()
    }

    /// Sets the entity resolver.
    fn set_entity_resolver(&self, resolver: Option<Rc<dyn XMLEntityResolver>>) {
        *self.entity_resolver.borrow_mut() = resolver;
    }

    /// Returns the entity resolver.
    fn get_entity_resolver(&self) -> Option<Rc<dyn XMLEntityResolver>> {
        self.entity_resolver.borrow().clone()
    }

    /// Sets the locale.
    fn set_locale(&self, locale: Option<Locale>) -> JResult<()> {
        *self.locale.borrow_mut() = locale.unwrap_or_else(Locale::get_default);
        Ok(())
    }

    /// Returns the locale.
    fn get_locale(&self) -> Locale {
        self.locale.borrow().clone()
    }

    fn as_pull_parser_configuration(&self) -> Option<&dyn XMLPullParserConfiguration> {
        Some(self)
    }
}

//
// XMLPullParserConfiguration methods
//

impl XMLPullParserConfiguration for HTMLConfiguration {
    /// Sets the input source for the document to parse.
    fn set_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        self.reset()?;
        self.close_stream
            .set(input_source.get_byte_stream().is_none() && input_source.get_character_stream().is_none());
        self.document_scanner.set_input_source(input_source)
    }

    /// Parses the document in a pull parsing fashion.
    fn parse_complete(&self, complete: bool) -> JResult<bool> {
        match self.document_scanner.scan_document(complete) {
            Ok(more) => {
                if !more {
                    self.cleanup();
                }
                Ok(more)
            }
            Err(e) => {
                // XNIException and IOException: clean up, then rethrow
                self.cleanup();
                Err(e)
            }
        }
    }

    /// If the application decides to terminate parsing before the xml
    /// document is fully parsed, the application should call this method
    /// to free any resource allocated during parsing.
    fn cleanup(&self) {
        self.document_scanner.cleanup(self.close_stream.get());
    }
}

/// Defines an error reporter for reporting HTML errors. There is no such
/// thing as a fatal error in parsing HTML. I/O errors are fatal but should
/// throw an `IOException` directly instead of reporting an error.
///
/// Equivalent to the protected inner class
/// `HTMLConfiguration.ErrorReporter`; it reaches its configuration weakly
/// (see the crate documentation on reference cycles).
pub struct ErrorReporter {
    configuration: Weak<HTMLConfiguration>,
    /// Last locale.
    last_locale: RefCell<Option<Locale>>,
    /// Error messages.
    error_messages: RefCell<Option<std::sync::Arc<ResourceBundle>>>,
}

impl std::fmt::Debug for ErrorReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErrorReporter")
    }
}

const ERROR_MESSAGES: &str = "org/cyberneko/html/res/ErrorMessages";

fn register_error_messages() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        ResourceBundle::register_properties(ERROR_MESSAGES, include_bytes!("res/ErrorMessages.properties"));
        ResourceBundle::register_properties(
            "org/cyberneko/html/res/ErrorMessages_ja",
            include_bytes!("res/ErrorMessages_ja.properties"),
        );
    });
}

impl ErrorReporter {
    fn config(&self) -> Rc<HTMLConfiguration> {
        self.configuration
            .upgrade()
            .expect("INVARIANT: the reporter lives in its configuration")
    }

    /// Creates parse exception.
    ///
    /// Equivalent to the protected `createException(String, Object[])`.
    pub fn create_exception(&self, key: &str, args: Option<&[JavaObject]>) -> XMLParseException {
        let message = self.format_message(key, args);
        let config = self.config();
        let locator: &dyn XMLLocator = &**config.document_scanner();
        XMLParseException::new(Some(locator), Some(&message))
    }

    /// Format simple message.
    ///
    /// Equivalent to the protected `formatSimpleMessage(String, Object[])`.
    pub fn format_simple_message(&self, key: &str, args: Option<&[JavaObject]>) -> String {
        let mut str = String::new();
        str.push_str(ERROR_DOMAIN);
        str.push('#');
        str.push_str(key);
        if let Some(args) = args.filter(|a| !a.is_empty()) {
            str.push('\t');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    str.push('\t');
                }
                str.push_str(&a.to_java_string());
            }
        }
        str
    }

    fn error_handler(&self) -> Option<Rc<dyn XMLErrorHandler>> {
        self.config().error_handler.borrow().clone()
    }
}

impl HTMLErrorReporter for ErrorReporter {
    /// Format message without reporting error.
    fn format_message(&self, key: &str, args: Option<&[JavaObject]>) -> String {
        let config = self.config();
        if !config.get_feature(SIMPLE_ERROR_FORMAT).unwrap_or(false) {
            let locale = config.locale.borrow().clone();
            if self.last_locale.borrow().as_ref() != Some(&locale) {
                *self.error_messages.borrow_mut() = None;
                *self.last_locale.borrow_mut() = Some(locale.clone());
            }
            if self.error_messages.borrow().is_none() {
                register_error_messages();
                // MissingResourceException is an unchecked exception that
                // Java does not catch here; the bundle is always present
                *self.error_messages.borrow_mut() = ResourceBundle::get_bundle(ERROR_MESSAGES, &locale).ok();
            }
            let bundle = self.error_messages.borrow().clone();
            if let Some(bundle) = bundle {
                // MissingResourceException: ignore and return a simple
                // format
                if let Ok(value) = bundle.get_string(Some(key)) {
                    if let Ok(message) =
                        MessageFormat::new(&value, &Locale::get_default()).and_then(|f| f.format(args))
                    {
                        return message;
                    }
                }
            }
        }
        self.format_simple_message(key, args)
    }

    /// Reports a warning.
    fn report_warning(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()> {
        if let Some(h) = self.error_handler() {
            h.warning(ERROR_DOMAIN, key, &self.create_exception(key, args))?;
        }
        Ok(())
    }

    /// Reports an error.
    fn report_error(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()> {
        if let Some(h) = self.error_handler() {
            h.error(ERROR_DOMAIN, key, &self.create_exception(key, args))?;
        }
        Ok(())
    }
}
