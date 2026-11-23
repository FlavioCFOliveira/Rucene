/*
 * Copyright 2004-2008 Andy Clark
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

//! Port of `NamespaceBinder.java`, of the Java package `org.cyberneko.html.filters` (NekoHTML 1.9.17).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::java::{object_as_str, string_object, to_lower_case, to_upper_case, JResult, Object};
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::{self, NamespaceContext};
use crate::org::apache::xerces::xni::parser::xml_component::XMLComponent;
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;
use crate::org::apache::xerces::xni::parser::xml_document_filter::XMLDocumentFilter;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xml_string::XMLString;
use crate::org::cyberneko::html::filters::default_filter::{merge, DefaultFilter};
use crate::org::cyberneko::html::html_component::HTMLComponent;
use crate::org::cyberneko::html::html_elements;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge;

/// XHTML 1.0 namespace URI (http://www.w3.org/1999/xhtml).
pub const XHTML_1_0_URI: &str = "http://www.w3.org/1999/xhtml";
/// XML namespace URI (http://www.w3.org/XML/1998/namespace).
pub const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";
/// XMLNS namespace URI (http://www.w3.org/2000/xmlns/).
pub const XMLNS_URI: &str = "http://www.w3.org/2000/xmlns/";
/// Namespaces.
pub const NAMESPACES: &str = "http://xml.org/sax/features/namespaces";
/// Override namespace binding URI.
pub const OVERRIDE_NAMESPACES: &str = "http://cyberneko.org/html/features/override-namespaces";
/// Insert namespace binding URIs.
pub const INSERT_NAMESPACES: &str = "http://cyberneko.org/html/features/insert-namespaces";
/// Recognized features.
const RECOGNIZED_FEATURES: [&str; 3] = [NAMESPACES, OVERRIDE_NAMESPACES, INSERT_NAMESPACES];
/// Feature defaults.
const FEATURE_DEFAULTS: [Option<bool>; 3] = [None, Some(false), Some(false)];
/// Modify HTML element names: { "upper", "lower", "default" }.
pub const NAMES_ELEMS: &str = "http://cyberneko.org/html/properties/names/elems";
/// Modify HTML attribute names: { "upper", "lower", "default" }.
pub const NAMES_ATTRS: &str = "http://cyberneko.org/html/properties/names/attrs";
/// Namespace URI.
pub const NAMESPACES_URI: &str = "http://cyberneko.org/html/properties/namespaces-uri";
/// Recognized properties.
const RECOGNIZED_PROPERTIES: [&str; 3] = [NAMES_ELEMS, NAMES_ATTRS, NAMESPACES_URI];
/// Don't modify HTML names.
pub const NAMES_NO_CHANGE: i16 = 0;
/// Uppercase HTML names.
pub const NAMES_UPPERCASE: i16 = 1;
/// Lowercase HTML names.
pub const NAMES_LOWERCASE: i16 = 2;

/// This filter binds namespaces if namespace processing is turned on by
/// setting the feature "http://xml.org/sax/features/namespaces" is set to
/// `true`.
///
/// Equivalent to `org.cyberneko.html.filters.NamespaceBinder` (a
/// `DefaultFilter`, held as `base`).
pub struct NamespaceBinder {
    /// The `DefaultFilter` this class extends.
    pub base: DefaultFilter,
    /// Namespaces.
    namespaces: Cell<bool>,
    /// Override namespaces.
    override_namespaces: Cell<bool>,
    /// Insert namespaces.
    insert_namespaces: Cell<bool>,
    /// Modify HTML element names.
    names_elems: Cell<i16>,
    /// Modify HTML attribute names.
    names_attrs: Cell<i16>,
    /// Namespaces URI.
    namespaces_uri: RefCell<String>,
    /// Namespace context.
    namespace_context: Rc<NamespaceSupport>,
    /// QName.
    qname: RefCell<QName>,
}

impl std::fmt::Debug for NamespaceBinder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamespaceBinder")
            .field("namespaces", &self.namespaces.get())
            .finish_non_exhaustive()
    }
}

impl Default for NamespaceBinder {
    fn default() -> Self {
        Self::new()
    }
}

/// Splits a qualified name.
///
/// Equivalent to the protected static `splitQName(QName)`.
pub fn split_qname(qname: &mut QName) {
    let rawname = qname.rawname.clone().unwrap_or_default();
    if let Some(index) = rawname.find(':') {
        qname.prefix = Some(rawname[..index].to_string());
        qname.localpart = Some(rawname[index + 1..].to_string());
    }
}

/// Converts HTML names string value to constant value.
///
/// Equivalent to the protected static `getNamesValue(String)`.
pub fn get_names_value(value: &str) -> i16 {
    if value == "lower" {
        return NAMES_LOWERCASE;
    }
    if value == "upper" {
        return NAMES_UPPERCASE;
    }
    NAMES_NO_CHANGE
}

/// Modifies the given name based on the specified mode.
///
/// Equivalent to the protected static `modifyName(String, short)`.
pub fn modify_name(name: &str, mode: i16) -> String {
    match mode {
        NAMES_UPPERCASE => to_upper_case(name),
        NAMES_LOWERCASE => to_lower_case(name),
        _ => name.to_string(),
    }
}

impl NamespaceBinder {
    /// Equivalent to `new NamespaceBinder()`.
    pub fn new() -> Self {
        Self {
            base: DefaultFilter::new(),
            namespaces: Cell::new(false),
            override_namespaces: Cell::new(false),
            insert_namespaces: Cell::new(false),
            names_elems: Cell::new(NAMES_NO_CHANGE),
            names_attrs: Cell::new(NAMES_NO_CHANGE),
            namespaces_uri: RefCell::new(String::new()),
            namespace_context: Rc::new(NamespaceSupport::new()),
            qname: RefCell::new(QName::new()),
        }
    }

    fn start_prefix_mappings(&self, augs: Option<&dyn Augmentations>) {
        let dcount = self.namespace_context.get_declared_prefix_count();
        if let Some(h) = self.base.next_handler() {
            if dcount > 0 {
                for i in 0..dcount {
                    let prefix = self.namespace_context.get_declared_prefix_at(i);
                    let uri = self.namespace_context.get_uri(&prefix);
                    xerces_bridge::get_instance().xml_document_handler_start_prefix_mapping(
                        &*h,
                        &prefix,
                        uri.as_deref(),
                        augs,
                    );
                }
            }
        }
    }

    fn end_prefix_mappings(&self, augs: Option<&dyn Augmentations>) {
        let dcount = self.namespace_context.get_declared_prefix_count();
        if let Some(h) = self.base.next_handler() {
            if dcount > 0 {
                for i in (0..dcount).rev() {
                    let prefix = self.namespace_context.get_declared_prefix_at(i);
                    xerces_bridge::get_instance()
                        .xml_document_handler_end_prefix_mapping(&*h, &prefix, augs);
                }
            }
        }
    }

    /// Binds namespaces.
    ///
    /// Equivalent to the protected `bindNamespaces(QName, XMLAttributes)`.
    pub fn bind_namespaces(&self, element: &mut QName, mut attrs: Option<&mut dyn XMLAttributes>) {
        // split element qname
        split_qname(element);

        // declare namespace prefixes
        let attr_count = attrs.as_ref().map_or(0, |a| a.get_length());
        if let Some(attrs) = attrs.as_deref_mut() {
            for i in (0..attr_count).rev() {
                let mut q = self.qname.borrow_mut();
                attrs.get_name(i, &mut q);
                let mut aname = q.rawname.clone().unwrap_or_default();
                let aname_upper = to_upper_case(&aname);
                if aname_upper.starts_with("XMLNS:") || aname_upper == "XMLNS" {
                    let units: Vec<u16> = aname.encode_utf16().collect();
                    let anamelen = units.len();
                    let mut aprefix = if anamelen > 5 {
                        Some(String::from_utf16_lossy(&units[..5]))
                    } else {
                        None
                    };
                    let mut alocal = if anamelen > 5 {
                        String::from_utf16_lossy(&units[6..])
                    } else {
                        aname.clone()
                    };
                    let avalue = attrs.get_value(i).unwrap_or_default();
                    if anamelen > 5 {
                        aprefix = Some(modify_name(
                            aprefix.as_deref().unwrap_or(""),
                            NAMES_LOWERCASE,
                        ));
                        alocal = modify_name(&alocal, self.names_elems.get());
                        aname = format!("{}:{}", aprefix.as_deref().unwrap_or(""), alocal);
                    } else {
                        alocal = modify_name(&alocal, NAMES_LOWERCASE);
                        aname = alocal.clone();
                    }
                    q.set_values(aprefix.as_deref(), Some(&alocal), Some(&aname), None);
                    attrs.set_name(i, &q);

                    // declare prefix
                    let prefix = if alocal != aname {
                        alocal.clone()
                    } else {
                        String::new()
                    };
                    let mut uri = if !avalue.is_empty() {
                        Some(avalue.clone())
                    } else {
                        None
                    };
                    if self.override_namespaces.get()
                        && Some(prefix.as_str()) == element.prefix.as_deref()
                        && element.localpart.as_deref().is_some_and(|l| {
                            !std::ptr::eq(
                                html_elements::get_element_or(l, html_elements::no_such_element()),
                                html_elements::no_such_element(),
                            )
                        })
                    {
                        uri = Some(self.namespaces_uri.borrow().clone());
                    }
                    self.namespace_context
                        .declare_prefix(&prefix, uri.as_deref());
                }
            }
        }

        // bind element
        let prefix = element.prefix.clone().unwrap_or_default();
        element.uri = self.namespace_context.get_uri(&prefix);
        // REVISIT: The prefix of a qualified element name that is bound to
        //          a namespace is passed (as recent as Xerces 2.4.0) as ""
        //          for start elements and null for end elements. Why? One
        //          of them is a bug, clearly. -Ac
        if element.uri.is_some() && element.prefix.is_none() {
            element.prefix = Some(String::new());
        }

        // do we need to insert namespace bindings?
        if self.insert_namespaces.get() && attrs.is_some() {
            let known = element.localpart.as_deref().is_some_and(|l| {
                !std::ptr::eq(
                    html_elements::get_element_or(l, html_elements::no_such_element()),
                    html_elements::no_such_element(),
                )
            });
            if known
                && (element.prefix.is_none()
                    || self
                        .namespace_context
                        .get_uri(element.prefix.as_deref().unwrap_or(""))
                        .is_none())
            {
                let xmlns = format!(
                    "xmlns{}",
                    element
                        .prefix
                        .as_ref()
                        .map(|p| format!(":{p}"))
                        .unwrap_or_default()
                );
                {
                    let mut q = self.qname.borrow_mut();
                    q.set_values(None, Some(&xmlns), Some(&xmlns), None);
                    let uri = self.namespaces_uri.borrow().clone();
                    attrs
                        .as_deref_mut()
                        .expect("INVARIANT: checked above")
                        .add_attribute(&q, "CDATA", Some(&uri));
                }
                self.bind_namespaces(element, attrs);
                return;
            }
        }

        // bind attributes
        if let Some(attrs) = attrs {
            let attr_count = attrs.get_length();
            for i in 0..attr_count {
                let mut q = self.qname.borrow_mut();
                attrs.get_name(i, &mut q);
                split_qname(&mut q);
                let prefix = if q.rawname.as_deref() != Some("xmlns") {
                    q.prefix.clone().unwrap_or_default()
                } else {
                    "xmlns".to_string()
                };
                // PATCH: Joseph Walton
                if !prefix.is_empty() {
                    q.uri = if prefix == "xml" {
                        Some(XML_URI.to_string())
                    } else {
                        self.namespace_context.get_uri(&prefix)
                    };
                }
                // NOTE: You would think the xmlns namespace would be handled
                //       by NamespaceSupport but it's not. -Ac
                if prefix == "xmlns" && q.uri.is_none() {
                    q.uri = Some(XMLNS_URI.to_string());
                }
                attrs.set_name(i, &q);
            }
        }
    }
}

impl XMLComponent for NamespaceBinder {
    /// Returns a list of feature identifiers that are recognized by this
    /// component.
    fn get_recognized_features(&self) -> Option<Vec<String>> {
        merge(
            self.base.get_recognized_features(),
            Some(RECOGNIZED_FEATURES.iter().map(|s| s.to_string()).collect()),
        )
    }

    /// Returns the default state for a feature.
    fn get_feature_default(&self, feature_id: &str) -> Option<bool> {
        for (i, f) in RECOGNIZED_FEATURES.iter().enumerate() {
            if *f == feature_id {
                return FEATURE_DEFAULTS[i];
            }
        }
        self.base.get_feature_default(feature_id)
    }

    /// Returns a list of property identifiers that are recognized by this
    /// component.
    fn get_recognized_properties(&self) -> Option<Vec<String>> {
        merge(
            self.base.get_recognized_properties(),
            Some(
                RECOGNIZED_PROPERTIES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
        )
    }

    /// Returns the default value for a property.
    fn get_property_default(&self, property_id: &str) -> Option<Object> {
        for (i, p) in RECOGNIZED_PROPERTIES.iter().enumerate() {
            if *p == property_id {
                return if i == 2 {
                    Some(string_object(XHTML_1_0_URI))
                } else {
                    None
                };
            }
        }
        self.base.get_property_default(property_id)
    }

    /// Resets the component.
    fn reset(&self, manager: &dyn XMLComponentManager) -> JResult<()> {
        self.base.reset(manager)?;

        // features
        self.namespaces.set(manager.get_feature(NAMESPACES)?);
        self.override_namespaces
            .set(manager.get_feature(OVERRIDE_NAMESPACES)?);
        self.insert_namespaces
            .set(manager.get_feature(INSERT_NAMESPACES)?);

        // properties
        let value_of = |o: Option<Object>| -> String {
            o.as_ref()
                .and_then(object_as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "null".to_string())
        };
        self.names_elems.set(get_names_value(&value_of(
            manager.get_property(NAMES_ELEMS)?,
        )));
        self.names_attrs.set(get_names_value(&value_of(
            manager.get_property(NAMES_ATTRS)?,
        )));
        *self.namespaces_uri.borrow_mut() = value_of(manager.get_property(NAMESPACES_URI)?);

        // initialize state
        self.namespace_context.reset();
        Ok(())
    }

    fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()> {
        self.base.set_feature(feature_id, state)
    }

    fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        self.base.set_property(property_id, value)
    }
}

impl HTMLComponent for NamespaceBinder {}

impl XMLDocumentSource for NamespaceBinder {
    fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>) {
        self.base.set_document_handler(handler);
    }

    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.base.get_document_handler()
    }
}

impl XMLDocumentHandler for NamespaceBinder {
    /// Start document.
    fn start_document(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        _nscontext: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        // perform default handling
        // NOTE: using own namespace context
        let nc: Rc<dyn NamespaceContext> =
            Rc::clone(&self.namespace_context) as Rc<dyn NamespaceContext>;
        self.base.start_document(locator, encoding, Some(nc), augs)
    }

    fn xml_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        standalone: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.xml_decl(version, encoding, standalone, augs)
    }

    fn doctype_decl(
        &self,
        root: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.doctype_decl(root, public_id, system_id, augs)
    }

    fn comment(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.comment(text, augs)
    }

    fn processing_instruction(
        &self,
        target: &str,
        data: Option<&XMLString>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.processing_instruction(target, data, augs)
    }

    /// Start element.
    fn start_element(
        &self,
        element: &mut QName,
        attrs: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        // bind namespaces, if needed
        if self.namespaces.get() {
            self.namespace_context.push_context();
            self.bind_namespaces(element, Some(&mut *attrs));
            self.start_prefix_mappings(augs);
        }

        // perform default handling
        self.base.start_element(element, attrs, augs)
    }

    /// Empty element.
    fn empty_element(
        &self,
        element: &mut QName,
        attrs: &mut dyn XMLAttributes,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        // bind namespaces, if needed
        if self.namespaces.get() {
            self.namespace_context.push_context();
            self.bind_namespaces(element, Some(&mut *attrs));
            self.start_prefix_mappings(augs);
        }

        // perform default handling
        self.base.empty_element(element, attrs, augs)?;

        // pop context
        if self.namespaces.get() {
            self.end_prefix_mappings(augs);
            self.namespace_context.pop_context();
        }
        Ok(())
    }

    fn start_general_entity(
        &self,
        name: &str,
        id: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.start_general_entity(name, id, encoding, augs)
    }

    fn text_decl(
        &self,
        version: Option<&str>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.text_decl(version, encoding, augs)
    }

    fn end_general_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_general_entity(name, augs)
    }

    fn characters(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.characters(text, augs)
    }

    fn ignorable_whitespace(
        &self,
        text: &XMLString,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.base.ignorable_whitespace(text, augs)
    }

    /// End element.
    fn end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        // bind namespaces, if needed
        if self.namespaces.get() {
            self.bind_namespaces(element, None);
        }

        // perform default handling
        self.base.end_element(element, augs)?;

        // pop context
        if self.namespaces.get() {
            self.end_prefix_mappings(augs);
            self.namespace_context.pop_context();
        }
        Ok(())
    }

    fn start_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.start_cdata(augs)
    }

    fn end_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_cdata(augs)
    }

    fn end_document(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.base.end_document(augs)
    }

    fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>) {
        self.base.set_document_source(source);
    }

    fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>> {
        self.base.get_document_source()
    }
}

impl XMLDocumentFilter for NamespaceBinder {
    fn as_document_handler(self: Rc<Self>) -> Rc<dyn XMLDocumentHandler> {
        self
    }

    fn as_document_source(self: Rc<Self>) -> Rc<dyn XMLDocumentSource> {
        self
    }

    fn as_html_component(self: Rc<Self>) -> Option<Rc<dyn HTMLComponent>> {
        Some(self)
    }
}

/// This namespace context object implements the old and new XNI
/// `NamespaceContext` interface methods so that it can be used across all
/// versions of Xerces2.
///
/// Equivalent to the nested class `NamespaceBinder.NamespaceSupport`.
#[derive(Debug)]
pub struct NamespaceSupport {
    state: RefCell<SupportState>,
}

#[derive(Debug)]
struct SupportState {
    /// The current context index.
    top: usize,
    /// The levels of the entries.
    levels: Vec<usize>,
    /// The entries: (prefix, uri).
    entries: Vec<(String, Option<String>)>,
}

impl Default for NamespaceSupport {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceSupport {
    /// Default constructor.
    ///
    /// Equivalent to `new NamespaceSupport()`.
    pub fn new() -> Self {
        let s = Self {
            state: RefCell::new(SupportState {
                top: 0,
                levels: vec![0; 10],
                entries: Vec::with_capacity(10),
            }),
        };
        s.push_context();
        s.declare_prefix("xml", Some(namespace_context::XML_URI));
        s.declare_prefix("xmlns", Some(namespace_context::XMLNS_URI));
        s
    }

    /// Returns the parent context.
    ///
    /// Equivalent to `getParentContext()` (it returns `this`).
    pub fn get_parent_context(&self) -> &dyn NamespaceContext {
        self
    }
}

impl NamespaceContext for NamespaceSupport {
    fn get_uri(&self, prefix: &str) -> Option<String> {
        let s = self.state.borrow();
        (0..s.levels[s.top])
            .rev()
            .find(|&i| s.entries[i].0 == prefix)
            .and_then(|i| s.entries[i].1.clone())
    }

    fn get_declared_prefix_count(&self) -> i32 {
        let s = self.state.borrow();
        (s.levels[s.top] - s.levels[s.top - 1]) as i32
    }

    fn get_declared_prefix_at(&self, index: i32) -> String {
        let s = self.state.borrow();
        s.entries[s.levels[s.top - 1] + index as usize].0.clone()
    }

    fn reset(&self) {
        let mut s = self.state.borrow_mut();
        s.top = 1;
        s.levels[1] = s.levels[0];
    }

    fn push_context(&self) {
        let mut s = self.state.borrow_mut();
        s.top += 1;
        if s.top == s.levels.len() {
            let len = s.levels.len();
            s.levels.resize(len + 10, 0);
        }
        let top = s.top;
        s.levels[top] = s.levels[top - 1];
    }

    fn pop_context(&self) {
        let mut s = self.state.borrow_mut();
        if s.top > 1 {
            s.top -= 1;
        }
    }

    fn declare_prefix(&self, prefix: &str, uri: Option<&str>) -> bool {
        let count = self.get_declared_prefix_count();
        for i in 0..count {
            if self.get_declared_prefix_at(i) == prefix {
                return false;
            }
        }
        let mut s = self.state.borrow_mut();
        let top = s.top;
        let at = s.levels[top];
        s.entries.truncate(at);
        s.entries
            .push((prefix.to_string(), uri.map(str::to_string)));
        s.levels[top] += 1;
        true
    }

    /// # Panics
    ///
    /// For an entry bound to no URI, where Java's `entry.uri.equals(uri)`
    /// throws `NullPointerException`.
    fn get_prefix(&self, uri: Option<&str>) -> Option<String> {
        let s = self.state.borrow();
        for i in (0..s.levels[s.top]).rev() {
            let entry = &s.entries[i];
            let entry_uri = entry.1.as_deref().expect("java.lang.NullPointerException");
            if Some(entry_uri) == uri {
                return Some(entry.0.clone());
            }
        }
        None
    }

    // The loop keeps Java's indexed `for (int i = fLevels[1]; ...)`.
    #[allow(clippy::needless_range_loop)]
    fn get_all_prefixes(&self) -> Vec<String> {
        let s = self.state.borrow();
        let mut prefixes: Vec<String> = Vec::new();
        for i in s.levels[1]..s.levels[s.top] {
            let prefix = &s.entries[i].0;
            if !prefixes.contains(prefix) {
                prefixes.push(prefix.clone());
            }
        }
        prefixes
    }
}
