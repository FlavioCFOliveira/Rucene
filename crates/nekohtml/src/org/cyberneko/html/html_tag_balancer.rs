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

//! Port of `HTMLTagBalancer.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use rucene::error::LuceneError;
use rucene_queryparser::jdk::object::JavaObject;

use crate::java::{is_java_whitespace, object_as_interface, object_as_str, Exception, JResult, Object};
use crate::org::apache::xerces::util::xml_attributes_impl::XMLAttributesImpl;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
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
use crate::org::cyberneko::html::filters::namespace_binder::XHTML_1_0_URI;
use crate::org::cyberneko::html::html_augmentations::HTMLAugmentations;
use crate::org::cyberneko::html::html_component::HTMLComponent;
use crate::org::cyberneko::html::html_elements::{self, Element};
use crate::org::cyberneko::html::html_error_reporter::HTMLErrorReporter;
use crate::org::cyberneko::html::html_event_info::SynthesizedItem;
use crate::org::cyberneko::html::html_scanner;
use crate::org::cyberneko::html::html_tag_balancing_listener::HTMLTagBalancingListener;
use crate::org::cyberneko::html::lost_text::LostText;
use crate::org::cyberneko::html::xercesbridge::xerces_bridge;

// features

/// Namespaces.
pub const NAMESPACES: &str = "http://xml.org/sax/features/namespaces";
/// Include infoset augmentations.
pub const AUGMENTATIONS: &str = "http://cyberneko.org/html/features/augmentations";
/// Report errors.
pub const REPORT_ERRORS: &str = "http://cyberneko.org/html/features/report-errors";
/// Document fragment balancing only (deprecated).
pub const DOCUMENT_FRAGMENT_DEPRECATED: &str = "http://cyberneko.org/html/features/document-fragment";
/// Document fragment balancing only.
pub const DOCUMENT_FRAGMENT: &str = "http://cyberneko.org/html/features/balance-tags/document-fragment";
/// Ignore outside content.
pub const IGNORE_OUTSIDE_CONTENT: &str = "http://cyberneko.org/html/features/balance-tags/ignore-outside-content";

/// Recognized features.
const RECOGNIZED_FEATURES: [&str; 6] = [
    NAMESPACES,
    AUGMENTATIONS,
    REPORT_ERRORS,
    DOCUMENT_FRAGMENT_DEPRECATED,
    DOCUMENT_FRAGMENT,
    IGNORE_OUTSIDE_CONTENT,
];

/// Recognized features defaults.
const RECOGNIZED_FEATURES_DEFAULTS: [Option<bool>; 6] = [None, None, None, None, Some(false), Some(false)];

// properties

/// Modify HTML element names: { "upper", "lower", "default" }.
pub const NAMES_ELEMS: &str = "http://cyberneko.org/html/properties/names/elems";
/// Modify HTML attribute names: { "upper", "lower", "default" }.
pub const NAMES_ATTRS: &str = "http://cyberneko.org/html/properties/names/attrs";
/// Error reporter.
pub const ERROR_REPORTER: &str = "http://cyberneko.org/html/properties/error-reporter";
/// For feature "http://cyberneko.org/html/features/balance-tags/document-fragment",
/// an array of qualified names of the elements in which the fragment is
/// parsed (a `QName[]`, here an `Rc<Vec<QName>>` object).
pub const FRAGMENT_CONTEXT_STACK: &str = "http://cyberneko.org/html/properties/balance-tags/fragment-context-stack";

/// Recognized properties.
const RECOGNIZED_PROPERTIES: [&str; 4] = [NAMES_ELEMS, NAMES_ATTRS, ERROR_REPORTER, FRAGMENT_CONTEXT_STACK];

// modify HTML names

/// Don't modify HTML names.
pub const NAMES_NO_CHANGE: i16 = 0;
/// Match HTML element names.
pub const NAMES_MATCH: i16 = 0;
/// Uppercase HTML names.
pub const NAMES_UPPERCASE: i16 = 1;
/// Lowercase HTML names.
pub const NAMES_LOWERCASE: i16 = 2;

/// Synthesized event info item.
pub const SYNTHESIZED_ITEM: SynthesizedItem = SynthesizedItem;

/// Modifies the given name based on the specified mode.
///
/// Equivalent to the protected static `modifyName(String, short)`.
pub fn modify_name(name: &str, mode: i16) -> String {
    html_scanner::modify_name(name, mode)
}

/// Converts HTML names string value to constant value.
///
/// Equivalent to the protected static `getNamesValue(String)`.
pub fn get_names_value(value: &str) -> i16 {
    html_scanner::get_names_value(value)
}

fn npe() -> Exception {
    Exception::runtime(LuceneError::null_pointer())
}

/// Element info for each start element. This information is used when
/// closing unbalanced inline elements.
///
/// Equivalent to the nested class `HTMLTagBalancer.Info`.
#[derive(Debug)]
pub struct Info {
    /// The element.
    pub element: &'static Element,
    /// The element qualified name.
    pub qname: QName,
    /// The element attributes.
    pub attributes: Option<XMLAttributesImpl>,
}

impl Info {
    /// Creates an element information object.
    ///
    /// Equivalent to `new Info(Element, QName)`.
    pub fn new(element: &'static Element, qname: &QName) -> Self {
        Self::with_attributes(element, qname, None)
    }

    /// Creates an element information object (the attributes are copied).
    ///
    /// Equivalent to `new Info(Element, QName, XMLAttributes)`.
    pub fn with_attributes(element: &'static Element, qname: &QName, attributes: Option<&dyn XMLAttributes>) -> Self {
        let mut copy = None;
        if let Some(attributes) = attributes {
            let length = attributes.get_length();
            if length > 0 {
                let mut aqname = QName::new();
                let mut newattrs = XMLAttributesImpl::new();
                for i in 0..length {
                    attributes.get_name(i, &mut aqname);
                    let type_ = attributes.get_type(i);
                    let value = attributes.get_value(i);
                    let non_normalized_value = attributes.get_non_normalized_value(i);
                    let specified = attributes.is_specified(i);
                    newattrs.add_attribute(&aqname, type_.as_deref().unwrap_or(""), value.as_deref());
                    newattrs.set_non_normalized_value(i, non_normalized_value.as_deref());
                    newattrs.set_specified(i, specified);
                }
                copy = Some(newattrs);
            }
        }
        Self {
            element,
            qname: qname.clone(),
            attributes: copy,
        }
    }

    /// Simple representation to make debugging easier: `Object.toString()`
    /// (class and address) then the name.
    pub fn to_java_string(&self) -> String {
        format!(
            "org.cyberneko.html.HTMLTagBalancer$Info@{:x}{}",
            self as *const Info as usize as u32,
            self.qname.to_java_string()
        )
    }
}

/// Unsynchronized stack of element information.
///
/// Equivalent to the nested class `HTMLTagBalancer.InfoStack`; an `Info`
/// is shared between the element and inline stacks, as in Java.
#[derive(Debug, Default)]
pub struct InfoStack {
    /// The top of the stack.
    pub top: usize,
    /// The stack data.
    pub data: Vec<Rc<Info>>,
}

impl InfoStack {
    /// Pushes element information onto the stack.
    ///
    /// Equivalent to `push(Info)`.
    pub fn push(&mut self, info: Rc<Info>) {
        if self.top == self.data.len() {
            self.data.push(info);
        } else {
            self.data[self.top] = info;
        }
        self.top += 1;
    }

    /// Peeks at the top of the stack.
    ///
    /// Equivalent to `peek()`.
    pub fn peek(&self) -> Rc<Info> {
        Rc::clone(&self.data[self.top - 1])
    }

    /// Pops the top item off of the stack.
    ///
    /// Equivalent to `pop()`.
    pub fn pop(&mut self) -> Rc<Info> {
        self.top -= 1;
        Rc::clone(&self.data[self.top])
    }

    /// Simple representation to make debugging easier.
    ///
    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        let mut sb = String::from("InfoStack(");
        for i in (0..self.top).rev() {
            sb.push_str(&self.data[i].to_java_string());
            if i != 0 {
                sb.push_str(", ");
            }
        }
        sb.push(')');
        sb
    }
}

/// Structure to hold information about an element placed in buffer to be
/// comsumed later.
///
/// Equivalent to the package-private nested class
/// `HTMLTagBalancer.ElementEntry`.
#[derive(Debug)]
struct ElementEntry {
    name: QName,
    augs: Option<HTMLAugmentations>,
}

impl ElementEntry {
    fn new(element: &QName, augs: Option<&dyn Augmentations>) -> Self {
        Self {
            name: element.clone(),
            augs: augs.map(HTMLAugmentations::copy_of),
        }
    }
}

/// Balances tags in an HTML document. This component receives document
/// events and tries to correct many common mistakes that human (and
/// computer) HTML document authors make.
///
/// Equivalent to `org.cyberneko.html.HTMLTagBalancer`; its state is in
/// cells (see the crate documentation), and the document source is held
/// weakly.
pub struct HTMLTagBalancer {
    #[allow(dead_code)]
    self_ref: Weak<HTMLTagBalancer>,

    // features
    /// Namespaces.
    namespaces: Cell<bool>,
    /// Include infoset augmentations.
    augmentations: Cell<bool>,
    /// Report errors.
    report_errors: Cell<bool>,
    /// Document fragment balancing only.
    document_fragment: Cell<bool>,
    /// Ignore outside content.
    ignore_outside_content: Cell<bool>,
    /// Allows self closing iframe tags.
    allow_selfclosing_iframe: Cell<bool>,
    /// Allows self closing tags.
    allow_selfclosing_tags: Cell<bool>,

    // properties
    /// Modify HTML element names.
    names_elems: Cell<i16>,
    /// Modify HTML attribute names.
    names_attrs: Cell<i16>,
    /// Error reporter.
    error_reporter: RefCell<Option<Rc<dyn HTMLErrorReporter>>>,

    // connections
    /// The document source.
    document_source: RefCell<Option<Weak<dyn XMLDocumentSource>>>,
    /// The document handler.
    document_handler: RefCell<Option<Rc<dyn XMLDocumentHandler>>>,

    // state
    /// The element stack.
    element_stack: RefCell<InfoStack>,
    /// The inline stack.
    inline_stack: RefCell<InfoStack>,
    /// True if seen anything. Important for xml declaration.
    seen_anything: Cell<bool>,
    /// True if root element has been seen.
    seen_doctype: Cell<bool>,
    /// True if root element has been seen.
    seen_root_element: Cell<bool>,
    /// True if seen the end of the document element.
    seen_root_element_end: Cell<bool>,
    /// True if seen &lt;head&lt; element.
    seen_head_element: Cell<bool>,
    /// True if seen &lt;body&lt; element.
    seen_body_element: Cell<bool>,
    seen_body_element_end: Cell<bool>,
    seen_frameset_element: Cell<bool>,
    /// True if a form is in the stack (allow to discard opening of nested
    /// forms).
    opened_form: Cell<bool>,

    // temp vars
    /// A qualified name.
    qname: RefCell<QName>,
    /// Empty attributes.
    empty_attrs: RefCell<Option<XMLAttributesImpl>>,
    /// Augmentations.
    infoset_augs: HTMLAugmentations,
    /// The tag balancing listener.
    tag_balancing_listener: RefCell<Option<Rc<dyn HTMLTagBalancingListener>>>,
    lost_text: RefCell<LostText>,
    forced_start_element: Cell<bool>,
    forced_end_element: Cell<bool>,
    /// Stack of elements determining the context in which a document
    /// fragment should be parsed.
    fragment_context_stack: RefCell<Option<Rc<Vec<QName>>>>,
    /// Not 0 only when a fragment is parsed and fragmentContextStack_ is
    /// set.
    fragment_context_stack_size: Cell<usize>,
    end_elements_buffer: RefCell<Vec<ElementEntry>>,
}

impl std::fmt::Debug for HTMLTagBalancer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HTMLTagBalancer")
            .field("element_stack", &self.element_stack.borrow().to_java_string())
            .finish_non_exhaustive()
    }
}

impl HTMLTagBalancer {
    /// Equivalent to `new HTMLTagBalancer()`.
    pub fn new() -> Rc<Self> {
        Rc::new_cyclic(|weak| Self {
            self_ref: weak.clone(),
            namespaces: Cell::new(false),
            augmentations: Cell::new(false),
            report_errors: Cell::new(false),
            document_fragment: Cell::new(false),
            ignore_outside_content: Cell::new(false),
            allow_selfclosing_iframe: Cell::new(false),
            allow_selfclosing_tags: Cell::new(false),
            names_elems: Cell::new(NAMES_NO_CHANGE),
            names_attrs: Cell::new(NAMES_NO_CHANGE),
            error_reporter: RefCell::new(None),
            document_source: RefCell::new(None),
            document_handler: RefCell::new(None),
            element_stack: RefCell::new(InfoStack::default()),
            inline_stack: RefCell::new(InfoStack::default()),
            seen_anything: Cell::new(false),
            seen_doctype: Cell::new(false),
            seen_root_element: Cell::new(false),
            seen_root_element_end: Cell::new(false),
            seen_head_element: Cell::new(false),
            seen_body_element: Cell::new(false),
            seen_body_element_end: Cell::new(false),
            seen_frameset_element: Cell::new(false),
            opened_form: Cell::new(false),
            qname: RefCell::new(QName::new()),
            empty_attrs: RefCell::new(Some(XMLAttributesImpl::new())),
            infoset_augs: HTMLAugmentations::new(),
            tag_balancing_listener: RefCell::new(None),
            lost_text: RefCell::new(LostText::default()),
            forced_start_element: Cell::new(false),
            forced_end_element: Cell::new(false),
            fragment_context_stack: RefCell::new(None),
            fragment_context_stack_size: Cell::new(0),
            end_elements_buffer: RefCell::new(Vec::new()),
        })
    }

    fn handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }

    fn reporter(&self) -> JResult<Rc<dyn HTMLErrorReporter>> {
        self.error_reporter.borrow().clone().ok_or_else(npe)
    }

    fn report_error(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()> {
        self.reporter()?.report_error(key, args)
    }

    fn report_warning(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()> {
        self.reporter()?.report_warning(key, args)
    }

    fn arg(s: &Option<String>) -> JavaObject {
        match s {
            Some(s) => JavaObject::String(s.clone()),
            None => JavaObject::Null,
        }
    }

    /// Sets the tag balancing listener.
    ///
    /// Equivalent to the package-private `setTagBalancingListener(
    /// HTMLTagBalancingListener)`.
    pub(crate) fn set_tag_balancing_listener(&self, tag_balancing_listener: Option<Rc<dyn HTMLTagBalancingListener>>) {
        *self.tag_balancing_listener.borrow_mut() = tag_balancing_listener;
    }

    /// Notifies the tagBalancingListener (if any) of an ignored start
    /// element.
    fn notify_discarded_start_element(&self, elem: &QName, attrs: Option<&dyn XMLAttributes>, augs: Option<&dyn Augmentations>) {
        let l_handler = self.tag_balancing_listener.borrow().clone();
        if let Some(l) = l_handler {
            match attrs {
                Some(attrs) => l.ignored_start_element(elem, attrs, augs),
                None => l.ignored_start_element(elem, &XMLAttributesImpl::new(), augs),
            }
        }
    }

    /// Notifies the tagBalancingListener (if any) of an ignored end
    /// element.
    fn notify_discarded_end_element(&self, element: &QName, augs: Option<&dyn Augmentations>) {
        let l_handler = self.tag_balancing_listener.borrow().clone();
        if let Some(l) = l_handler {
            l.ignored_end_element(element, augs);
        }
    }

    fn consume_buffered_end_elements(&self) -> JResult<()> {
        // shallow clone
        let to_consume: Vec<ElementEntry> = std::mem::take(&mut *self.end_elements_buffer.borrow_mut());
        for entry in &to_consume {
            self.forced_end_element.set(true);
            let mut name = entry.name.clone();
            self.end_element_impl(&mut name, entry.augs.as_ref().map(|a| a as &dyn Augmentations))?;
        }
        self.end_elements_buffer.borrow_mut().clear();
        Ok(())
    }

    fn consume_early_text_if_needed(&self) -> JResult<()> {
        if !self.lost_text.borrow().is_empty() {
            if !self.seen_body_element.get() {
                self.force_start_body()?;
            }
            let mut lost_text = std::mem::take(&mut *self.lost_text.borrow_mut());
            lost_text.refeed(self)?;
        }
        Ok(())
    }

    /// Forces an element start, taking care to set the information to allow
    /// startElement to "see" that's the element has been forced.
    ///
    /// Returns `true` if creation could be done (TABLE's creation for
    /// instance can't be forced).
    fn force_start_element(&self, elem: &mut QName, attrs: Option<&mut dyn XMLAttributes>, augs: Option<&dyn Augmentations>) -> JResult<bool> {
        self.forced_start_element.set(true);
        self.start_element_impl(elem, attrs, augs)?;

        let stack = self.element_stack.borrow();
        Ok(stack.top > 0 && elem.equals(&stack.peek().qname))
    }

    fn create_qname(&self, tag_name: &str) -> QName {
        let tag_name = modify_name(tag_name, self.names_elems.get());
        QName::with_values(None, Some(&tag_name), Some(&tag_name), Some(XHTML_1_0_URI))
    }

    fn force_start_body(&self) -> JResult<()> {
        let mut body = self.create_qname("body");
        if self.report_errors.get() {
            self.report_warning("HTML2006", Some(&[Self::arg(&body.localpart)]))?;
        }
        self.force_start_element(&mut body, None, self.synthesized_augs())?;
        Ok(())
    }

    /// Start element.
    fn start_element_impl(
        &self,
        elem: &mut QName,
        mut attrs: Option<&mut dyn XMLAttributes>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.seen_anything.set(true);

        let is_forced_creation = self.forced_start_element.get();
        self.forced_start_element.set(false);

        // check for end of document
        if self.seen_root_element_end.get() {
            self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
            return Ok(());
        }

        // get element information
        let element = self.get_element(elem);
        let element_code = element.code;

        // the creation of some elements like TABLE or SELECT can't be
        // forced. Any others?
        if is_forced_creation && (element_code == html_elements::TABLE || element_code == html_elements::SELECT) {
            return Ok(()); // don't accept creation
        }

        // ignore multiple html, head, body elements
        if self.seen_root_element.get() && element_code == html_elements::HTML {
            self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
            return Ok(());
        }
        if self.seen_frameset_element.get()
            && element_code != html_elements::FRAME
            && element_code != html_elements::FRAMESET
            && element_code != html_elements::NOFRAMES
        {
            self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
            return Ok(());
        }
        if element_code == html_elements::HEAD {
            if self.seen_head_element.get() {
                self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
                return Ok(());
            }
            self.seen_head_element.set(true);
        } else if element_code == html_elements::FRAMESET {
            if !self.seen_head_element.get() {
                let mut head = self.create_qname("head");
                self.force_start_element(&mut head, None, self.synthesized_augs())?;
                self.end_element_impl(&mut head, self.synthesized_augs())?;
            }
            self.consume_buffered_end_elements()?; // </head> (if any) has been buffered
            self.seen_frameset_element.set(true);
        } else if element_code == html_elements::BODY {
            // create <head></head> if none was present
            if !self.seen_head_element.get() {
                let mut head = self.create_qname("head");
                self.force_start_element(&mut head, None, self.synthesized_augs())?;
                self.end_element_impl(&mut head, self.synthesized_augs())?;
            }
            self.consume_buffered_end_elements()?; // </head> (if any) has been buffered

            if self.seen_body_element.get() {
                self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
                return Ok(());
            }
            self.seen_body_element.set(true);
        } else if element_code == html_elements::FORM {
            if self.opened_form.get() {
                self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
                return Ok(());
            }
            self.opened_form.set(true);
        } else if element_code == html_elements::UNKNOWN {
            self.consume_buffered_end_elements()?;
        }

        // check proper parent
        if let Some(parents) = element.parent() {
            let prefered_parent = parents[0];
            if self.document_fragment.get()
                && (prefered_parent.code == html_elements::HEAD || prefered_parent.code == html_elements::BODY)
            {
                // nothing, don't force HEAD or BODY creation for a document
                // fragment
            } else if !self.seen_root_element.get() && !self.document_fragment.get() {
                let pname = modify_name(&prefered_parent.name, self.names_elems.get());
                if self.report_errors.get() {
                    self.report_warning(
                        "HTML2002",
                        Some(&[Self::arg(&elem.rawname), JavaObject::String(pname.clone())]),
                    )?;
                }
                let mut qname = QName::with_values(None, Some(&pname), Some(&pname), None);
                let parent_created = self.force_start_element(&mut qname, None, self.synthesized_augs())?;
                if !parent_created {
                    if !is_forced_creation {
                        self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
                    }
                    return Ok(());
                }
            } else if prefered_parent.code != html_elements::HEAD
                || (!self.seen_body_element.get() && !self.document_fragment.get())
            {
                let depth = self.get_parent_depth(Some(&parents), element.bounds);
                if depth == -1 {
                    // no parent found
                    let pname = modify_name(&prefered_parent.name, self.names_elems.get());
                    let mut qname = QName::with_values(None, Some(&pname), Some(&pname), None);
                    if self.report_errors.get() {
                        self.report_warning(
                            "HTML2004",
                            Some(&[Self::arg(&elem.rawname), JavaObject::String(pname.clone())]),
                        )?;
                    }
                    let parent_created = self.force_start_element(&mut qname, None, self.synthesized_augs())?;
                    if !parent_created {
                        if !is_forced_creation {
                            self.notify_discarded_start_element(elem, attrs.as_deref().map(|a| &*a), augs);
                        }
                        return Ok(());
                    }
                }
            }
        }

        // if block element, save immediate parent inline elements
        let mut depth = 0;
        if element.flags == 0 {
            let length = self.element_stack.borrow().top;
            self.inline_stack.borrow_mut().top = 0;
            for i in (0..length).rev() {
                let info = Rc::clone(&self.element_stack.borrow().data[i]);
                if !info.element.is_inline() {
                    break;
                }
                self.inline_stack.borrow_mut().push(Rc::clone(&info));
                let mut q = info.qname.clone();
                self.end_element_impl(&mut q, self.synthesized_augs())?;
            }
            depth = self.inline_stack.borrow().top;
        }

        // close previous elements
        // all elements close a <script>
        // in head, no element has children
        let pop_one = {
            let stack = self.element_stack.borrow();
            (stack.top > 1 && stack.peek().element.code == html_elements::SCRIPT)
                || (stack.top > 2 && stack.data[stack.top - 2].element.code == html_elements::HEAD)
        };
        if pop_one {
            let info = self.element_stack.borrow_mut().pop();
            if self.handler().is_some() {
                let mut q = info.qname.clone();
                self.call_end_element(&mut q, self.synthesized_augs())?;
            }
        }
        if element.closes.is_some() {
            let mut length = self.element_stack.borrow().top;
            let mut i = length as isize - 1;
            while i >= 0 {
                let info = Rc::clone(&self.element_stack.borrow().data[i as usize]);

                // does it close the element we're looking at?
                if element.closes(info.element.code) {
                    if self.report_errors.get() {
                        self.report_warning(
                            "HTML2005",
                            Some(&[Self::arg(&elem.rawname), Self::arg(&info.qname.rawname)]),
                        )?;
                    }
                    let mut j = length as isize - 1;
                    while j >= i {
                        let info = self.element_stack.borrow_mut().pop();
                        if self.handler().is_some() {
                            // PATCH: Marc-Andr Morissette
                            let mut q = info.qname.clone();
                            self.call_end_element(&mut q, self.synthesized_augs())?;
                        }
                        j -= 1;
                    }
                    length = i as usize;
                    i -= 1;
                    continue;
                }

                // should we stop searching?
                if info.element.is_block() || element.is_parent(info.element) {
                    break;
                }
                i -= 1;
            }
        }
        // TODO: investigate if only table is special here
        // else if (elementCode == HTMLElements.TABLE) {
        //     ...
        // }

        // call handler
        self.seen_root_element.set(true);
        if element.is_empty() {
            if attrs.is_none() {
                let mut empty = self.empty_attributes();
                if let Some(h) = self.handler() {
                    h.empty_element(elem, &mut empty, augs)?;
                }
                *self.empty_attrs.borrow_mut() = Some(empty);
            } else if let Some(h) = self.handler() {
                h.empty_element(elem, attrs.as_deref_mut().expect("INVARIANT: checked above"), augs)?;
            }
        } else {
            let inline = element.is_inline();
            let info = Info::with_attributes(element, elem, if inline { attrs.as_deref().map(|a| &*a) } else { None });
            self.element_stack.borrow_mut().push(Rc::new(info));
            if attrs.is_none() {
                let mut empty = self.empty_attributes();
                if self.handler().is_some() {
                    self.call_start_element(elem, &mut empty, augs)?;
                }
                *self.empty_attrs.borrow_mut() = Some(empty);
            } else if self.handler().is_some() {
                self.call_start_element(elem, attrs.as_deref_mut().expect("INVARIANT: checked above"), augs)?;
            }
        }

        // re-open inline elements
        for _ in 0..depth {
            let info = self.inline_stack.borrow_mut().pop();
            let mut q = info.qname.clone();
            let mut attributes = info.attributes.as_ref().map(clone_attributes);
            self.force_start_element(
                &mut q,
                attributes.as_mut().map(|a| a as &mut dyn XMLAttributes),
                self.synthesized_augs(),
            )?;
        }

        if element_code == html_elements::BODY {
            let mut lost_text = std::mem::take(&mut *self.lost_text.borrow_mut());
            lost_text.refeed(self)?;
        }
        Ok(())
    }

    /// Characters.
    fn characters_impl(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        // check for end of document
        if self.seen_root_element_end.get() || self.seen_body_element_end.get() {
            return Ok(());
        }

        if self.element_stack.borrow().top == 0 && !self.document_fragment.get() {
            // character before first opening tag
            self.lost_text.borrow_mut().add(text, augs);
            return Ok(());
        }

        // is this text whitespace?
        let whitespace = text.length <= 0 || text.chars().iter().all(|&c| is_java_whitespace(c));

        if !self.document_fragment.get() {
            // handle bare characters
            if !self.seen_root_element.get() {
                if whitespace {
                    return Ok(());
                }
                self.force_start_body()?;
            }

            if whitespace
                && (self.element_stack.borrow().top < 2 || self.end_elements_buffer.borrow().len() == 1)
            {
                // ignore spaces directly within <html>
                return Ok(());
            }
            // handle character content in head
            // NOTE: This frequently happens when the document looks like:
            //       <title>Title</title>
            //       And here's some text.
            else if !whitespace {
                let info = self.element_stack.borrow().peek();
                if info.element.code == html_elements::HEAD || info.element.code == html_elements::HTML {
                    let hname = modify_name("head", self.names_elems.get());
                    let bname = modify_name("body", self.names_elems.get());
                    if self.report_errors.get() {
                        self.report_warning("HTML2009", Some(&[JavaObject::String(hname), JavaObject::String(bname)]))?;
                    }
                    self.force_start_body()?;
                }
            }
        }

        // call handler
        if let Some(h) = self.handler() {
            h.characters(text, augs)?;
        }
        Ok(())
    }

    /// End element.
    fn end_element_impl(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        let forced_end_element = self.forced_end_element.get();
        // is there anything to do?
        if self.seen_root_element_end.get() {
            self.notify_discarded_end_element(element, augs);
            return Ok(());
        }

        // get element information
        let elem = self.get_element(element);

        // if we consider outside content, just buffer the elements
        if !self.ignore_outside_content.get() && (elem.code == html_elements::BODY || elem.code == html_elements::HTML) {
            self.end_elements_buffer.borrow_mut().push(ElementEntry::new(element, augs));
            return Ok(());
        }

        if self.seen_frameset_element.get() && elem.code != html_elements::FRAME && elem.code != html_elements::FRAMESET {
            self.notify_discarded_end_element(element, augs);
            return Ok(());
        }

        // check for end of document
        if elem.code == html_elements::HTML {
            self.seen_root_element_end.set(true);
        } else if self.ignore_outside_content.get() {
            if elem.code == html_elements::BODY {
                self.seen_body_element_end.set(true);
            } else if self.seen_body_element_end.get() {
                self.notify_discarded_end_element(element, augs);
                return Ok(());
            }
        } else if elem.code == html_elements::FORM {
            self.opened_form.set(false);
        } else if elem.code == html_elements::HEAD && !forced_end_element {
            // consume </head> first when <body> is reached to retrieve
            // content lost between </head> and <body>
            self.end_elements_buffer.borrow_mut().push(ElementEntry::new(element, augs));
            return Ok(());
        }

        // empty element
        let depth = self.get_element_depth(elem);
        if depth == -1 {
            if elem.code == html_elements::P {
                let mut empty = self.empty_attributes();
                self.force_start_element(element, Some(&mut empty), self.synthesized_augs())?;
                *self.empty_attrs.borrow_mut() = Some(empty);
                self.end_element_impl(element, augs)?;
            } else if !elem.is_empty() {
                self.notify_discarded_end_element(element, augs);
            }
            return Ok(());
        }

        // find unbalanced inline elements
        if depth > 1 && elem.is_inline() {
            let size = self.element_stack.borrow().top;
            self.inline_stack.borrow_mut().top = 0;
            for i in 0..(depth - 1) as usize {
                let info = Rc::clone(&self.element_stack.borrow().data[size - i - 1]);
                let pelem = info.element;
                if pelem.is_inline() || pelem.code == html_elements::FONT {
                    // TODO: investigate if only FONT
                    // NOTE: I don't have to make a copy of the info because
                    //       it will just be popped off of the element stack
                    //       as soon as we close it, anyway.
                    self.inline_stack.borrow_mut().push(info);
                }
            }
        }

        // close children up to appropriate element
        for i in 0..depth {
            let info = self.element_stack.borrow_mut().pop();
            if self.report_errors.get() && i < depth - 1 {
                let ename = element.rawname.as_deref().map(|r| modify_name(r, self.names_elems.get()));
                self.report_warning("HTML2007", Some(&[Self::arg(&ename), Self::arg(&info.qname.rawname)]))?;
            }
            if self.handler().is_some() {
                // PATCH: Marc-Andr Morissette
                let mut q = info.qname.clone();
                self.call_end_element(&mut q, if i < depth - 1 { self.synthesized_augs() } else { augs })?;
            }
        }

        // re-open inline elements
        if depth > 1 {
            let size = self.inline_stack.borrow().top;
            for _ in 0..size {
                let info = self.inline_stack.borrow_mut().pop();
                let mut attributes = info.attributes.as_ref().map(clone_attributes);
                if self.report_errors.get() {
                    self.report_warning("HTML2008", Some(&[Self::arg(&info.qname.rawname)]))?;
                }
                let mut q = info.qname.clone();
                self.force_start_element(
                    &mut q,
                    attributes.as_mut().map(|a| a as &mut dyn XMLAttributes),
                    self.synthesized_augs(),
                )?;
            }
        }
        Ok(())
    }

    //
    // Protected methods
    //

    /// Returns an HTML element.
    ///
    /// Equivalent to the protected `getElement(QName)`.
    pub fn get_element(&self, element_name: &QName) -> &'static Element {
        let mut name = element_name.rawname.clone().unwrap_or_default();
        if self.namespaces.get() && element_name.uri.as_deref() == Some(XHTML_1_0_URI) {
            if let Some(index) = name.find(':') {
                name = name[index + 1..].to_string();
            }
        }
        html_elements::get_element(&name)
    }

    /// Call document handler start element.
    ///
    /// Equivalent to the protected `callStartElement(QName, XMLAttributes,
    /// Augmentations)`.
    fn call_start_element(&self, element: &mut QName, attrs: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.handler().ok_or_else(npe)?.start_element(element, attrs, augs)
    }

    /// Call document handler end element.
    ///
    /// Equivalent to the protected `callEndElement(QName, Augmentations)`.
    fn call_end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.handler().ok_or_else(npe)?.end_element(element, augs)
    }

    /// Returns the depth of the open tag associated with the specified
    /// element name or -1 if no matching element is found.
    ///
    /// Equivalent to the protected `getElementDepth(Element)`.
    pub fn get_element_depth(&self, element: &Element) -> i32 {
        let container = element.is_container();
        let element_code = element.code;
        let table_body_or_html = element_code == html_elements::TABLE
            || element_code == html_elements::BODY
            || element_code == html_elements::HTML;
        let stack = self.element_stack.borrow();
        let mut depth = -1;
        let mut i = stack.top as isize - 1;
        while i >= self.fragment_context_stack_size.get() as isize {
            let info = &stack.data[i as usize];
            if info.element.code == element.code {
                depth = (stack.top as isize - i) as i32;
                break;
            }
            if !container && info.element.is_block() {
                break;
            }
            if info.element.code == html_elements::TABLE && !table_body_or_html {
                return -1; // current element not allowed to close a table
            }
            i -= 1;
        }
        depth
    }

    /// Returns the depth of the open tag associated with the specified
    /// element parent names or -1 if no matching element is found.
    ///
    /// Equivalent to the protected `getParentDepth(Element[], short)`.
    pub fn get_parent_depth(&self, parents: Option<&[&'static Element]>, bounds: i16) -> i32 {
        if let Some(parents) = parents {
            let stack = self.element_stack.borrow();
            for i in (0..stack.top).rev() {
                let info = &stack.data[i];
                if info.element.code == bounds {
                    break;
                }
                for p in parents {
                    if info.element.code == p.code {
                        return (stack.top - i) as i32;
                    }
                }
            }
        }
        -1
    }

    /// Returns a set of empty attributes.
    ///
    /// Equivalent to the protected `emptyAttributes()` (the object is taken
    /// out while a handler uses it, see the crate documentation).
    fn empty_attributes(&self) -> XMLAttributesImpl {
        let mut attrs = self.empty_attrs.borrow_mut().take().unwrap_or_default();
        attrs.remove_all_attributes();
        attrs
    }

    /// Returns an augmentations object with a synthesized item added.
    ///
    /// Equivalent to the protected `synthesizedAugs()`.
    fn synthesized_augs(&self) -> Option<&dyn Augmentations> {
        if self.augmentations.get() {
            self.infoset_augs.remove_all_items();
            self.infoset_augs
                .put_item(AUGMENTATIONS, Some(Rc::new(SYNTHESIZED_ITEM) as Object));
            Some(&self.infoset_augs)
        } else {
            None
        }
    }

    /// Start prefix mapping (XNI 2.0).
    ///
    /// Equivalent to `startPrefixMapping(String, String, Augmentations)`.
    pub fn start_prefix_mapping(&self, prefix: &str, uri: Option<&str>, augs: Option<&dyn Augmentations>) {
        // check for end of document
        if self.seen_root_element_end.get() {
            return;
        }
        // call handler
        if let Some(h) = self.handler() {
            xerces_bridge::get_instance().xml_document_handler_start_prefix_mapping(&*h, prefix, uri, augs);
        }
    }

    /// End prefix mapping (XNI 2.0).
    ///
    /// Equivalent to `endPrefixMapping(String, Augmentations)`.
    pub fn end_prefix_mapping(&self, prefix: &str, augs: Option<&dyn Augmentations>) {
        // check for end of document
        if self.seen_root_element_end.get() {
            return;
        }
        // call handler
        if let Some(h) = self.handler() {
            xerces_bridge::get_instance().xml_document_handler_end_prefix_mapping(&*h, prefix, augs);
        }
    }

    /// Start document (XNI 2.0).
    ///
    /// Equivalent to `startDocument(XMLLocator, String, Augmentations)`.
    ///
    /// # Errors
    ///
    /// What the next handler throws.
    pub fn start_document_no_context(&self, locator: Option<Rc<dyn XMLLocator>>, encoding: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        XMLDocumentHandler::start_document(self, locator, encoding, None, augs)
    }
}

/// A copy of saved attributes, for a handler that takes them mutably
/// (Java passes the saved object itself).
fn clone_attributes(attributes: &XMLAttributesImpl) -> XMLAttributesImpl {
    let mut copy = XMLAttributesImpl::new();
    let mut aqname = QName::new();
    for i in 0..attributes.get_length() {
        attributes.get_name(i, &mut aqname);
        copy.add_attribute(
            &aqname,
            attributes.get_type(i).as_deref().unwrap_or(""),
            attributes.get_value(i).as_deref(),
        );
        copy.set_non_normalized_value(i, attributes.get_non_normalized_value(i).as_deref());
        copy.set_specified(i, attributes.is_specified(i));
    }
    copy
}

//
// HTMLComponent methods
//

impl XMLComponent for HTMLTagBalancer {
    fn get_feature_default(&self, feature_id: &str) -> Option<bool> {
        RECOGNIZED_FEATURES
            .iter()
            .position(|f| *f == feature_id)
            .and_then(|i| RECOGNIZED_FEATURES_DEFAULTS[i])
    }

    fn get_property_default(&self, _property_id: &str) -> Option<Object> {
        None
    }

    fn get_recognized_features(&self) -> Option<Vec<String>> {
        Some(RECOGNIZED_FEATURES.iter().map(|s| s.to_string()).collect())
    }

    fn get_recognized_properties(&self) -> Option<Vec<String>> {
        Some(RECOGNIZED_PROPERTIES.iter().map(|s| s.to_string()).collect())
    }

    /// Resets the component.
    fn reset(&self, manager: &dyn XMLComponentManager) -> JResult<()> {
        // get features
        self.namespaces.set(manager.get_feature(NAMESPACES)?);
        self.augmentations.set(manager.get_feature(AUGMENTATIONS)?);
        self.report_errors.set(manager.get_feature(REPORT_ERRORS)?);
        self.document_fragment
            .set(manager.get_feature(DOCUMENT_FRAGMENT)? || manager.get_feature(DOCUMENT_FRAGMENT_DEPRECATED)?);
        self.ignore_outside_content.set(manager.get_feature(IGNORE_OUTSIDE_CONTENT)?);
        self.allow_selfclosing_iframe
            .set(manager.get_feature(html_scanner::ALLOW_SELFCLOSING_IFRAME)?);
        self.allow_selfclosing_tags
            .set(manager.get_feature(html_scanner::ALLOW_SELFCLOSING_TAGS)?);

        // get properties
        let value_of = |o: Option<Object>| -> String {
            o.as_ref()
                .and_then(object_as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "null".to_string())
        };
        self.names_elems.set(get_names_value(&value_of(manager.get_property(NAMES_ELEMS)?)));
        self.names_attrs.set(get_names_value(&value_of(manager.get_property(NAMES_ATTRS)?)));
        let reporter = manager.get_property(ERROR_REPORTER)?;
        *self.error_reporter.borrow_mut() = match &reporter {
            None => None,
            Some(o) => Some(object_as_interface::<dyn HTMLErrorReporter>(o).ok_or_else(|| {
                Exception::runtime(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "org.cyberneko.html.HTMLErrorReporter",
                ))
            })?),
        };
        let stack = manager.get_property(FRAGMENT_CONTEXT_STACK)?;
        *self.fragment_context_stack.borrow_mut() = match &stack {
            None => None,
            Some(o) => Some(o.clone().downcast::<Vec<QName>>().map_err(|_| {
                Exception::runtime(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "[Lorg.apache.xerces.xni.QName;",
                ))
            })?),
        };
        Ok(())
    }

    /// Sets a feature.
    fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()> {
        if feature_id == AUGMENTATIONS {
            self.augmentations.set(state);
            return Ok(());
        }
        if feature_id == REPORT_ERRORS {
            self.report_errors.set(state);
            return Ok(());
        }
        if feature_id == IGNORE_OUTSIDE_CONTENT {
            self.ignore_outside_content.set(state);
            return Ok(());
        }
        Ok(())
    }

    /// Sets a property.
    fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        let value_of = |o: &Option<Object>| -> String {
            o.as_ref()
                .and_then(object_as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "null".to_string())
        };
        if property_id == NAMES_ELEMS {
            self.names_elems.set(get_names_value(&value_of(&value)));
            return Ok(());
        }
        if property_id == NAMES_ATTRS {
            self.names_attrs.set(get_names_value(&value_of(&value)));
            return Ok(());
        }
        Ok(())
    }
}

impl HTMLComponent for HTMLTagBalancer {}

//
// XMLDocumentSource methods
//

impl XMLDocumentSource for HTMLTagBalancer {
    fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>) {
        *self.document_handler.borrow_mut() = handler;
    }

    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }
}

//
// XMLDocumentHandler methods
//

impl XMLDocumentHandler for HTMLTagBalancer {
    /// Start document.
    fn start_document(
        &self,
        locator: Option<Rc<dyn XMLLocator>>,
        encoding: Option<&str>,
        nscontext: Option<Rc<dyn NamespaceContext>>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        // reset state
        self.element_stack.borrow_mut().top = 0;
        let fragment_context_stack = self.fragment_context_stack.borrow().clone();
        if let Some(stack) = fragment_context_stack {
            self.fragment_context_stack_size.set(stack.len());
            for name in stack.iter() {
                let elt = html_elements::get_element(name.localpart.as_deref().unwrap_or(""));
                self.element_stack.borrow_mut().push(Rc::new(Info::new(elt, name)));
            }
        } else {
            self.fragment_context_stack_size.set(0);
        }

        self.seen_anything.set(false);
        self.seen_doctype.set(false);
        self.seen_root_element.set(false);
        self.seen_root_element_end.set(false);
        self.seen_head_element.set(false);
        self.seen_body_element.set(false);

        // pass on event
        if let Some(h) = self.handler() {
            xerces_bridge::get_instance().xml_document_handler_start_document(&*h, locator, encoding, nscontext, augs)?;
        }
        Ok(())
    }

    /// XML declaration.
    fn xml_decl(&self, version: Option<&str>, encoding: Option<&str>, standalone: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        if !self.seen_anything.get() {
            if let Some(h) = self.handler() {
                h.xml_decl(version, encoding, standalone, augs)?;
            }
        }
        Ok(())
    }

    /// Doctype declaration.
    fn doctype_decl(
        &self,
        root_element_name: Option<&str>,
        public_id: Option<&str>,
        system_id: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.seen_anything.set(true);
        if self.report_errors.get() {
            if self.seen_root_element.get() {
                self.report_error("HTML2010", None)?;
            } else if self.seen_doctype.get() {
                self.report_error("HTML2011", None)?;
            }
        }
        if !self.seen_root_element.get() && !self.seen_doctype.get() {
            self.seen_doctype.set(true);
            if let Some(h) = self.handler() {
                h.doctype_decl(root_element_name, public_id, system_id, augs)?;
            }
        }
        Ok(())
    }

    /// Comment.
    fn comment(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.seen_anything.set(true);
        self.consume_early_text_if_needed()?;
        if let Some(h) = self.handler() {
            h.comment(text, augs)?;
        }
        Ok(())
    }

    /// Processing instruction.
    fn processing_instruction(&self, target: &str, data: Option<&XMLString>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.seen_anything.set(true);
        self.consume_early_text_if_needed()?;
        if let Some(h) = self.handler() {
            h.processing_instruction(target, data, augs)?;
        }
        Ok(())
    }

    /// Start element.
    fn start_element(&self, elem: &mut QName, attrs: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.start_element_impl(elem, Some(attrs), augs)
    }

    /// Empty element.
    fn empty_element(&self, element: &mut QName, attrs: &mut dyn XMLAttributes, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.start_element_impl(element, Some(attrs), augs)?;
        // browser ignore the closing indication for non empty tags like
        // <form .../> but not for unknown element
        let elem = self.get_element(element);
        if elem.is_empty()
            || self.allow_selfclosing_tags.get()
            || elem.code == html_elements::UNKNOWN
            || (elem.code == html_elements::IFRAME && self.allow_selfclosing_iframe.get())
        {
            self.end_element_impl(element, augs)?;
        }
        Ok(())
    }

    /// Start entity.
    fn start_general_entity(
        &self,
        name: &str,
        id: Option<&dyn XMLResourceIdentifier>,
        encoding: Option<&str>,
        augs: Option<&dyn Augmentations>,
    ) -> JResult<()> {
        self.seen_anything.set(true);

        // check for end of document
        if self.seen_root_element_end.get() {
            return Ok(());
        }

        // insert body, if needed
        if !self.document_fragment.get() {
            let mut insert_body = !self.seen_root_element.get();
            if !insert_body {
                let info = self.element_stack.borrow().peek();
                if info.element.code == html_elements::HEAD || info.element.code == html_elements::HTML {
                    let hname = modify_name("head", self.names_elems.get());
                    let bname = modify_name("body", self.names_elems.get());
                    if self.report_errors.get() {
                        self.report_warning(
                            "HTML2009",
                            Some(&[JavaObject::String(hname.clone()), JavaObject::String(bname)]),
                        )?;
                    }
                    let mut q = {
                        let mut q = self.qname.borrow_mut();
                        q.set_values(None, Some(&hname), Some(&hname), None);
                        q.clone()
                    };
                    self.end_element_impl(&mut q, self.synthesized_augs())?;
                    insert_body = true;
                }
            }
            if insert_body {
                self.force_start_body()?;
            }
        }

        // call handler
        if let Some(h) = self.handler() {
            h.start_general_entity(name, id, encoding, augs)?;
        }
        Ok(())
    }

    /// Text declaration.
    fn text_decl(&self, version: Option<&str>, encoding: Option<&str>, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.seen_anything.set(true);

        // check for end of document
        if self.seen_root_element_end.get() {
            return Ok(());
        }

        // call handler
        if let Some(h) = self.handler() {
            h.text_decl(version, encoding, augs)?;
        }
        Ok(())
    }

    /// End entity.
    fn end_general_entity(&self, name: &str, augs: Option<&dyn Augmentations>) -> JResult<()> {
        // check for end of document
        if self.seen_root_element_end.get() {
            return Ok(());
        }

        // call handler
        if let Some(h) = self.handler() {
            h.end_general_entity(name, augs)?;
        }
        Ok(())
    }

    /// Start CDATA section.
    fn start_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.seen_anything.set(true);

        self.consume_early_text_if_needed()?;

        // check for end of document
        if self.seen_root_element_end.get() {
            return Ok(());
        }

        // call handler
        if let Some(h) = self.handler() {
            h.start_cdata(augs)?;
        }
        Ok(())
    }

    /// End CDATA section.
    fn end_cdata(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        // check for end of document
        if self.seen_root_element_end.get() {
            return Ok(());
        }

        // call handler
        if let Some(h) = self.handler() {
            h.end_cdata(augs)?;
        }
        Ok(())
    }

    /// Characters.
    fn characters(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.characters_impl(text, augs)
    }

    /// Ignorable whitespace.
    fn ignorable_whitespace(&self, text: &XMLString, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.characters_impl(text, augs)
    }

    /// End element.
    fn end_element(&self, element: &mut QName, augs: Option<&dyn Augmentations>) -> JResult<()> {
        self.end_element_impl(element, augs)
    }

    /// End document.
    fn end_document(&self, augs: Option<&dyn Augmentations>) -> JResult<()> {
        // </body> and </html> have been buffered to consider outside
        // content
        self.ignore_outside_content.set(true); // endElement should not ignore the elements passed from buffer
        self.consume_buffered_end_elements()?;

        // handle empty document
        if !self.seen_root_element.get() && !self.document_fragment.get() {
            if self.report_errors.get() {
                self.report_error("HTML2000", None)?;
            }
            if self.handler().is_some() {
                self.seen_root_element_end.set(false);
                self.force_start_body()?; // will force <html> and <head></head>
                let body = modify_name("body", self.names_elems.get());
                let mut q = QName::with_values(None, Some(&body), Some(&body), None);
                *self.qname.borrow_mut() = q.clone();
                self.call_end_element(&mut q, self.synthesized_augs())?;

                let ename = modify_name("html", self.names_elems.get());
                let mut q = QName::with_values(None, Some(&ename), Some(&ename), None);
                *self.qname.borrow_mut() = q.clone();
                self.call_end_element(&mut q, self.synthesized_augs())?;
            }
        }
        // pop all remaining elements
        else {
            let length = self.element_stack.borrow().top as isize - self.fragment_context_stack_size.get() as isize;
            for _ in 0..length.max(0) {
                let info = self.element_stack.borrow_mut().pop();
                if self.report_errors.get() {
                    self.report_warning("HTML2001", Some(&[Self::arg(&info.qname.rawname)]))?;
                }
                if self.handler().is_some() {
                    let mut q = info.qname.clone();
                    self.call_end_element(&mut q, self.synthesized_augs())?;
                }
            }
        }

        // call handler
        if let Some(h) = self.handler() {
            h.end_document(augs)?;
        }
        Ok(())
    }

    /// Sets the document source.
    fn set_document_source(&self, source: Option<Rc<dyn XMLDocumentSource>>) {
        *self.document_source.borrow_mut() = source.map(|s| Rc::downgrade(&s));
    }

    /// Returns the document source.
    fn get_document_source(&self) -> Option<Rc<dyn XMLDocumentSource>> {
        self.document_source.borrow().as_ref().and_then(Weak::upgrade)
    }
}

impl XMLDocumentFilter for HTMLTagBalancer {
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
