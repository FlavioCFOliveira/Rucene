/*
 * Copyright Marc Guillemot
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

//! Port of `LostText.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use crate::java::{trim, JResult};
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_string::XMLString;
use crate::org::cyberneko::html::html_augmentations::HTMLAugmentations;

/// Container for text that should be hold and re-feed later like text
/// before &lt;html&gt; that will be re-feed in &lt;body&gt;.
///
/// Equivalent to the package-private `org.cyberneko.html.LostText`.
#[derive(Debug, Default)]
pub(crate) struct LostText {
    entries: Vec<Entry>,
}

/// Pair of (text, augmentation).
///
/// Equivalent to the nested class `LostText.Entry`.
#[derive(Debug)]
struct Entry {
    text: XMLString,
    augs: Option<HTMLAugmentations>,
}

impl Entry {
    fn new(text: &XMLString, augs: Option<&dyn Augmentations>) -> Self {
        let chars = text.chars().to_vec();
        let len = chars.len() as i32;
        Self {
            text: XMLString::with_values(&chars, 0, len),
            augs: augs.map(HTMLAugmentations::copy_of),
        }
    }
}

impl LostText {
    /// Adds some text that need to be re-feed later. The information gets
    /// copied.
    ///
    /// Equivalent to `add(XMLString, Augmentations)`.
    pub(crate) fn add(&mut self, text: &XMLString, augs: Option<&dyn Augmentations>) {
        if !self.entries.is_empty() || !trim(&text.to_java_string()).is_empty() {
            self.entries.push(Entry::new(text, augs));
        }
    }

    /// Pushes the characters into the tag balancer.
    ///
    /// Equivalent to `refeed(XMLDocumentHandler)`.
    pub(crate) fn refeed(&mut self, tag_balancer: &dyn XMLDocumentHandler) -> JResult<()> {
        let entries = std::mem::take(&mut self.entries);
        for entry in &entries {
            tag_balancer.characters(
                &entry.text,
                entry.augs.as_ref().map(|a| a as &dyn Augmentations),
            )?;
        }
        // not needed anymore once it has been used -> clear to free memory
        Ok(())
    }

    /// Indicates if this container contains something.
    ///
    /// Equivalent to `isEmpty()`.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
