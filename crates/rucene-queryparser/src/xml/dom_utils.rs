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

//! Port of `DOMUtils.java`, of the Java package
//! `org.apache.lucene.queryparser.xml`.

use rucene::error::{LuceneError, Result};

use super::parser_exception::ParserException;
use crate::jdk::dom::{Element, Node, ELEMENT_NODE, ENTITY_REFERENCE_NODE, TEXT_NODE};
use crate::jdk::string::{equals_ignore_case, trim_str};

/// Helper methods for parsing XML
///
/// Equivalent to `org.apache.lucene.queryparser.xml.DOMUtils`; Java's
/// overloads of `getAttribute` are [`get_attribute_string`](Self::get_attribute_string),
/// [`get_attribute_float`](Self::get_attribute_float),
/// [`get_attribute_int`](Self::get_attribute_int) and
/// [`get_attribute_boolean`](Self::get_attribute_boolean).
#[derive(Debug)]
pub struct DOMUtils {
    _no_instances: (),
}

impl DOMUtils {
    /// Equivalent to `DOMUtils.getChildByTagOrFail(Element e, String name)`.
    ///
    /// # Errors
    ///
    /// `ParserException(tag + " missing \"" + name + "\" child element")`.
    pub fn get_child_by_tag_or_fail(e: &Element, name: &str) -> Result<Element> {
        Self::get_child_by_tag_name(e, name).ok_or_else(|| {
            ParserException::with_message(format!(
                "{} missing \"{name}\" child element",
                e.get_tag_name()
            ))
            .error()
        })
    }

    /// Equivalent to `DOMUtils.getFirstChildOrFail(Element e)`.
    ///
    /// # Errors
    ///
    /// `ParserException(tag + " does not contain a child element")`.
    pub fn get_first_child_or_fail(e: &Element) -> Result<Element> {
        Self::get_first_child_element(e).ok_or_else(|| {
            ParserException::with_message(format!(
                "{} does not contain a child element",
                e.get_tag_name()
            ))
            .error()
        })
    }

    /// Equivalent to `DOMUtils.getAttributeOrFail(Element e, String name)`:
    /// `getAttribute` gives `""` for an absent attribute, never `null`, so
    /// it never fails.
    ///
    /// # Errors
    ///
    /// None in practice (the `null` check of Java).
    pub fn get_attribute_or_fail(e: &Element, name: &str) -> Result<String> {
        Ok(e.get_attribute(name))
    }

    /// Equivalent to `DOMUtils.getAttributeWithInheritanceOrFail(Element e,
    /// String name)`.
    ///
    /// # Errors
    ///
    /// `ParserException(tag + " missing \"" + name + "\" attribute")`.
    pub fn get_attribute_with_inheritance_or_fail(e: &Element, name: &str) -> Result<String> {
        Self::get_attribute_with_inheritance(e, name).ok_or_else(|| {
            ParserException::with_message(format!(
                "{} missing \"{name}\" attribute",
                e.get_tag_name()
            ))
            .error()
        })
    }

    /// Equivalent to `DOMUtils.getNonBlankTextOrFail(Element e)`: the text,
    /// trimmed.
    ///
    /// # Errors
    ///
    /// `ParserException(tag + " has no text")` for blank text.
    pub fn get_non_blank_text_or_fail(e: &Element) -> Result<String> {
        let v = trim_str(&Self::get_text(e)).to_string();
        if v.is_empty() {
            return Err(
                ParserException::with_message(format!("{} has no text", e.get_tag_name())).error(),
            );
        }
        Ok(v)
    }

    /// Convenience method where there is only one child Element of a given
    /// name
    ///
    /// Equivalent to `DOMUtils.getChildByTagName(Element e, String name)`.
    pub fn get_child_by_tag_name(e: &Element, name: &str) -> Option<Element> {
        let mut kid = e.get_first_child();
        while let Some(k) = kid {
            if k.get_node_type() == ELEMENT_NODE && name == k.get_node_name() {
                return k.as_element();
            }
            kid = k.get_next_sibling();
        }
        None
    }

    /// Equivalent to `DOMUtils.getAttributeWithInheritance(Element element,
    /// String attributeName)`: the attribute, or that of the nearest
    /// ancestor element that has it; `None` (Java's `null`) at the top.
    pub fn get_attribute_with_inheritance(
        element: &Element,
        attribute_name: &str,
    ) -> Option<String> {
        let result = element.get_attribute(attribute_name);
        if result.is_empty() {
            let n = element.get_parent_node()?;
            if n == **element {
                return None;
            }
            if let Some(parent) = n.as_element() {
                return Self::get_attribute_with_inheritance(&parent, attribute_name);
            }
            return None; // we reached the top level of the document without finding attribute
        }
        Some(result)
    }

    /// Convenience method where there is only one child Element of a given
    /// name
    ///
    /// Equivalent to `DOMUtils.getChildTextByTagName(Element e, String
    /// tagName)`.
    pub fn get_child_text_by_tag_name(e: &Element, tag_name: &str) -> Option<String> {
        Self::get_child_by_tag_name(e, tag_name).map(|child| Self::get_text(&child))
    }

    /// Convenience method to append a new child with text
    ///
    /// Equivalent to `DOMUtils.insertChild(Element parent, String tagName,
    /// String text)`.
    ///
    /// # Errors
    ///
    /// The `DOMException`s of creating and appending the nodes.
    pub fn insert_child(parent: &Element, tag_name: &str, text: Option<&str>) -> Result<Element> {
        let document = parent
            .get_owner_document()
            .ok_or_else(LuceneError::null_pointer)?;
        let child = document.create_element(tag_name)?;
        parent.append_child(&child)?;
        if let Some(text) = text {
            let document = child
                .get_owner_document()
                .ok_or_else(LuceneError::null_pointer)?;
            child.append_child(&document.create_text_node(text))?;
        }
        Ok(child)
    }

    /// Equivalent to `DOMUtils.getAttribute(Element element, String
    /// attributeName, String deflt)`: the default for an empty or absent
    /// attribute.
    pub fn get_attribute_string(
        element: &Element,
        attribute_name: &str,
        deflt: Option<&str>,
    ) -> Option<String> {
        let result = element.get_attribute(attribute_name);
        if result.is_empty() {
            deflt.map(str::to_string)
        } else {
            Some(result)
        }
    }

    /// Equivalent to `DOMUtils.getAttribute(Element element, String
    /// attributeName, float deflt)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `Float.parseFloat`.
    pub fn get_attribute_float(element: &Element, attribute_name: &str, deflt: f32) -> Result<f32> {
        let result = element.get_attribute(attribute_name);
        if result.is_empty() {
            Ok(deflt)
        } else {
            crate::javacc::parse_float(&result)
        }
    }

    /// Equivalent to `DOMUtils.getAttribute(Element element, String
    /// attributeName, int deflt)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `Integer.parseInt`.
    pub fn get_attribute_int(element: &Element, attribute_name: &str, deflt: i32) -> Result<i32> {
        let result = element.get_attribute(attribute_name);
        if result.is_empty() {
            Ok(deflt)
        } else {
            crate::javacc::parse_int(&result)
        }
    }

    /// Equivalent to `DOMUtils.getAttribute(Element element, String
    /// attributeName, boolean deflt)`: `Boolean.valueOf`, true for `true` in
    /// any case.
    pub fn get_attribute_boolean(element: &Element, attribute_name: &str, deflt: bool) -> bool {
        let result = element.get_attribute(attribute_name);
        if result.is_empty() {
            deflt
        } else {
            equals_ignore_case(&result, "true")
        }
    }

    /// Returns text of node and all child nodes - without markup
    ///
    /// Equivalent to `DOMUtils.getText(Node e)`.
    pub fn get_text(e: &Node) -> String {
        let mut sb = String::new();
        Self::get_text_buffer(e, &mut sb);
        sb
    }

    /// Equivalent to `DOMUtils.getFirstChildElement(Element element)`.
    pub fn get_first_child_element(element: &Element) -> Option<Element> {
        let mut kid = element.get_first_child();
        while let Some(k) = kid {
            if k.get_node_type() == ELEMENT_NODE {
                return k.as_element();
            }
            kid = k.get_next_sibling();
        }
        None
    }

    /// Equivalent to the private `DOMUtils.getTextBuffer(Node e,
    /// StringBuilder sb)`: the text nodes, recursively through elements and
    /// entity references (CDATA sections are not text nodes).
    fn get_text_buffer(e: &Node, sb: &mut String) {
        let mut kid = e.get_first_child();
        while let Some(k) = kid {
            match k.get_node_type() {
                TEXT_NODE => sb.push_str(k.get_node_value().as_deref().unwrap_or("null")),
                ELEMENT_NODE | ENTITY_REFERENCE_NODE => Self::get_text_buffer(&k, sb),
                _ => {}
            }
            kid = k.get_next_sibling();
        }
    }
}
