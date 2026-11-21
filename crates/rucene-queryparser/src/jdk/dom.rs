/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The part of JAXP (module `java.xml`) that the XML query parser
//! (`queryparser.xml.CoreParser` and `DOMUtils`) uses:
//! `javax.xml.parsers.DocumentBuilderFactory` and `DocumentBuilder`, the
//! `org.w3c.dom` tree they build (`Document`, `Element`, `Node`,
//! `NodeList`), `org.xml.sax.EntityResolver`, `ErrorHandler`,
//! `InputSource`, `SAXException` and `SAXParseException`.
//!
//! # Divergence: a DOM-shaped builder over `quick-xml`
//!
//! Java builds the tree with the JDK's own parser (the Xerces-derived
//! `com.sun.org.apache.xerces.internal`). Rust has none; per the decision on
//! rmp task #493 the builder is an adapter over the `quick-xml` pull parser
//! (MIT, approved on task #461), started from the SAX adapter of
//! `rucene-analysis-common` (`jdk::sax`), which this crate cannot depend on.
//! What it reproduces, observed on JDK 25.0.5 (`P1.java` to `P4.java`, a
//! factory set up as `CoreParser.parseXML` sets it up: not validating,
//! `FEATURE_SECURE_PROCESSING`, a `null` error handler):
//!
//! * **The tree.** Not namespace aware (the factory default): an element is
//!   named by its qualified name and its attributes keep `xmlns`. Text is a
//!   `TEXT_NODE`, merged with an adjacent text node, entity expansions
//!   included (`expandEntityReferences`, the default, so no
//!   `ENTITY_REFERENCE_NODE` is built); a `CDATA` section is a
//!   `CDATA_SECTION_NODE` (`coalescing` is off); comments and processing
//!   instructions are nodes; the `DOCTYPE` is a `DOCUMENT_TYPE_NODE` child
//!   of the document; whitespace outside the root element is dropped. Line
//!   ends are normalized (XML 1.0 section 2.11) and attribute values as XML
//!   1.0 section 3.3.3 says. `getAttribute` gives `""` for an absent
//!   attribute; `getElementsByTagName` the descendants in document order.
//!   `NodeList`s are live, as Xerces' are.
//! * **Entities.** The general entities of the internal subset, and of an
//!   external subset that the entity resolver returns, are expanded, their
//!   replacement text parsed as content (markup included). A reference to an
//!   external general entity in content calls `resolveEntity` with the
//!   expanded system id; what it throws ends the parse; the source it
//!   returns is parsed as content. The external subset named by the
//!   `DOCTYPE` goes through `resolveEntity` too. With no resolver or a
//!   `null` source, a URL whose protocol the JDK does not know is the
//!   `MalformedURLException: unknown protocol: <p>` the JDK throws, and any
//!   other the `accessExternalDTD` restriction of secure processing (the JDK
//!   message), as the factory sets `FEATURE_SECURE_PROCESSING`. A reference
//!   to an undeclared entity is skipped when the `DOCTYPE` names an external
//!   subset and the document is not `standalone="yes"`, as the JDK does, and
//!   is a fatal error otherwise; recursion is the JDK's fatal error.
//! * **The limits of secure processing** (JDK 25 defaults): 2500 entity
//!   expansions, 200 attributes per element, an element depth of 100,
//!   element names of 1000 characters, 100000 characters for one general
//!   entity and for all entities together, each with the JDK's
//!   `JAXP0001000n` message (the JDK also limits attribute names, which the
//!   port does not check).
//! * **Errors.** A fatal error is a [`SaxException`] of class
//!   `SAXParseException` with the JDK's message for the well-formedness
//!   errors listed in [`DocumentBuilder::parse`]; the error handler, when one
//!   is set, gets `fatalError` first. A start tag is checked as the JDK's
//!   `scanStartElement` scans it (whitespace before each attribute, `=`, an
//!   open quote, no `<` in a value, the end of the input inside the tag),
//!   with the JDK's message and position; so is an element after the root
//!   (`MarkupNotRecognizedInMisc`, reported after the `<`). What the entity resolver throws ends the
//!   parse unchanged.
//!
//! What differs, and why:
//!
//! * **Locations.** Apart from the start-tag errors above, line and column
//!   numbers are those of the position `quick-xml` reports for an error (the JDK reports the position its
//!   scanner reached, often a few characters further), and of the reference
//!   for an error inside an entity's replacement text (the JDK reports the
//!   position inside the entity). The sizes the size limits report count the
//!   entity's characters, where the JDK reports the size its scanner had read
//!   when it noticed (in buffer increments); the limits apply at the same
//!   thresholds.
//! * **Messages.** A well-formedness error that is not listed carries
//!   `quick-xml`'s message. Character ranges (XML 1.0 section 2.2) and names
//!   are not validated. The numbers of the limit messages are formatted for
//!   the default locale, where the JDK uses the default locale of the
//!   `FORMAT` category.
//! * **What is not emulated.** Parameter entities are not expanded and the
//!   declarations of the external subset other than general entities
//!   (attribute defaults, element content models) are not applied; DTD
//!   validation is not available (`CoreParser` turns it off). The encodings
//!   are UTF-8, UTF-16 (with a byte-order mark or detected from `<?`),
//!   ISO-8859-1 and US-ASCII; another declared encoding is the JDK's
//!   `Invalid encoding name` error.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use quick_xml::errors::{Error as XmlError, IllFormedError, SyntaxError};
use quick_xml::events::attributes::AttrError;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind, Result};

use super::locale::Locale;
use super::number_format::get_instance;

// ----------------------------------------------------------------------
// org.w3c.dom.Node constants
// ----------------------------------------------------------------------

/// `Node.ELEMENT_NODE`.
pub const ELEMENT_NODE: u16 = 1;
/// `Node.ATTRIBUTE_NODE`.
pub const ATTRIBUTE_NODE: u16 = 2;
/// `Node.TEXT_NODE`.
pub const TEXT_NODE: u16 = 3;
/// `Node.CDATA_SECTION_NODE`.
pub const CDATA_SECTION_NODE: u16 = 4;
/// `Node.ENTITY_REFERENCE_NODE`.
pub const ENTITY_REFERENCE_NODE: u16 = 5;
/// `Node.ENTITY_NODE`.
pub const ENTITY_NODE: u16 = 6;
/// `Node.PROCESSING_INSTRUCTION_NODE`.
pub const PROCESSING_INSTRUCTION_NODE: u16 = 7;
/// `Node.COMMENT_NODE`.
pub const COMMENT_NODE: u16 = 8;
/// `Node.DOCUMENT_NODE`.
pub const DOCUMENT_NODE: u16 = 9;
/// `Node.DOCUMENT_TYPE_NODE`.
pub const DOCUMENT_TYPE_NODE: u16 = 10;
/// `Node.DOCUMENT_FRAGMENT_NODE`.
pub const DOCUMENT_FRAGMENT_NODE: u16 = 11;
/// `Node.NOTATION_NODE`.
pub const NOTATION_NODE: u16 = 12;

/// The binary name of `org.xml.sax.SAXException`.
pub const SAX_EXCEPTION: &str = "org.xml.sax.SAXException";
/// The binary name of `org.xml.sax.SAXParseException`.
pub const SAX_PARSE_EXCEPTION: &str = "org.xml.sax.SAXParseException";
/// The binary name of `javax.xml.parsers.ParserConfigurationException`.
pub const PARSER_CONFIGURATION_EXCEPTION: &str = "javax.xml.parsers.ParserConfigurationException";
/// `XMLConstants.FEATURE_SECURE_PROCESSING`.
pub const FEATURE_SECURE_PROCESSING: &str =
    "http://javax.xml.XMLConstants/feature/secure-processing";

/// The JDK 25 limits of secure processing.
const ENTITY_EXPANSION_LIMIT: u32 = 2500;
const ELEMENT_ATTRIBUTE_LIMIT: usize = 200;
const MAX_ELEMENT_DEPTH: usize = 100;
const MAX_XML_NAME_LIMIT: usize = 1000;
const MAX_GENERAL_ENTITY_SIZE_LIMIT: usize = 100_000;
const TOTAL_ENTITY_SIZE_LIMIT: usize = 100_000;

/// `new <class>(message)` for an `IOException` subclass.
fn io_exception(class: &'static str, message: String) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Io(
        rucene::store::java_nio_file::JavaIoException::new(class, Some(message)).into_io_error(),
    ))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ----------------------------------------------------------------------
// The tree
// ----------------------------------------------------------------------

/// One node of a document.
#[derive(Debug, Clone)]
struct NodeRecord {
    node_type: u16,
    name: String,
    value: Option<String>,
    attributes: Vec<(String, String)>,
    parent: Option<usize>,
    children: Vec<usize>,
    /// Built by the parser (Xerces' deferred node classes), not created by
    /// a `create*` call.
    deferred: bool,
}

/// The nodes of a document; index 0 is the document node.
#[derive(Debug)]
struct DocumentData {
    nodes: Mutex<Vec<NodeRecord>>,
}

/// A node of a DOM tree.
///
/// Equivalent to an `org.w3c.dom.Node` reference. A node holds its document
/// alive, as a Java reference keeps the tree it belongs to reachable;
/// cloning copies the reference, and `==` is Java's identity.
#[derive(Clone)]
pub struct Node {
    doc: Arc<DocumentData>,
    index: usize,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.doc, &other.doc) && self.index == other.index
    }
}

impl Eq for Node {}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nodes = lock(&self.doc.nodes);
        let record = &nodes[self.index];
        write!(f, "[{}: {:?}]", record.name, record.value)
    }
}

impl Node {
    fn with<R>(&self, f: impl FnOnce(&NodeRecord) -> R) -> R {
        f(&lock(&self.doc.nodes)[self.index])
    }

    fn node(&self, index: usize) -> Node {
        Node {
            doc: Arc::clone(&self.doc),
            index,
        }
    }

    /// Equivalent to `Node.getNodeType()`.
    pub fn get_node_type(&self) -> u16 {
        self.with(|r| r.node_type)
    }

    /// Equivalent to `Node.getNodeName()`: the tag name of an element, the
    /// target of a processing instruction, the name of a document type, and
    /// `#text`, `#cdata-section`, `#comment` or `#document` otherwise.
    pub fn get_node_name(&self) -> String {
        self.with(|r| r.name.clone())
    }

    /// Equivalent to `Node.getNodeValue()`: the text of a text, CDATA,
    /// comment or processing instruction node, `None` (Java's `null`) for
    /// the others.
    pub fn get_node_value(&self) -> Option<String> {
        self.with(|r| r.value.clone())
    }

    /// Equivalent to `Node.getParentNode()`.
    pub fn get_parent_node(&self) -> Option<Node> {
        self.with(|r| r.parent).map(|p| self.node(p))
    }

    /// Equivalent to `Node.getFirstChild()`.
    pub fn get_first_child(&self) -> Option<Node> {
        self.with(|r| r.children.first().copied())
            .map(|c| self.node(c))
    }

    /// Equivalent to `Node.getLastChild()`.
    pub fn get_last_child(&self) -> Option<Node> {
        self.with(|r| r.children.last().copied())
            .map(|c| self.node(c))
    }

    /// Equivalent to `Node.getNextSibling()`.
    pub fn get_next_sibling(&self) -> Option<Node> {
        let nodes = lock(&self.doc.nodes);
        let parent = nodes[self.index].parent?;
        let siblings = &nodes[parent].children;
        let at = siblings.iter().position(|&c| c == self.index)?;
        siblings.get(at + 1).copied().map(|c| self.node(c))
    }

    /// Equivalent to `Node.getChildNodes()`: a live list of the children.
    pub fn get_child_nodes(&self) -> NodeList {
        NodeList {
            root: self.clone(),
            kind: NodeListKind::Children,
        }
    }

    /// Equivalent to `Node.getOwnerDocument()`: `None` for the document
    /// itself.
    pub fn get_owner_document(&self) -> Option<Document> {
        if self.index == 0 {
            None
        } else {
            Some(Document(self.node(0)))
        }
    }

    /// Equivalent to `Node.appendChild(Node newChild)`: removes `new_child`
    /// from its parent, then adds it as the last child of this node.
    ///
    /// # Errors
    ///
    /// The `DOMException`s `HIERARCHY_REQUEST_ERR` (a child that is this
    /// node or one of its ancestors, or a node of a type this node cannot
    /// hold) and `WRONG_DOCUMENT_ERR` (a node of another document).
    pub fn append_child(&self, new_child: &Node) -> Result<Node> {
        if !Arc::ptr_eq(&self.doc, &new_child.doc) {
            return Err(dom_exception(
                "WRONG_DOCUMENT_ERR: A node is used in a different document than the one that created it.",
            ));
        }
        let mut nodes = lock(&self.doc.nodes);
        let parent_type = nodes[self.index].node_type;
        let child_type = nodes[new_child.index].node_type;
        let allowed = matches!(parent_type, ELEMENT_NODE | DOCUMENT_NODE)
            && child_type != DOCUMENT_NODE
            && (parent_type != DOCUMENT_NODE
                || !matches!(child_type, TEXT_NODE | CDATA_SECTION_NODE));
        let mut ancestor = Some(self.index);
        while let Some(a) = ancestor {
            if a == new_child.index {
                return Err(dom_exception(
                    "HIERARCHY_REQUEST_ERR: An attempt was made to insert a node where it is not permitted. ",
                ));
            }
            ancestor = nodes[a].parent;
        }
        if !allowed {
            return Err(dom_exception(
                "HIERARCHY_REQUEST_ERR: An attempt was made to insert a node where it is not permitted. ",
            ));
        }
        if let Some(old_parent) = nodes[new_child.index].parent {
            nodes[old_parent].children.retain(|&c| c != new_child.index);
        }
        nodes[new_child.index].parent = Some(self.index);
        nodes[self.index].children.push(new_child.index);
        Ok(new_child.clone())
    }

    /// The element this node is, for `instanceof Element`.
    pub fn as_element(&self) -> Option<Element> {
        if self.get_node_type() == ELEMENT_NODE {
            Some(Element(self.clone()))
        } else {
            None
        }
    }

    /// The Xerces class of this node, for `ClassCastException` messages.
    fn implementation_class(&self) -> String {
        let (node_type, deferred) = self.with(|r| (r.node_type, r.deferred));
        let simple = match node_type {
            ELEMENT_NODE => "ElementImpl",
            TEXT_NODE => "TextImpl",
            CDATA_SECTION_NODE => "CDATASectionImpl",
            COMMENT_NODE => "CommentImpl",
            PROCESSING_INSTRUCTION_NODE => "ProcessingInstructionImpl",
            DOCUMENT_TYPE_NODE => "DocumentTypeImpl",
            _ => "DocumentImpl",
        };
        format!(
            "com.sun.org.apache.xerces.internal.dom.{}{simple}",
            if deferred { "Deferred" } else { "" }
        )
    }
}

/// Equivalent to `new DOMException(code, message)`, an unchecked
/// `org.w3c.dom.DOMException`.
fn dom_exception(message: &str) -> LuceneError {
    LuceneError::unchecked("org.w3c.dom.DOMException", message)
}

/// An element of a DOM tree.
///
/// Equivalent to an `org.w3c.dom.Element` reference; it dereferences to its
/// [`Node`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Element(Node);

impl std::ops::Deref for Element {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.0
    }
}

impl Element {
    /// The cast `(Element) node`.
    ///
    /// # Errors
    ///
    /// The JDK's `ClassCastException` for a node that is not an element.
    pub fn cast(node: &Node) -> Result<Element> {
        match node.as_element() {
            Some(e) => Ok(e),
            None => {
                let class = node.implementation_class();
                Err(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    format!(
                        "class {class} cannot be cast to class org.w3c.dom.Element ({class} and org.w3c.dom.Element are in module java.xml of loader 'bootstrap')"
                    ),
                ))
            }
        }
    }

    /// Equivalent to `Element.getTagName()`.
    pub fn get_tag_name(&self) -> String {
        self.get_node_name()
    }

    /// Equivalent to `Element.getAttribute(String name)`: the value, or `""`
    /// when the element has no such attribute.
    pub fn get_attribute(&self, name: &str) -> String {
        self.with(|r| {
            r.attributes
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        })
    }

    /// Equivalent to `Element.hasAttribute(String name)`.
    pub fn has_attribute(&self, name: &str) -> bool {
        self.with(|r| r.attributes.iter().any(|(n, _)| n == name))
    }

    /// Equivalent to `Element.getElementsByTagName(String name)`: a live
    /// list of the descendant elements named `name` (`*` for all), in
    /// document order.
    pub fn get_elements_by_tag_name(&self, name: &str) -> NodeList {
        NodeList {
            root: self.0.clone(),
            kind: NodeListKind::Descendants(name.to_string()),
        }
    }
}

/// A DOM document.
///
/// Equivalent to an `org.w3c.dom.Document` reference; it dereferences to its
/// [`Node`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Document(Node);

impl std::ops::Deref for Document {
    type Target = Node;

    fn deref(&self) -> &Node {
        &self.0
    }
}

impl Document {
    fn new() -> Self {
        Document(Node {
            doc: Arc::new(DocumentData {
                nodes: Mutex::new(vec![NodeRecord {
                    node_type: DOCUMENT_NODE,
                    name: "#document".to_string(),
                    value: None,
                    attributes: Vec::new(),
                    parent: None,
                    children: Vec::new(),
                    deferred: true,
                }]),
            }),
            index: 0,
        })
    }

    /// Equivalent to `Document.getDocumentElement()`.
    pub fn get_document_element(&self) -> Option<Element> {
        let child = {
            let nodes = lock(&self.doc.nodes);
            nodes[0]
                .children
                .iter()
                .copied()
                .find(|&c| nodes[c].node_type == ELEMENT_NODE)
        };
        child.map(|c| Element(self.node(c)))
    }

    /// Adds a node without a parent and returns it.
    fn create(&self, node_type: u16, name: &str, value: Option<String>, deferred: bool) -> Node {
        let mut nodes = lock(&self.doc.nodes);
        nodes.push(NodeRecord {
            node_type,
            name: name.to_string(),
            value,
            attributes: Vec::new(),
            parent: None,
            children: Vec::new(),
            deferred,
        });
        let index = nodes.len() - 1;
        drop(nodes);
        self.node(index)
    }

    /// Equivalent to `Document.createElement(String tagName)`.
    ///
    /// # Errors
    ///
    /// The `DOMException` `INVALID_CHARACTER_ERR` for a name that is not an
    /// XML name.
    pub fn create_element(&self, tag_name: &str) -> Result<Element> {
        if !is_xml_name(tag_name) {
            return Err(dom_exception(
                "INVALID_CHARACTER_ERR: An invalid or illegal XML character is specified. ",
            ));
        }
        Ok(Element(self.create(ELEMENT_NODE, tag_name, None, false)))
    }

    /// Equivalent to `Document.createTextNode(String data)`.
    pub fn create_text_node(&self, data: &str) -> Node {
        self.create(TEXT_NODE, "#text", Some(data.to_string()), false)
    }
}

/// Whether `name` is an XML 1.0 name (the ASCII-level check of
/// `XMLChar.isValidName` with any non-ASCII character accepted).
fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == ':' || !c.is_ascii() => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.') || !c.is_ascii())
}

/// What a [`NodeList`] lists.
#[derive(Clone, Debug)]
enum NodeListKind {
    Children,
    Descendants(String),
}

/// An ordered list of nodes.
///
/// Equivalent to `org.w3c.dom.NodeList`: live, computed from the tree on
/// each call.
#[derive(Clone, Debug)]
pub struct NodeList {
    root: Node,
    kind: NodeListKind,
}

impl NodeList {
    fn indexes(&self) -> Vec<usize> {
        let nodes = lock(&self.root.doc.nodes);
        match &self.kind {
            NodeListKind::Children => nodes[self.root.index].children.clone(),
            NodeListKind::Descendants(name) => {
                let mut out = Vec::new();
                let mut stack: Vec<usize> = nodes[self.root.index]
                    .children
                    .iter()
                    .rev()
                    .copied()
                    .collect();
                while let Some(i) = stack.pop() {
                    let record = &nodes[i];
                    if record.node_type == ELEMENT_NODE && (name == "*" || record.name == *name) {
                        out.push(i);
                    }
                    stack.extend(record.children.iter().rev().copied());
                }
                out
            }
        }
    }

    /// Equivalent to `NodeList.getLength()`.
    pub fn get_length(&self) -> usize {
        self.indexes().len()
    }

    /// Equivalent to `NodeList.item(int index)`: `None` (Java's `null`) for
    /// an index outside the list.
    pub fn item(&self, index: usize) -> Option<Node> {
        self.indexes().get(index).map(|&i| self.root.node(i))
    }
}

// ----------------------------------------------------------------------
// org.xml.sax
// ----------------------------------------------------------------------

/// A `org.xml.sax.SAXException` or `org.xml.sax.SAXParseException`, carried
/// in [`LuceneErrorKind::Throwable`].
///
/// Equivalent to those classes: the message and, for a parse exception, the
/// public id, system id, line and column of the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaxException {
    message: Option<String>,
    location: Option<Locator>,
}

/// The location of a `SAXParseException`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Locator {
    public_id: Option<String>,
    system_id: Option<String>,
    line_number: i32,
    column_number: i32,
}

impl SaxException {
    /// Create a new SAXException.
    ///
    /// Equivalent to `new SAXException(String message)`.
    pub fn new(message: impl Into<String>) -> Self {
        SaxException {
            message: Some(message.into()),
            location: None,
        }
    }

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }

    /// The [`SaxException`] `error` carries, if it is one.
    pub fn of(error: &LuceneError) -> Option<&SaxException> {
        match error.kind() {
            LuceneErrorKind::Throwable(throwable) => throwable.as_any().downcast_ref(),
            _ => None,
        }
    }

    /// Equivalent to `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Equivalent to `SAXParseException.getLineNumber()`, `-1` for a plain
    /// `SAXException`.
    pub fn get_line_number(&self) -> i32 {
        self.location.as_ref().map_or(-1, |l| l.line_number)
    }

    /// Equivalent to `SAXParseException.getColumnNumber()`, `-1` for a plain
    /// `SAXException`.
    pub fn get_column_number(&self) -> i32 {
        self.location.as_ref().map_or(-1, |l| l.column_number)
    }

    /// Equivalent to `SAXException.toString()` (`Throwable.toString()`, the
    /// class then `": " + message`) and `SAXParseException.toString()` (the
    /// class, then `"; publicId: "`, `"; systemId: "`, `"; lineNumber: "`,
    /// `"; columnNumber: "` for each one known, then `"; " + message`).
    pub fn to_java_string(&self) -> String {
        match &self.location {
            None => match &self.message {
                Some(message) => format!("{SAX_EXCEPTION}: {message}"),
                None => SAX_EXCEPTION.to_string(),
            },
            Some(location) => {
                let mut buf = String::from(SAX_PARSE_EXCEPTION);
                if let Some(public_id) = &location.public_id {
                    buf.push_str("; publicId: ");
                    buf.push_str(public_id);
                }
                if let Some(system_id) = &location.system_id {
                    buf.push_str("; systemId: ");
                    buf.push_str(system_id);
                }
                if location.line_number != -1 {
                    buf.push_str(&format!("; lineNumber: {}", location.line_number));
                }
                if location.column_number != -1 {
                    buf.push_str(&format!("; columnNumber: {}", location.column_number));
                }
                if let Some(message) = &self.message {
                    buf.push_str("; ");
                    buf.push_str(message);
                }
                buf
            }
        }
    }
}

impl JavaThrowable for SaxException {
    fn java_class(&self) -> &'static str {
        if self.location.is_some() {
            SAX_PARSE_EXCEPTION
        } else {
            SAX_EXCEPTION
        }
    }

    fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A single input source for an XML entity.
///
/// Equivalent to `org.xml.sax.InputSource`: a public id, a system id, a
/// character stream (here its text), a byte stream (here its bytes) and an
/// encoding. The parser reads the character stream when there is one,
/// otherwise the byte stream, otherwise it opens the system id.
#[derive(Debug, Clone, Default)]
pub struct InputSource {
    /// `getPublicId()`.
    pub public_id: Option<String>,
    /// `getSystemId()`.
    pub system_id: Option<String>,
    /// `getCharacterStream()`, read to its end.
    pub character_stream: Option<String>,
    /// `getByteStream()`, read to its end.
    pub byte_stream: Option<Vec<u8>>,
    /// `getEncoding()`.
    pub encoding: Option<String>,
}

/// Basic interface for resolving entities.
///
/// Equivalent to `org.xml.sax.EntityResolver`.
pub trait EntityResolver: Send + Sync {
    /// Allow the application to resolve external entities; `None` (Java's
    /// `null`) lets the parser open the system id.
    ///
    /// Equivalent to `resolveEntity(String publicId, String systemId)`.
    ///
    /// # Errors
    ///
    /// A `SAXException` or an `IOException`; it ends the parse unchanged.
    fn resolve_entity(
        &self,
        public_id: Option<&str>,
        system_id: Option<&str>,
    ) -> Result<Option<InputSource>>;
}

/// Basic interface for SAX error handlers.
///
/// Equivalent to `org.xml.sax.ErrorHandler`.
pub trait ErrorHandler: Send + Sync {
    /// Receive notification of a warning.
    ///
    /// Equivalent to `warning(SAXParseException)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse.
    fn warning(&self, exception: &SaxException) -> Result<()>;

    /// Receive notification of a recoverable error.
    ///
    /// Equivalent to `error(SAXParseException)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse.
    fn error(&self, exception: &SaxException) -> Result<()>;

    /// Receive notification of a non-recoverable error; the parse ends with
    /// the exception afterwards.
    ///
    /// Equivalent to `fatalError(SAXParseException)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse instead.
    fn fatal_error(&self, exception: &SaxException) -> Result<()>;
}

// ----------------------------------------------------------------------
// javax.xml.parsers
// ----------------------------------------------------------------------

/// `isValidNameStartChar` of the JDK's scanner for the characters the
/// documents of the query parser use: letters, `_` and `:` (XML 1.0 section
/// 2.3, `NameStartChar`, outside the ranges of ideographs and marks it
/// spells out).
fn is_name_start_char(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

/// `isValidNameChar`: a name start character, a digit, `-`, `.` or U+00B7.
fn is_name_char(c: char) -> bool {
    is_name_start_char(c) || c.is_ascii_digit() || matches!(c, '-' | '.' | '\u{B7}')
}

/// The fatal error the JDK's `XMLDocumentFragmentScannerImpl.scanStartElement`
/// reports for the start tag at byte `lt` of `text` (its `<`), as the byte
/// offset of the character its scanner stopped before and the message, or
/// `None` for a start tag it accepts. It walks the tag as
/// `seekCloseOfStartTag` and `scanAttribute` do (JDK 25 source, read as
/// specification): whitespace is required before each attribute
/// (`ElementUnterminated`), `=` after its name (`EqRequiredInAttribute`), a
/// quote before its value (`OpenQuoteExpected`), no `<` inside the value
/// (`LessthanInAttValue`), `>` after `/`; the end of the input inside the tag
/// is `XML document structures must start and end within the same entity.`.
fn xerces_start_tag_error(text: &str, lt: usize) -> Option<(usize, String)> {
    let eof = || {
        Some((
            text.len(),
            "XML document structures must start and end within the same entity.".to_string(),
        ))
    };
    let chars: Vec<(usize, char)> = text[lt..]
        .char_indices()
        .map(|(i, c)| (lt + i, c))
        .collect();
    let mut i = 1; // past `<`
    let name_from = i;
    while i < chars.len() && is_name_char(chars[i].1) {
        i += 1;
    }
    let element: String = chars[name_from..i].iter().map(|(_, c)| *c).collect();
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    loop {
        // `seekCloseOfStartTag`.
        let space_from = i;
        while i < chars.len() && is_space(chars[i].1) {
            i += 1;
        }
        let saw_space = i > space_from;
        let Some(&(at, c)) = chars.get(i) else {
            return eof();
        };
        if c == '>' {
            return None;
        }
        if c == '/' {
            return match chars.get(i + 1) {
                Some((_, '>')) => None,
                Some(&(at, _)) => Some((
                    at,
                    format!("Element type \"{element}\" must be followed by either attribute specifications, \">\" or \"/>\"."),
                )),
                None => eof(),
            };
        }
        if !is_name_start_char(c) || !saw_space {
            return Some((
                at,
                format!("Element type \"{element}\" must be followed by either attribute specifications, \">\" or \"/>\"."),
            ));
        }
        // `scanAttribute`.
        let attr_from = i;
        while i < chars.len() && is_name_char(chars[i].1) {
            i += 1;
        }
        let attribute: String = chars[attr_from..i].iter().map(|(_, c)| *c).collect();
        while i < chars.len() && is_space(chars[i].1) {
            i += 1;
        }
        match chars.get(i) {
            Some((_, '=')) => i += 1,
            Some(&(at, _)) => {
                return Some((
                    at,
                    format!("Attribute name \"{attribute}\" associated with an element type \"{element}\" must be followed by the ' = ' character."),
                ))
            }
            None => return eof(),
        }
        while i < chars.len() && is_space(chars[i].1) {
            i += 1;
        }
        let quote = match chars.get(i) {
            Some(&(_, q)) if q == '"' || q == '\'' => q,
            Some(&(at, _)) => {
                return Some((
                    at,
                    format!("Open quote is expected for attribute \"{attribute}\" associated with an  element type  \"{element}\"."),
                ))
            }
            None => return eof(),
        };
        i += 1;
        loop {
            match chars.get(i) {
                Some(&(_, c)) if c == quote => {
                    i += 1;
                    break;
                }
                Some(&(at, '<')) => {
                    return Some((
                        at,
                        format!("The value of attribute \"{attribute}\" associated with an element type \"{element}\" must not contain the '<' character."),
                    ))
                }
                Some(_) => i += 1,
                None => return eof(),
            }
        }
    }
}

/// Defines a factory API that enables applications to obtain a parser that
/// produces DOM object trees from XML documents.
///
/// Equivalent to `javax.xml.parsers.DocumentBuilderFactory` as
/// `DocumentBuilderFactory.newInstance()` creates it (the JDK's
/// `DocumentBuilderFactoryImpl`).
#[derive(Debug, Clone, Default)]
pub struct DocumentBuilderFactory {
    validating: bool,
    secure_processing: bool,
    features: Vec<(String, bool)>,
}

impl DocumentBuilderFactory {
    /// Equivalent to `DocumentBuilderFactory.newInstance()`.
    pub fn new_instance() -> Self {
        Self::default()
    }

    /// Equivalent to `setValidating(boolean)`.
    pub fn set_validating(&mut self, validating: bool) {
        self.validating = validating;
    }

    /// Equivalent to `isValidating()`.
    pub fn is_validating(&self) -> bool {
        self.validating
    }

    /// Equivalent to `setFeature(String name, boolean value)`.
    ///
    /// # Errors
    ///
    /// None for `FEATURE_SECURE_PROCESSING`; another feature is recorded and
    /// checked by [`new_document_builder`](Self::new_document_builder).
    pub fn set_feature(&mut self, name: &str, value: bool) -> Result<()> {
        if name == FEATURE_SECURE_PROCESSING {
            self.secure_processing = value;
        } else {
            self.features.push((name.to_string(), value));
        }
        Ok(())
    }

    /// Equivalent to `newDocumentBuilder()`.
    ///
    /// # Errors
    ///
    /// `ParserConfigurationException: Feature '<name>' is not recognized.`
    /// for a feature other than `FEATURE_SECURE_PROCESSING`, which the port
    /// does not emulate.
    pub fn new_document_builder(&self) -> Result<DocumentBuilder> {
        if let Some((name, _)) = self.features.first() {
            return Err(LuceneError::unchecked(
                PARSER_CONFIGURATION_EXCEPTION,
                format!("Feature '{name}' is not recognized."),
            ));
        }
        Ok(DocumentBuilder {
            secure_processing: self.secure_processing,
            entity_resolver: None,
            error_handler: None,
        })
    }
}

/// Defines the API to obtain DOM Document instances from an XML document.
///
/// Equivalent to `javax.xml.parsers.DocumentBuilder` (the JDK's
/// `DocumentBuilderImpl`).
#[derive(Clone)]
pub struct DocumentBuilder {
    secure_processing: bool,
    entity_resolver: Option<Arc<dyn EntityResolver>>,
    error_handler: Option<Arc<dyn ErrorHandler>>,
}

impl fmt::Debug for DocumentBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentBuilder")
            .field("secure_processing", &self.secure_processing)
            .finish()
    }
}

impl DocumentBuilder {
    /// Equivalent to `setEntityResolver(EntityResolver)`.
    pub fn set_entity_resolver(&mut self, resolver: Option<Arc<dyn EntityResolver>>) {
        self.entity_resolver = resolver;
    }

    /// Equivalent to `setErrorHandler(ErrorHandler)`; `None` makes fatal
    /// errors thrown without a report.
    pub fn set_error_handler(&mut self, handler: Option<Arc<dyn ErrorHandler>>) {
        self.error_handler = handler;
    }

    /// Parse the content of the given input stream as an XML document and
    /// return a new DOM Document object.
    ///
    /// Equivalent to `parse(InputStream is)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of reading the stream; what the entity resolver or
    /// the error handler throws, unchanged; the `MalformedURLException` and
    /// the access restriction of opening an external entity without a
    /// resolver; and a [`SaxException`] of class `SAXParseException` for a
    /// fatal error, with the JDK's message for `Content is not allowed in
    /// prolog.`, `Content is not allowed in trailing section.`, `Premature
    /// end of file.`, `XML document structures must start and end within the
    /// same entity.`, `The markup in the document preceding the root element
    /// must be well-formed.`, `The markup in the document following the root
    /// element must be well-formed.`, `The element type "a" must be
    /// terminated by the matching end-tag "</a>".`, `Attribute "x" was
    /// already specified for element "a".`, the open quote, `=` and `<`
    /// attribute errors, the `--` comment error, the reference errors
    /// (undeclared, unterminated, recursive, external in an attribute value),
    /// `Character reference "&#n" is an invalid XML character.`, `XML
    /// version "v" is not supported, only XML 1.0 is supported.`, `Invalid
    /// encoding name "x".` and the limits of secure processing.
    pub fn parse(&self, is: &mut dyn Read) -> Result<Document> {
        let mut bytes = Vec::new();
        is.read_to_end(&mut bytes)?;
        let mut builder = Builder {
            config: self,
            text: String::new(),
            entities: HashMap::new(),
            external_subset: false,
            standalone: false,
            document: Document::new(),
            open: vec![0],
            expansions: 0,
            total_entity_size: 0,
            entity_stack: Vec::new(),
            seen_root: false,
        };
        builder.text = builder.decode(&bytes, None)?;
        let text = builder.text.clone();
        builder.run(&text)?;
        Ok(builder.document)
    }
}

// ----------------------------------------------------------------------
// The builder
// ----------------------------------------------------------------------

/// The predefined general entities of XML 1.0 section 4.6.
fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

/// A general entity: an internal one's replacement text with character
/// references expanded and entity references kept, or an external one's
/// public and system ids.
#[derive(Debug, Clone)]
enum EntityDecl {
    Internal(String),
    External {
        public_id: Option<String>,
        system_id: String,
    },
}

/// The state of one parse.
struct Builder<'b> {
    config: &'b DocumentBuilder,
    /// The decoded document, for error locations.
    text: String,
    entities: HashMap<String, EntityDecl>,
    /// The `DOCTYPE` names an external subset.
    external_subset: bool,
    /// The XML declaration says `standalone="yes"`.
    standalone: bool,
    document: Document,
    /// The indexes of the open nodes, the document first.
    open: Vec<usize>,
    expansions: u32,
    total_entity_size: usize,
    /// The entities being expanded, outermost first.
    entity_stack: Vec<String>,
    seen_root: bool,
}

impl Builder<'_> {
    /// A `SAXParseException` at byte offset `pos` of the decoded document;
    /// the error handler gets `fatalError` first.
    fn fatal(&self, pos: usize, message: impl Into<String>) -> LuceneError {
        let pos = pos.min(self.text.len());
        let before = &self.text[..pos];
        let line = before.matches('\n').count() as i32 + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map(|l| l.chars().count())
            .unwrap_or(0) as i32
            + 1;
        self.fatal_at(line, column, message)
    }

    fn fatal_at(&self, line: i32, column: i32, message: impl Into<String>) -> LuceneError {
        let exception = SaxException {
            message: Some(message.into()),
            location: Some(Locator {
                public_id: None,
                system_id: None,
                line_number: line,
                column_number: column,
            }),
        };
        if let Some(handler) = &self.config.error_handler {
            if let Err(e) = handler.fatal_error(&exception) {
                return e;
            }
        }
        exception.error()
    }

    /// A number of a limit message, formatted for the default locale.
    fn number(n: usize) -> String {
        get_instance(&Locale::get_default())
            .and_then(|f| f.format_long(n as i64))
            .unwrap_or_else(|_| n.to_string())
    }

    /// Decodes the document as the JDK detects its encoding, then applies the
    /// XML line-end normalization (XML 1.0 section 2.11).
    fn decode(&self, bytes: &[u8], encoding: Option<&str>) -> Result<String> {
        let (name, skip): (String, usize) = if let Some(encoding) = encoding {
            (encoding.to_string(), 0)
        } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            ("UTF-8".to_string(), 3)
        } else if bytes.starts_with(&[0xFE, 0xFF]) {
            ("UTF-16BE".to_string(), 2)
        } else if bytes.starts_with(&[0xFF, 0xFE]) {
            ("UTF-16LE".to_string(), 2)
        } else if bytes.starts_with(&[0x00, 0x3C, 0x00, 0x3F]) {
            ("UTF-16BE".to_string(), 0)
        } else if bytes.starts_with(&[0x3C, 0x00, 0x3F, 0x00]) {
            ("UTF-16LE".to_string(), 0)
        } else {
            (
                declared_encoding(bytes).unwrap_or_else(|| "UTF-8".to_string()),
                0,
            )
        };
        let body = &bytes[skip..];
        let upper = name.to_ascii_uppercase();
        let units: Vec<u16> = match upper.as_str() {
            "UTF-8" | "UTF8" => decode_utf8(body).map_err(|m| self.fatal_at(1, 1, m))?,
            "UTF-16" | "UTF-16BE" | "UTF-16LE" => {
                let little =
                    upper == "UTF-16LE" || (upper == "UTF-16" && bytes.starts_with(&[0xFF, 0xFE]));
                body.chunks(2)
                    .map(|c| {
                        let (a, b) = (c[0], *c.get(1).unwrap_or(&0));
                        if little {
                            u16::from_le_bytes([a, b])
                        } else {
                            u16::from_be_bytes([a, b])
                        }
                    })
                    .collect()
            }
            "ISO-8859-1" | "ISO8859-1" | "ISO8859_1" | "LATIN1" | "ISO-LATIN-1" | "L1" => {
                body.iter().map(|&b| u16::from(b)).collect()
            }
            "US-ASCII" | "ASCII" => {
                if let Some(at) = body.iter().position(|&b| b >= 0x80) {
                    let _ = at;
                    return Err(self.fatal_at(1, 1, "Invalid byte 1 of 1-byte UTF-8 sequence."));
                }
                body.iter().map(|&b| u16::from(b)).collect()
            }
            _ => return Err(self.fatal_at(1, 1, format!("Invalid encoding name \"{name}\"."))),
        };
        let text = String::from_utf16(&units)
            .map_err(|_| self.fatal_at(1, 1, "Invalid surrogate pair in the decoded document."))?;
        Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
    }

    /// The index of the node new content goes into.
    fn current(&self) -> usize {
        *self
            .open
            .last()
            .expect("INVARIANT: the document stays open")
    }

    /// Appends a child to the current node.
    fn append(
        &self,
        node_type: u16,
        name: &str,
        value: Option<String>,
        attributes: Vec<(String, String)>,
    ) -> usize {
        let parent = self.current();
        let mut nodes = lock(&self.document.doc.nodes);
        nodes.push(NodeRecord {
            node_type,
            name: name.to_string(),
            value,
            attributes,
            parent: Some(parent),
            children: Vec::new(),
            deferred: true,
        });
        let index = nodes.len() - 1;
        nodes[parent].children.push(index);
        index
    }

    /// Appends text to the current node, merged with a text node there.
    fn append_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let parent = self.current();
        {
            let mut nodes = lock(&self.document.doc.nodes);
            if let Some(&last) = nodes[parent].children.last() {
                if nodes[last].node_type == TEXT_NODE {
                    nodes[last]
                        .value
                        .get_or_insert_with(String::new)
                        .push_str(text);
                    return;
                }
            }
        }
        self.append(TEXT_NODE, "#text", Some(text.to_string()), Vec::new());
    }

    /// Parses the decoded document.
    fn run(&mut self, text: &str) -> Result<()> {
        self.content(text, None, 0)?;
        Ok(())
    }

    /// Parses `text`: the document (`entity` is `None`) or the replacement
    /// text of the entity `entity`, referenced at `ref_pos` of the document.
    fn content(&mut self, text: &str, entity: Option<&str>, ref_pos: usize) -> Result<()> {
        let mut reader = Reader::from_str(text);
        {
            let config = reader.config_mut();
            config.expand_empty_elements = true;
            config.trim_text(false);
            config.check_end_names = true;
            config.check_comments = true;
        }
        let base_depth = self.open.len();
        let in_entity = entity.is_some();
        // A position in the document for errors: the reference inside an
        // entity.
        let loc = |p: u64| if in_entity { ref_pos } else { p as usize };
        loop {
            let pos = reader.buffer_position();
            let event = match reader.read_event() {
                Ok(event) => event,
                Err(e) => {
                    return Err(self.xml_error(
                        &e,
                        loc(reader.error_position()),
                        text,
                        reader.error_position() as usize,
                    ))
                }
            };
            match event {
                Event::Decl(decl) => {
                    if in_entity {
                        continue;
                    }
                    if let Ok(version) = decl.version() {
                        let version: &str = &version;
                        if version != "1.0" && version != "1.1" {
                            return Err(self.fatal(
                                loc(reader.buffer_position()),
                                format!("XML version \"{version}\" is not supported, only XML 1.0 is supported."),
                            ));
                        }
                    }
                    if let Some(Ok(standalone)) = decl.standalone() {
                        self.standalone = &*standalone == "yes";
                    }
                }
                Event::Comment(c) => {
                    let content: &str = &c;
                    self.append(
                        COMMENT_NODE,
                        "#comment",
                        Some(content.to_string()),
                        Vec::new(),
                    );
                }
                Event::PI(pi) => {
                    let target = pi.target().to_string();
                    let data = pi.content().trim_start().to_string();
                    self.append(PROCESSING_INSTRUCTION_NODE, &target, Some(data), Vec::new());
                }
                Event::DocType(doctype) => {
                    if self.seen_root || in_entity {
                        return Err(self.fatal(
                            loc(pos),
                            "The markup in the document following the root element must be well-formed.",
                        ));
                    }
                    let doctype: &str = &doctype;
                    self.doctype(doctype, loc(pos))?;
                }
                Event::Start(start) => {
                    if self.open.len() == 1 && self.seen_root {
                        // `TrailingMiscDriver`: `skipChar('<')` consumed the
                        // `<`, the name start is only peeked
                        // (`MarkupNotRecognizedInMisc`).
                        return Err(self.fatal(
                            loc(pos + 1),
                            "The markup in the document following the root element must be well-formed.",
                        ));
                    }
                    if let Some((at, message)) = xerces_start_tag_error(text, pos as usize) {
                        return Err(self.fatal(loc(at as u64), message));
                    }
                    let name = start.name().as_ref().to_string();
                    if name.chars().count() > MAX_XML_NAME_LIMIT {
                        return Err(self.fatal(
                            loc(reader.buffer_position()),
                            format!(
                                "JAXP00010005: The length of entity \"[xml]\" is \"{}\" that exceeds the \"{}\" limit set by \"jaxp.properties\".",
                                Self::number(name.chars().count()),
                                Self::number(MAX_XML_NAME_LIMIT)
                            ),
                        ));
                    }
                    let mut attributes: Vec<(String, String)> = Vec::new();
                    for attr in start.attributes() {
                        let attr = match attr {
                            Ok(attr) => attr,
                            Err(e) => {
                                return Err(self.attr_error(
                                    &e,
                                    &name,
                                    &start,
                                    loc(reader.buffer_position()),
                                ))
                            }
                        };
                        let qname = attr.key.as_ref().to_string();
                        if attributes.len() == ELEMENT_ATTRIBUTE_LIMIT {
                            return Err(self.fatal(
                                loc(reader.buffer_position()),
                                format!(
                                    "JAXP00010002:  Element \"{name}\" has more than \"{ELEMENT_ATTRIBUTE_LIMIT}\" attributes, \"{ELEMENT_ATTRIBUTE_LIMIT}\" is the limit set by \"jdk.xml.elementAttributeLimit\"."
                                ),
                            ));
                        }
                        let value = self.attribute_value(
                            &attr.value,
                            &qname,
                            &name,
                            loc(reader.buffer_position()),
                        )?;
                        attributes.push((qname, value));
                    }
                    let depth = self.open.len();
                    if depth > MAX_ELEMENT_DEPTH {
                        return Err(self.fatal(
                            loc(reader.buffer_position()),
                            format!(
                                "JAXP00010006: The element \"{name}\" has a depth of \"{depth}\" that exceeds the limit \"{MAX_ELEMENT_DEPTH}\" set by \"jdk.xml.maxElementDepth\"."
                            ),
                        ));
                    }
                    let index = self.append(ELEMENT_NODE, &name, None, attributes);
                    self.open.push(index);
                    self.seen_root = true;
                }
                Event::End(_) => {
                    if self.open.len() <= base_depth {
                        return Err(self.fatal(
                            loc(pos),
                            "XML document structures must start and end within the same entity.",
                        ));
                    }
                    self.open.pop();
                }
                Event::Empty(_) => {
                    unreachable!("INVARIANT: expand_empty_elements reports Start and End")
                }
                Event::Text(t) => {
                    let content: &str = &t;
                    if self.open.len() == 1 && !in_entity {
                        if !content.chars().all(|c| matches!(c, ' ' | '\t' | '\n')) {
                            let message = if self.seen_root {
                                "Content is not allowed in trailing section."
                            } else {
                                "Content is not allowed in prolog."
                            };
                            return Err(self.fatal(loc(pos), message));
                        }
                    } else {
                        self.append_text(content);
                    }
                }
                Event::CData(c) => {
                    if self.open.len() == 1 && !in_entity {
                        let message = if self.seen_root {
                            "The markup in the document following the root element must be well-formed."
                        } else {
                            "The markup in the document preceding the root element must be well-formed."
                        };
                        return Err(self.fatal(loc(pos), message));
                    }
                    let content: &str = &c;
                    self.append(
                        CDATA_SECTION_NODE,
                        "#cdata-section",
                        Some(content.to_string()),
                        Vec::new(),
                    );
                }
                Event::GeneralRef(r) => {
                    let name: &str = &r;
                    if self.open.len() == 1 && !in_entity {
                        let message = if self.seen_root {
                            "Content is not allowed in trailing section."
                        } else {
                            "Content is not allowed in prolog."
                        };
                        return Err(self.fatal(loc(pos), message));
                    }
                    self.reference(name, loc(pos))?;
                }
                Event::Eof => {
                    if in_entity {
                        if self.open.len() != base_depth {
                            return Err(self.fatal(
                                ref_pos,
                                "XML document structures must start and end within the same entity.",
                            ));
                        }
                        return Ok(());
                    }
                    if self.open.len() > 1 {
                        return Err(self.fatal(
                            text.len(),
                            "XML document structures must start and end within the same entity.",
                        ));
                    }
                    if !self.seen_root {
                        return Err(self.fatal(text.len(), "Premature end of file."));
                    }
                    return Ok(());
                }
            }
        }
    }

    /// A reference `&name;` in content.
    fn reference(&mut self, name: &str, pos: usize) -> Result<()> {
        if name.starts_with('#') {
            let c = self.char_ref(name, pos)?;
            self.append_text(&c.to_string());
            return Ok(());
        }
        if let Some(c) = predefined_entity(name) {
            self.append_text(&c.to_string());
            return Ok(());
        }
        let decl = match self.entities.get(name).cloned() {
            Some(decl) => decl,
            // With an external subset the declaration may be there (a
            // validity constraint, not a well-formedness one): the JDK skips
            // the reference without an error.
            None if self.external_subset && !self.standalone => return Ok(()),
            None => {
                return Err(self.fatal(
                    pos,
                    format!("The entity \"{name}\" was referenced, but not declared."),
                ))
            }
        };
        if self.entity_stack.iter().any(|e| e == name) {
            let mut path = self.entity_stack.clone();
            let start = path.iter().position(|e| e == name).unwrap_or(0);
            path.drain(..start);
            path.push(name.to_string());
            return Err(self.fatal(
                pos,
                format!(
                    "Recursive entity reference \"{name}\". (Reference path: {}),",
                    path.join(" -> ")
                ),
            ));
        }
        self.expansions += 1;
        if self.expansions > ENTITY_EXPANSION_LIMIT {
            return Err(self.fatal_at(
                1,
                1,
                format!(
                    "JAXP00010001: The parser has encountered more than \"{ENTITY_EXPANSION_LIMIT}\" entity expansions in this document; this is the limit imposed by \"jdk.xml.entityExpansionLimit\"."
                ),
            ));
        }
        let replacement = match decl {
            EntityDecl::Internal(text) => text,
            EntityDecl::External {
                public_id,
                system_id,
            } => {
                let expanded = expand_system_id(&system_id, None);
                self.open_external(
                    public_id.as_deref(),
                    &system_id,
                    &expanded,
                    "External Entity: Failed to read external document",
                    pos,
                )?
            }
        };
        let size = replacement.chars().count();
        if size > MAX_GENERAL_ENTITY_SIZE_LIMIT {
            return Err(self.fatal(
                pos,
                format!(
                    "JAXP00010003: The length of entity \"{name}\" is \"{}\" that exceeds the \"{}\" limit set by \"jdk.xml.maxGeneralEntitySizeLimit\".",
                    Self::number(size),
                    Self::number(MAX_GENERAL_ENTITY_SIZE_LIMIT)
                ),
            ));
        }
        self.total_entity_size += size;
        if self.total_entity_size > TOTAL_ENTITY_SIZE_LIMIT {
            return Err(self.fatal(
                pos,
                format!(
                    "JAXP00010004: The accumulated size of entities is \"{}\" that exceeded the \"{}\" limit set by \"jdk.xml.totalEntitySizeLimit\".",
                    Self::number(self.total_entity_size),
                    Self::number(TOTAL_ENTITY_SIZE_LIMIT)
                ),
            ));
        }
        self.entity_stack.push(name.to_string());
        let result = self.content(&replacement, Some(name), pos);
        self.entity_stack.pop();
        result
    }

    /// Reads an external entity through the resolver, or opens its system
    /// id as the JDK does without one: the text of the entity.
    fn open_external(
        &self,
        public_id: Option<&str>,
        literal: &str,
        expanded: &str,
        failure: &str,
        pos: usize,
    ) -> Result<String> {
        let source = match &self.config.entity_resolver {
            Some(resolver) => resolver.resolve_entity(public_id, Some(expanded))?,
            None => None,
        };
        if let Some(source) = source {
            if let Some(text) = source.character_stream {
                return Ok(text.replace("\r\n", "\n").replace('\r', "\n"));
            }
            if let Some(bytes) = source.byte_stream {
                return self.decode(&bytes, source.encoding.as_deref());
            }
            if let Some(id) = source.system_id {
                return self.open_url(&id, literal, failure, pos);
            }
        }
        self.open_url(expanded, literal, failure, pos)
    }

    /// Opens a URL as the JDK's `XMLEntityManager.setupCurrentEntity` does
    /// under secure processing.
    fn open_url(&self, url: &str, literal: &str, failure: &str, pos: usize) -> Result<String> {
        let protocol = url.split_once(':').map(|(p, _)| p.to_ascii_lowercase());
        match protocol.as_deref() {
            Some("file") | Some("http") | Some("https") | Some("jar") | Some("ftp")
            | Some("jrt") => {
                if self.config.secure_processing {
                    return Err(self.fatal(
                        pos,
                        format!(
                            "{failure} '{literal}', because '{}' access is not allowed due to restriction set by the accessExternalDTD property.",
                            protocol.unwrap_or_default()
                        ),
                    ));
                }
                let path = url
                    .strip_prefix("file://")
                    .or_else(|| url.strip_prefix("file:"));
                match path {
                    Some(path) => {
                        let bytes = std::fs::read(path)
                            .map_err(|e| LuceneError::from(LuceneErrorKind::Io(e)))?;
                        self.decode(&bytes, None)
                    }
                    None => Err(io_exception(
                        "java.io.FileNotFoundException",
                        url.to_string(),
                    )),
                }
            }
            Some(p) => Err(io_exception(
                "java.net.MalformedURLException",
                format!("unknown protocol: {p}"),
            )),
            None => Err(io_exception(
                "java.net.MalformedURLException",
                format!("no protocol: {url}"),
            )),
        }
    }

    /// A `quick-xml` error as the JDK's fatal error.
    fn xml_error(&self, error: &XmlError, pos: usize, text: &str, local: usize) -> LuceneError {
        match error {
            XmlError::IllFormed(IllFormedError::MismatchedEndTag { expected, .. }) => self.fatal(
                pos,
                format!("The element type \"{expected}\" must be terminated by the matching end-tag \"</{expected}>\"."),
            ),
            XmlError::IllFormed(IllFormedError::MissingEndTag(_)) => self.fatal(
                self.text.len(),
                "XML document structures must start and end within the same entity.",
            ),
            XmlError::IllFormed(IllFormedError::UnmatchedEndTag(_)) => {
                if self.open.len() == 1 {
                    self.fatal(
                        pos,
                        "The markup in the document following the root element must be well-formed.",
                    )
                } else {
                    self.fatal(
                        pos,
                        "XML document structures must start and end within the same entity.",
                    )
                }
            }
            XmlError::IllFormed(IllFormedError::DoubleHyphenInComment) => {
                self.fatal(pos, "The string \"--\" is not permitted within comments.")
            }
            XmlError::IllFormed(IllFormedError::UnclosedReference) => {
                let after = text.get(local..).unwrap_or("");
                let after = after.strip_prefix('&').unwrap_or(after);
                let name: String = after
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != ';' && *c != '<' && *c != '&')
                    .collect();
                if name.is_empty() {
                    self.fatal(
                        pos,
                        "The entity name must immediately follow the '&' in the entity reference.",
                    )
                } else {
                    self.fatal(
                        pos,
                        format!("The reference to entity \"{name}\" must end with the ';' delimiter."),
                    )
                }
            }
            XmlError::IllFormed(IllFormedError::UnknownVersion) => {
                self.fatal(pos, "XML version \"\" is not supported, only XML 1.0 is supported.")
            }
            XmlError::Syntax(SyntaxError::UnclosedTag)
                if text[local.min(text.len())..].starts_with('<')
                    && text[local.min(text.len())..]
                        .chars()
                        .nth(1)
                        .is_some_and(is_name_start_char) =>
            {
                // An element's start tag that the input ends inside:
                // `scanStartElement` reports what it meets first.
                let (at, message) = xerces_start_tag_error(text, local).unwrap_or((
                    text.len(),
                    "XML document structures must start and end within the same entity."
                        .to_string(),
                ));
                self.fatal(pos + at - local.min(at), message)
            }
            XmlError::Syntax(SyntaxError::UnclosedTag) | XmlError::Syntax(SyntaxError::InvalidBangMarkup) if !self.seen_root => self.fatal(
                pos,
                "The markup in the document preceding the root element must be well-formed.",
            ),
            other => self.fatal(pos, other.to_string()),
        }
    }

    /// An attribute error of `quick-xml` as the JDK's fatal error.
    fn attr_error(
        &self,
        error: &AttrError,
        element: &str,
        start: &quick_xml::events::BytesStart<'_>,
        pos: usize,
    ) -> LuceneError {
        let raw = start.attributes_raw();
        let name_at = |offset: usize| -> String {
            let tag: &str = start;
            let rest = tag.get(offset..).unwrap_or("");
            let _ = raw;
            rest.chars()
                .take_while(|c| !c.is_whitespace() && *c != '=')
                .collect()
        };
        match error {
            AttrError::Duplicated(at, _) => self.fatal(
                pos,
                format!(
                    "Attribute \"{}\" was already specified for element \"{element}\".",
                    name_at(*at)
                ),
            ),
            AttrError::UnquotedValue(at) | AttrError::ExpectedValue(at) => {
                let tag: &str = start;
                let before = tag.get(..*at).unwrap_or("");
                let name: String = before
                    .trim_end()
                    .trim_end_matches('=')
                    .trim_end()
                    .rsplit(|c: char| c.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .to_string();
                self.fatal(
                    pos,
                    format!(
                        "Open quote is expected for attribute \"{name}\" associated with an  element type  \"{element}\"."
                    ),
                )
            }
            AttrError::ExpectedEq(at) => {
                let tag: &str = start;
                let before = tag.get(..*at).unwrap_or("");
                let name: String = before
                    .trim_end()
                    .rsplit(|c: char| c.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .to_string();
                self.fatal(
                    pos,
                    format!(
                        "Attribute name \"{name}\" associated with an element type \"{element}\" must be followed by the ' = ' character."
                    ),
                )
            }
            other => self.fatal(pos, other.to_string()),
        }
    }

    /// The value of a `<!DOCTYPE ...>`: the internal subset's general
    /// entities, then the external subset through `resolveEntity`.
    fn doctype(&mut self, doctype: &str, pos: usize) -> Result<()> {
        let mut rest = doctype.trim_start();
        // The root element name.
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(rest.len());
        let root_name = rest[..name_end].to_string();
        rest = rest[name_end..].trim_start();
        let mut public_id = None;
        let mut system_id = None;
        if let Some(after) = rest.strip_prefix("SYSTEM") {
            let (literal, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal(
                    pos,
                    "The system identifier must begin with either a single or double quote character.",
                )
            })?;
            system_id = Some(literal.to_string());
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix("PUBLIC") {
            let (public, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal(
                    pos,
                    "The public identifier must begin with either a single or double quote character.",
                )
            })?;
            let (literal, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal(
                    pos,
                    "White spaces are required between publicId and systemId.",
                )
            })?;
            public_id = Some(public.to_string());
            system_id = Some(literal.to_string());
            rest = after.trim_start();
        }
        self.append(DOCUMENT_TYPE_NODE, &root_name, None, Vec::new());
        if let Some(subset) = rest.strip_prefix('[') {
            let end = subset.rfind(']').unwrap_or(subset.len());
            self.declarations(&subset[..end], pos)?;
        }
        if let Some(system_id) = system_id {
            self.external_subset = true;
            let expanded = expand_system_id(&system_id, None);
            let dtd = self.open_external(
                public_id.as_deref(),
                &system_id,
                &expanded,
                "External DTD: Failed to read external DTD",
                pos,
            )?;
            self.declarations(&dtd, pos)?;
        }
        Ok(())
    }

    /// Records the `<!ENTITY name ...>` declarations of a DTD subset; the
    /// first declaration of a name wins (XML 1.0 section 4.2).
    fn declarations(&mut self, subset: &str, pos: usize) -> Result<()> {
        let mut rest = subset;
        while let Some(start) = rest.find("<!ENTITY") {
            rest = rest[start + "<!ENTITY".len()..].trim_start();
            if rest.starts_with('%') {
                // A parameter entity: not expanded by this builder.
                continue;
            }
            let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let name = rest[..name_end].to_string();
            rest = rest[name_end..].trim_start();
            let decl = if let Some(after) = rest.strip_prefix("SYSTEM") {
                let (literal, _) = quoted(after.trim_start()).ok_or_else(|| {
                    self.fatal(
                        pos,
                        "The system identifier must begin with either a single or double quote character.",
                    )
                })?;
                EntityDecl::External {
                    public_id: None,
                    system_id: literal.to_string(),
                }
            } else if let Some(after) = rest.strip_prefix("PUBLIC") {
                let (public, after) = quoted(after.trim_start()).ok_or_else(|| {
                    self.fatal(
                        pos,
                        "The public identifier must begin with either a single or double quote character.",
                    )
                })?;
                let (literal, _) = quoted(after.trim_start()).ok_or_else(|| {
                    self.fatal(
                        pos,
                        "White spaces are required between publicId and systemId.",
                    )
                })?;
                EntityDecl::External {
                    public_id: Some(public.to_string()),
                    system_id: literal.to_string(),
                }
            } else {
                let (literal, _) = quoted(rest).ok_or_else(|| {
                    self.fatal(
                        pos,
                        format!("The entity \"{name}\" must have a quoted value."),
                    )
                })?;
                EntityDecl::Internal(self.expand_char_refs(literal, pos)?)
            };
            self.entities.entry(name).or_insert(decl);
        }
        Ok(())
    }

    /// Expands the character references of an entity value literal.
    fn expand_char_refs(&self, literal: &str, pos: usize) -> Result<String> {
        let mut out = String::with_capacity(literal.len());
        let mut rest = literal;
        while let Some(amp) = rest.find("&#") {
            out.push_str(&rest[..amp]);
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal(
                    pos,
                    "The character reference must end with the ';' delimiter.",
                )
            })?;
            out.push(self.char_ref(&after[..semi], pos)?);
            rest = &after[semi + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The character of a reference body `#n` or `#xh`.
    fn char_ref(&self, body: &str, pos: usize) -> Result<char> {
        let digits = body.strip_prefix('#').unwrap_or(body);
        let value = match digits.strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse::<u32>().ok(),
        };
        value
            .filter(|&v| {
                v == 0x9
                    || v == 0xA
                    || v == 0xD
                    || (0x20..=0xD7FF).contains(&v)
                    || (0xE000..=0xFFFD).contains(&v)
                    || (0x10000..=0x10FFFF).contains(&v)
            })
            .and_then(char::from_u32)
            .ok_or_else(|| {
                self.fatal(
                    pos,
                    format!("Character reference \"&{body}\" is an invalid XML character."),
                )
            })
    }

    /// The text a reference `&name;` stands for in an attribute value.
    fn attribute_reference(
        &self,
        name: &str,
        attribute: &str,
        element: &str,
        pos: usize,
        level: usize,
    ) -> Result<String> {
        if name.starts_with('#') {
            return Ok(self.char_ref(name, pos)?.to_string());
        }
        if let Some(c) = predefined_entity(name) {
            return Ok(c.to_string());
        }
        match self.entities.get(name) {
            None if self.external_subset && !self.standalone => Ok(String::new()),
            None => Err(self.fatal(
                pos,
                format!("The entity \"{name}\" was referenced, but not declared."),
            )),
            Some(EntityDecl::External { .. }) => Err(self.fatal(
                pos,
                format!("The external entity reference \"&{name};\" is not permitted in an attribute value."),
            )),
            Some(EntityDecl::Internal(text)) => {
                if level > 64 {
                    return Err(self.fatal(
                        pos,
                        format!("Recursive entity reference \"{name}\". (Reference path: {name} -> {name}),"),
                    ));
                }
                if text.contains('<') {
                    return Err(self.fatal(
                        pos,
                        format!("The value of attribute \"{attribute}\" associated with an element type \"{element}\" must not contain the '<' character."),
                    ));
                }
                self.normalize_attribute(text, attribute, element, pos, level + 1)
            }
        }
    }

    /// A normalized attribute value (XML 1.0 section 3.3.3).
    fn attribute_value(
        &self,
        raw: &str,
        attribute: &str,
        element: &str,
        pos: usize,
    ) -> Result<String> {
        if raw.contains('<') {
            return Err(self.fatal(
                pos,
                format!("The value of attribute \"{attribute}\" associated with an element type \"{element}\" must not contain the '<' character."),
            ));
        }
        self.normalize_attribute(raw, attribute, element, pos, 0)
    }

    fn normalize_attribute(
        &self,
        raw: &str,
        attribute: &str,
        element: &str,
        pos: usize,
        level: usize,
    ) -> Result<String> {
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(amp) = rest.find('&') {
            out.extend(
                rest[..amp]
                    .chars()
                    .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
            );
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal(
                    pos,
                    "The reference to entity must end with the ';' delimiter.",
                )
            })?;
            let name = &after[..semi];
            if name.starts_with('#') {
                // A character reference is kept as is, a tab or line end too.
                out.push(self.char_ref(name, pos)?);
            } else {
                out.push_str(&self.attribute_reference(name, attribute, element, pos, level)?);
            }
            rest = &after[semi + 1..];
        }
        out.extend(
            rest.chars()
                .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
        );
        Ok(out)
    }
}

/// Expands a system id as the JDK's `XMLEntityManager.expandSystemId` does
/// for a document read from a stream: an absolute URI (`scheme:`) as is, a
/// relative one resolved against `base`, or against the working directory
/// (`file:///<dir>/`) without one.
fn expand_system_id(system_id: &str, base: Option<&str>) -> String {
    let has_scheme = system_id.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if has_scheme {
        return system_id.to_string();
    }
    if system_id.starts_with('/') {
        return format!("file://{system_id}");
    }
    let base = match base {
        Some(base) => base.to_string(),
        None => {
            let dir = std::env::current_dir()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("file://{}/", dir.trim_end_matches('/'))
        }
    };
    match base.rfind('/') {
        Some(slash) => format!("{}{system_id}", &base[..=slash]),
        None => system_id.to_string(),
    }
}

/// A quoted literal at the start of `s` and the text after it.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let quote = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let body = &s[1..];
    let end = body.find(quote)?;
    Some((&body[..end], &body[end + 1..]))
}

/// The `encoding` pseudo-attribute of an XML declaration at the start of an
/// ASCII-compatible document.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }
    let end = bytes.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(&bytes[..end]).ok()?;
    let at = decl.find("encoding")?;
    let after = decl[at + "encoding".len()..]
        .trim_start()
        .strip_prefix('=')?;
    quoted(after.trim_start()).map(|(name, _)| name.to_string())
}

/// Decodes UTF-8 as the JDK's `UTF8Reader` (Xerces) does, failing with its
/// message on the first malformed sequence.
fn decode_utf8(bytes: &[u8]) -> std::result::Result<Vec<u16>, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.encode_utf16().collect()),
        Err(e) => {
            let at = e.valid_up_to();
            let first = bytes[at];
            let expected = if first & 0xE0 == 0xC0 {
                2
            } else if first & 0xF0 == 0xE0 {
                3
            } else if first & 0xF8 == 0xF0 {
                4
            } else {
                1
            };
            let index = match e.error_len() {
                Some(n) if expected > 1 => n + 1,
                _ => 1,
            };
            Err(format!(
                "Invalid byte {index} of {expected}-byte UTF-8 sequence."
            ))
        }
    }
}
